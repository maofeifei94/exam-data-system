//! PDF 生成：把「可编辑模板 + 数据行」渲染成 PDF。
//!
//! 模板元素坐标系：以内容区（页面去掉页边距后的区域）左上角为原点，单位毫米。
//! - single 模式：一行数据一页，元素相对页面内容区。
//! - matrix 模式：网格铺满页面，元素相对单元格左上角；
//!   `page_level = true` 的元素相对页面内容区，每页只画一次（用于标题）。
//!
//! 占位符：`{字段名}` 取该行字段值，`{{index}}` 本页序号，`{{no}}` 全局行号，
//! `{{page}}` / `{{pages}}` / `{{count}}`。

use crate::error::{AppError, AppResult};
use crate::models::{
    DatasetColumn, Margins, MatrixSetup, PageSetup, TableSetup, TemplateBody, TemplateElement,
};
use crate::util;
use printpdf::path::PaintMode;
use printpdf::*;
use std::collections::{HashMap, HashSet};

const FONT_BYTES: &[u8] = include_bytes!("../../assets/NotoSansSC.ttf");
const MM_PER_PT: f32 = 0.352778;
const PT_PER_MM: f32 = 2.834646;
const IMAGE_DPI: f32 = 72.0;

#[derive(Debug, Default, Clone)]
pub struct RenderOpts {
    pub photo_dir: Option<String>,
    pub only_page: Option<i64>,
}

struct Ctx<'a> {
    row: &'a [String],
    index: &'a HashMap<String, usize>,
    /// 全部数据行，用于 `{{sum:字段}}` 等统计占位符
    all_rows: &'a [Vec<String>],
    index_no: i64,
    no: i64,
    page: i64,
    pages: i64,
    count: i64,
}

impl<'a> Ctx<'a> {
    fn value(&self, field: &str) -> &str {
        let f = field.trim();
        if f.is_empty() {
            return "";
        }
        self.index
            .get(f)
            .and_then(|i| self.row.get(*i))
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    fn lookup(&self, key: &str) -> String {
        let key = key.trim();
        // `{字段|默认值}`：字段为空时输出默认值，避免模板出现空白。
        if let Some((name, fallback)) = key.split_once('|') {
            let value = self.lookup(name.trim());
            return if value.trim().is_empty() {
                fallback.to_string()
            } else {
                value
            };
        }
        match key {
            "index" => self.index_no.to_string(),
            "no" => self.no.to_string(),
            "page" => self.page.to_string(),
            "pages" => self.pages.to_string(),
            "count" => self.count.to_string(),
            "total" => self.all_rows.len().to_string(),
            "date" => chrono::Local::now().format("%Y-%m-%d").to_string(),
            "time" => chrono::Local::now().format("%H:%M").to_string(),
            other => self
                .stat(other)
                .unwrap_or_else(|| self.value(other).to_string()),
        }
    }

    /// 统计占位符：`sum/avg/min/max/std/count/distinct:字段`、`rate:字段:阈值`。
    fn stat(&self, key: &str) -> Option<String> {
        let mut parts = key.splitn(3, ':');
        let func = parts.next()?.trim();
        let field = parts.next()?.trim();
        let idx = *self.index.get(field)?;
        let values: Vec<&str> = self
            .all_rows
            .iter()
            .filter_map(|r| r.get(idx))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        match func {
            "count" => Some(values.len().to_string()),
            "distinct" => {
                let set: HashSet<&str> = values.into_iter().collect();
                Some(set.len().to_string())
            }
            "sum" | "avg" | "min" | "max" | "std" => {
                let nums: Vec<f64> = values
                    .iter()
                    .filter_map(|s| s.parse::<f64>().ok())
                    .collect();
                if nums.is_empty() {
                    return Some(String::new());
                }
                let value = match func {
                    "sum" => nums.iter().sum(),
                    "avg" => nums.iter().sum::<f64>() / nums.len() as f64,
                    "min" => nums.iter().copied().fold(f64::INFINITY, f64::min),
                    "max" => nums.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    _ => {
                        let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                        let var = nums.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                            / nums.len() as f64;
                        var.sqrt()
                    }
                };
                Some(fmt_num(value))
            }
            "rate" => {
                let threshold: f64 = parts.next()?.trim().parse().ok()?;
                let nums: Vec<f64> = values
                    .iter()
                    .filter_map(|s| s.parse::<f64>().ok())
                    .collect();
                if nums.is_empty() {
                    return Some(String::new());
                }
                let hit = nums.iter().filter(|v| **v >= threshold).count();
                Some(fmt_num(hit as f64 * 100.0 / nums.len() as f64))
            }
            _ => None,
        }
    }
}

/// 统计结果保留最多两位小数，整数不显示 `.0`。
fn fmt_num(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

/// 解析 `{字段}` / `{{内置变量}}` 占位符。
fn interpolate(text: &str, ctx: &Ctx) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '{' {
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                if let Some(end) = find_seq(&chars, i + 2, &['}', '}']) {
                    let key: String = chars[i + 2..end].iter().collect();
                    out.push_str(&ctx.lookup(&key));
                    i = end + 2;
                    continue;
                }
            } else if let Some(end) = find_seq(&chars, i + 1, &['}']) {
                let key: String = chars[i + 1..end].iter().collect();
                out.push_str(&ctx.lookup(&key));
                i = end + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn find_seq(chars: &[char], from: usize, seq: &[char]) -> Option<usize> {
    let mut i = from;
    while i + seq.len() <= chars.len() {
        if chars[i..i + seq.len()] == *seq {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn char_width_mm(ch: char, size_pt: f32) -> f32 {
    let c = ch as u32;
    let units = if c >= 0x2E80 {
        1.0
    } else if c < 0x80 {
        0.53
    } else {
        0.85
    };
    units * size_pt * MM_PER_PT
}

fn text_width_mm(s: &str, size_pt: f32) -> f32 {
    s.chars().map(|ch| char_width_mm(ch, size_pt)).sum()
}

/// 按元素宽度把文本拆成多行：保留显式换行，超宽时在字符边界折行。
fn wrap_text(text: &str, size_pt: f32, max_w: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for para in text.split('\n') {
        if max_w <= 0.0 {
            lines.push(para.to_string());
            continue;
        }
        let mut line = String::new();
        let mut width = 0.0f32;
        for ch in para.chars() {
            let w = char_width_mm(ch, size_pt);
            if width + w > max_w && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                width = 0.0;
            }
            line.push(ch);
            width += w;
        }
        lines.push(line);
    }
    lines
}

fn color_of(spec: &str) -> Option<Color> {
    util::parse_hex_color(spec).map(|(r, g, b)| Color::Rgb(Rgb::new(r, g, b, None)))
}

struct Origin {
    x: f32,
    y_top: f32,
}

fn draw_text(
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    text: &str,
    el: &TemplateElement,
    origin: &Origin,
    page_h: f32,
) {
    if text.is_empty() {
        return;
    }
    let fs_mm = el.font_size * MM_PER_PT;
    let (x0, w) = (origin.x + el.x_mm, el.w_mm.max(0.0));
    layer.set_fill_color(color_of(&el.color).unwrap_or(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None))));

    // 旋转文字：绕元素中心旋转，适合桌贴/门签的侧边文字；旋转时不折行。
    if el.rotate.abs() > 0.1 {
        let cx = origin.x + el.x_mm + el.w_mm / 2.0;
        let cy = page_h - (origin.y_top + el.y_mm + el.h_mm / 2.0);
        let tw = text_width_mm(text, el.font_size);
        let local_x = match el.align.as_str() {
            "left" => -el.w_mm / 2.0,
            "right" => el.w_mm / 2.0 - tw,
            _ => -tw / 2.0,
        };
        layer.save_graphics_state();
        layer.set_ctm(CurTransMat::TranslateRotate(
            Pt(cx * PT_PER_MM),
            Pt(cy * PT_PER_MM),
            el.rotate,
        ));
        layer.use_text(text, el.font_size, Mm(local_x), Mm(-fs_mm * 0.30), font);
        if el.bold {
            layer.use_text(
                text,
                el.font_size,
                Mm(local_x + 0.18),
                Mm(-fs_mm * 0.30),
                font,
            );
        }
        layer.restore_graphics_state();
        return;
    }

    let draw_line = |line: &str, top: f32| {
        let tw = text_width_mm(line, el.font_size);
        let x = match el.align.as_str() {
            "center" => x0 + (w - tw) / 2.0,
            "right" => x0 + w - tw,
            _ => x0,
        };
        let baseline = page_h - (top + fs_mm * 0.80);
        layer.use_text(line, el.font_size, Mm(x), Mm(baseline), font);
        if el.bold {
            // 没有独立粗体字体，用微小偏移重绘模拟加粗
            layer.use_text(line, el.font_size, Mm(x + 0.18), Mm(baseline), font);
        }
    };

    if el.wrap && w > 0.0 {
        let lines = wrap_text(text, el.font_size, w);
        let line_h = fs_mm * 1.35;
        let bottom = origin.y_top + el.y_mm + el.h_mm.max(line_h);
        for (index, line) in lines.iter().enumerate() {
            let top = origin.y_top + el.y_mm + index as f32 * line_h;
            if index > 0 && top + fs_mm > bottom + 0.01 {
                break;
            }
            draw_line(line, top);
        }
        return;
    }

    let top_offset = ((el.h_mm - fs_mm) / 2.0).max(0.0);
    draw_line(text, origin.y_top + el.y_mm + top_offset);
}

fn draw_rect(layer: &PdfLayerReference, el: &TemplateElement, origin: &Origin, page_h: f32) {
    let llx = origin.x + el.x_mm;
    let ury = page_h - (origin.y_top + el.y_mm);
    let lly = ury - el.h_mm;
    let urx = llx + el.w_mm;
    // 填充色非法时按「无填充」处理，不能 panic 掉整个渲染任务。
    let fill_color = color_of(&el.fill);
    let has_fill = fill_color.is_some();
    if let Some(c) = fill_color {
        layer.set_fill_color(c);
    }
    if el.border {
        layer.set_outline_color(
            color_of(&el.color).unwrap_or(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None))),
        );
        layer.set_outline_thickness(0.4);
    }
    let mode = match (has_fill, el.border) {
        (true, true) => PaintMode::FillStroke,
        (true, false) => PaintMode::Fill,
        (false, true) => PaintMode::Stroke,
        (false, false) => return,
    };
    layer.add_rect(Rect::new(Mm(llx), Mm(lly), Mm(urx), Mm(ury)).with_mode(mode));
}

fn draw_image(
    doc: &PdfDocumentReference,
    layer: &PdfLayerReference,
    el: &TemplateElement,
    origin: &Origin,
    page_h: f32,
    path: Option<std::path::PathBuf>,
) {
    let llx = origin.x + el.x_mm;
    let ury = page_h - (origin.y_top + el.y_mm);
    let lly = ury - el.h_mm;
    if let Some(p) = path {
        if let Ok(bytes) = std::fs::read(&p) {
            if let Ok(img) = printpdf::image_crate::load_from_memory(&bytes) {
                if place_image(layer, el, llx, lly, &img) {
                    let _ = doc;
                    return;
                }
            }
        }
    }
    // 找不到照片时画一个空框，便于排查
    if el.border || !el.fill.trim().is_empty() {
        let mut placeholder = el.clone();
        placeholder.border = true;
        placeholder.fill = if el.fill.trim().is_empty() {
            "#F2F2F2".to_string()
        } else {
            el.fill.clone()
        };
        draw_rect(layer, &placeholder, origin, page_h);
    }
}

/// 计算图片在元素矩形内的缩放比例与居中偏移（mm）。
///
/// - `stretch`：横向、纵向分别铺满，可能变形；
/// - `contain`：等比缩放至完整放入元素，并居中留白，人像照不会拉伸变形。
fn photo_layout(el: &TemplateElement, img_w: f32, img_h: f32) -> (f32, f32, f32, f32) {
    let (scale_x, scale_y) = if el.photo_fit == "contain" {
        let uniform = ((el.w_mm * PT_PER_MM) / img_w).min((el.h_mm * PT_PER_MM) / img_h);
        (uniform, uniform)
    } else {
        ((el.w_mm * PT_PER_MM) / img_w, (el.h_mm * PT_PER_MM) / img_h)
    };
    let draw_w = img_w * scale_x / PT_PER_MM;
    let draw_h = img_h * scale_y / PT_PER_MM;
    (
        scale_x,
        scale_y,
        (el.w_mm - draw_w) / 2.0,
        (el.h_mm - draw_h) / 2.0,
    )
}

/// 把内存中的图片按元素矩形铺到 PDF 层上；尺寸非法时返回 false。
fn place_image(
    layer: &PdfLayerReference,
    el: &TemplateElement,
    llx: f32,
    lly: f32,
    img: &printpdf::image_crate::DynamicImage,
) -> bool {
    let (pw, ph) = (img.width() as f32, img.height() as f32);
    if pw <= 0.0 || ph <= 0.0 {
        return false;
    }
    let (scale_x, scale_y, dx, dy) = photo_layout(el, pw, ph);
    let xobj = ImageXObject::from_dynamic_image(img);
    Image::from(xobj).add_to_layer(
        layer.clone(),
        ImageTransform {
            translate_x: Some(Mm(llx + dx)),
            translate_y: Some(Mm(lly + dy)),
            rotate: None,
            scale_x: Some(scale_x),
            scale_y: Some(scale_y),
            dpi: Some(IMAGE_DPI),
        },
    );
    true
}

/// 把文本编码成二维码并绘制到元素矩形中；内容为空或编码失败时留空。
fn draw_qr(
    layer: &PdfLayerReference,
    el: &TemplateElement,
    origin: &Origin,
    text: &str,
    page_h: f32,
) {
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else {
        return;
    };
    let modules = code.width();
    let quiet = 2usize;
    let size = modules + quiet * 2;
    let mut img = printpdf::image_crate::RgbImage::new(size as u32, size as u32);
    for pixel in img.pixels_mut() {
        *pixel = printpdf::image_crate::Rgb([255, 255, 255]);
    }
    for y in 0..modules {
        for x in 0..modules {
            if code[(x, y)] == qrcode::Color::Dark {
                img.put_pixel(
                    (x + quiet) as u32,
                    (y + quiet) as u32,
                    printpdf::image_crate::Rgb([0, 0, 0]),
                );
            }
        }
    }
    let llx = origin.x + el.x_mm;
    let ury = page_h - (origin.y_top + el.y_mm);
    let lly = ury - el.h_mm;
    let img = printpdf::image_crate::DynamicImage::ImageRgb8(img);
    place_image(layer, el, llx, lly, &img);
}

/// Code 128 的 107 组条空宽度（每组 6 位，终止符 7 位）。
const CODE128_PATTERNS: [&str; 107] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212",
    "221213", "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221",
    "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221",
    "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
    "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131",
    "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131",
    "311123", "311321", "331121", "312113", "312311", "332111", "314111", "221411", "431111",
    "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
    "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242",
    "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311",
    "113141", "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
];

/// Code 128B 编码：仅支持可打印 ASCII（32~126），返回码字序列（含起始、校验与终止）。
fn code128b_values(text: &str) -> Option<Vec<u8>> {
    let mut values: Vec<u8> = vec![104]; // Start B
    for b in text.bytes() {
        if !(32..=126).contains(&b) {
            return None;
        }
        values.push(b - 32);
    }
    let mut checksum = 104u32;
    for (pos, value) in values.iter().skip(1).enumerate() {
        checksum += *value as u32 * (pos as u32 + 1);
    }
    values.push((checksum % 103) as u8);
    values.push(106); // Stop
    Some(values)
}

/// 把文本编码成 Code 128 条形码，按元素矩形铺满宽度；内容非法时留空。
fn draw_barcode(
    layer: &PdfLayerReference,
    el: &TemplateElement,
    origin: &Origin,
    text: &str,
    page_h: f32,
) {
    let Some(values) = code128b_values(text.trim()) else {
        return;
    };
    let modules: u32 = values
        .iter()
        .map(|v| {
            CODE128_PATTERNS[*v as usize]
                .bytes()
                .map(|b| (b - b'0') as u32)
                .sum::<u32>()
        })
        .sum();
    let quiet = 10u32; // 两侧静区，保证扫码枪可识别
    let total = modules + quiet * 2;
    let module_w = el.w_mm / total as f32;
    if module_w <= 0.0 {
        return;
    }
    let llx = origin.x + el.x_mm + quiet as f32 * module_w;
    let ury = page_h - (origin.y_top + el.y_mm);
    let lly = (ury - el.h_mm).min(ury);
    layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
    let mut x = llx;
    for value in values {
        let mut is_bar = true;
        for ch in CODE128_PATTERNS[value as usize].bytes() {
            let w = (ch - b'0') as f32 * module_w;
            if is_bar {
                layer.add_rect(
                    Rect::new(Mm(x), Mm(lly), Mm(x + w), Mm(ury)).with_mode(PaintMode::Fill),
                );
            }
            x += w;
            is_bar = !is_bar;
        }
    }
}

/// 计算明细表各列宽度：固定宽度优先，剩余宽度由未设置宽度的列平均分配。
fn table_column_widths(setup: &TableSetup, total: f32) -> Vec<f32> {
    let fixed: f32 = setup.columns.iter().map(|c| c.width_mm.max(0.0)).sum();
    let flexible = setup.columns.iter().filter(|c| c.width_mm <= 0.0).count();
    let share = if flexible > 0 {
        (total - fixed).max(1.0) / flexible as f32
    } else {
        0.0
    };
    setup
        .columns
        .iter()
        .map(|c| if c.width_mm > 0.0 { c.width_mm } else { share })
        .collect()
}

/// 数字汇总的展示：最多两位小数，整数不带小数位。
fn format_summary_number(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if (rounded - rounded.round()).abs() < 1e-9 {
        format!("{}", rounded.round() as i64)
    } else {
        format!("{rounded:.2}")
    }
}

/// 计算明细表汇总行；没有任何列配置汇总时返回 None。
fn table_summary_row(
    setup: &TableSetup,
    rows: &[Vec<String>],
    index: &HashMap<String, usize>,
) -> Option<Vec<String>> {
    if !setup.columns.iter().any(|c| !c.summary.trim().is_empty()) {
        return None;
    }
    let mut out: Vec<String> = setup.columns.iter().map(|_| String::new()).collect();
    for (i, col) in setup.columns.iter().enumerate() {
        let func = col.summary.trim().to_ascii_lowercase();
        if func.is_empty() {
            if i == 0 {
                out[i] = setup.summary_label.clone();
            }
            continue;
        }
        let Some(idx) = index.get(col.field.trim()).copied() else {
            continue;
        };
        let non_empty = rows
            .iter()
            .filter_map(|r| r.get(idx))
            .filter(|s| !s.trim().is_empty())
            .count();
        let nums: Vec<f64> = rows
            .iter()
            .filter_map(|r| r.get(idx))
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .collect();
        out[i] = match func.as_str() {
            "sum" => format_summary_number(nums.iter().sum()),
            "avg" if !nums.is_empty() => {
                format_summary_number(nums.iter().sum::<f64>() / nums.len() as f64)
            }
            "count" => non_empty.to_string(),
            _ => String::new(),
        };
    }
    Some(out)
}

/// 矩阵网格的几何信息：宽度、高度、列数、行数。
struct Grid {
    w: f32,
    h: f32,
    cols: i64,
    rows: i64,
}

/// 一次绘制所需的全部上下文。
///
/// 把文档、图层、字体、页高、照片目录打包在一起，避免每个绘制函数都透传
/// 五六个参数；数据上下文 `Ctx` 仍按单元格/行单独传入。
struct Painter<'a> {
    doc: &'a PdfDocumentReference,
    layer: &'a PdfLayerReference,
    font: &'a IndirectFontRef,
    page_h: f32,
    photo_dir: Option<&'a str>,
}

impl<'a> Painter<'a> {
    fn matrix_frame(&self, mx: &MatrixSetup, origin: &Origin, grid: &Grid) {
        let cell_w = grid.w / grid.cols as f32;
        let cell_h = grid.h / grid.rows as f32;
        // 边框：整个矩阵的外轮廓
        if mx.show_border {
            let llx = origin.x;
            let ury = self.page_h - origin.y_top;
            let lly = ury - grid.h;
            let urx = llx + grid.w;
            self.layer
                .set_outline_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
            self.layer.set_outline_thickness(0.35);
            self.layer.add_rect(
                Rect::new(Mm(llx), Mm(lly), Mm(urx), Mm(ury)).with_mode(PaintMode::Stroke),
            );
        }
        // 网格：仅内部的分隔线，单元格之间无间隔
        if mx.show_grid {
            self.layer
                .set_outline_color(Color::Rgb(Rgb::new(0.45, 0.45, 0.45, None)));
            self.layer.set_outline_thickness(0.22);
            for i in 1..grid.cols {
                let x = origin.x + cell_w * i as f32;
                self.layer.add_line(Line {
                    points: vec![
                        (Point::new(Mm(x), Mm(self.page_h - origin.y_top)), false),
                        (
                            Point::new(Mm(x), Mm(self.page_h - origin.y_top - grid.h)),
                            false,
                        ),
                    ],
                    is_closed: false,
                });
            }
            for j in 1..grid.rows {
                let y = self.page_h - (origin.y_top + cell_h * j as f32);
                self.layer.add_line(Line {
                    points: vec![
                        (Point::new(Mm(origin.x), Mm(y)), false),
                        (Point::new(Mm(origin.x + grid.w), Mm(y)), false),
                    ],
                    is_closed: false,
                });
            }
        }
    }

    /// 绘制明细表：先画表头，再逐行绘制数据，网格可选。
    fn table(
        &self,
        setup: &TableSetup,
        el: &TemplateElement,
        widths: &[f32],
        rows: &[Vec<String>],
        index: &HashMap<String, usize>,
        origin: &Origin,
    ) {
        let row_h = setup.row_height_mm.max(3.0);
        let header_h = if setup.show_header {
            setup.header_height_mm.max(3.0)
        } else {
            0.0
        };
        let mut cell = el.clone();
        cell.children.clear();
        cell.x_mm = 0.0;
        cell.y_mm = 0.0;
        cell.wrap = setup.wrap;
        cell.rotate = 0.0;

        let mut y = origin.y_top;
        if setup.show_header {
            let mut x = origin.x;
            for (i, col) in setup.columns.iter().enumerate() {
                let w = widths.get(i).copied().unwrap_or(0.0);
                let label = if col.label.trim().is_empty() {
                    col.field.trim()
                } else {
                    col.label.trim()
                };
                self.table_fill((x, y, w, header_h), &setup.header_fill);
                cell.bold = true;
                self.table_cell(
                    label,
                    &mut cell,
                    &col.align,
                    (x, y, w, header_h),
                    setup.show_grid,
                );
                x += w;
            }
            y += header_h;
        }
        for (ri, row) in rows.iter().enumerate() {
            let banded = setup.row_band && ri % 2 == 1;
            let mut x = origin.x;
            for (i, col) in setup.columns.iter().enumerate() {
                let w = widths.get(i).copied().unwrap_or(0.0);
                if banded {
                    self.table_fill((x, y, w, row_h), "#F7F7F7");
                }
                let text = index
                    .get(col.field.trim())
                    .and_then(|idx| row.get(*idx))
                    .map(|s| s.as_str())
                    .unwrap_or("");
                cell.bold = el.bold;
                self.table_cell(
                    text,
                    &mut cell,
                    &col.align,
                    (x, y, w, row_h),
                    setup.show_grid,
                );
                x += w;
            }
            y += row_h;
        }
    }

    /// 给表格单元格铺底色；颜色非法时跳过。
    fn table_fill(&self, rect: (f32, f32, f32, f32), color: &str) {
        let Some(c) = color_of(color) else {
            return;
        };
        let (x, y_top, w, h) = rect;
        let ury = self.page_h - y_top;
        let lly = ury - h;
        self.layer.set_fill_color(c);
        self.layer
            .add_rect(Rect::new(Mm(x), Mm(lly), Mm(x + w), Mm(ury)).with_mode(PaintMode::Fill));
    }

    /// 绘制汇总行，`geometry` 为（x, y_top, 行高）。
    fn table_summary(
        &self,
        setup: &TableSetup,
        el: &TemplateElement,
        widths: &[f32],
        summary: &[String],
        geometry: (f32, f32, f32),
    ) {
        let (x, y_top, row_h) = geometry;
        let mut cell = el.clone();
        cell.children.clear();
        cell.x_mm = 0.0;
        cell.y_mm = 0.0;
        cell.wrap = setup.wrap;
        cell.rotate = 0.0;
        cell.bold = true;
        let mut cx = x;
        for (i, col) in setup.columns.iter().enumerate() {
            let w = widths.get(i).copied().unwrap_or(0.0);
            let text = summary.get(i).map(|s| s.as_str()).unwrap_or("");
            self.table_fill((cx, y_top, w, row_h), &setup.header_fill);
            self.table_cell(
                text,
                &mut cell,
                &col.align,
                (cx, y_top, w, row_h),
                setup.show_grid,
            );
            cx += w;
        }
    }

    /// `rect` 为单元格矩形（x, y_top, 宽, 高）。
    fn table_cell(
        &self,
        text: &str,
        cell: &mut TemplateElement,
        align: &str,
        rect: (f32, f32, f32, f32),
        grid: bool,
    ) {
        let (x, y_top, w, h) = rect;
        cell.align = align.to_string();
        cell.w_mm = w;
        cell.h_mm = h;
        draw_text(
            self.layer,
            self.font,
            text,
            cell,
            &Origin { x, y_top },
            self.page_h,
        );
        if grid {
            let llx = x;
            let ury = self.page_h - y_top;
            let lly = ury - h;
            self.layer
                .set_outline_color(Color::Rgb(Rgb::new(0.55, 0.55, 0.55, None)));
            self.layer.set_outline_thickness(0.25);
            self.layer.add_rect(
                Rect::new(Mm(llx), Mm(lly), Mm(llx + w), Mm(ury)).with_mode(PaintMode::Stroke),
            );
        }
    }

    fn element(&self, el: &TemplateElement, ctx: &Ctx<'_>, origin: &Origin) {
        if el.etype == "matrix" {
            let mx = el.matrix.clone().unwrap_or_default();
            let grid = Grid {
                w: el.w_mm,
                h: el.h_mm,
                cols: mx.cols.max(1),
                rows: mx.rows.max(1),
            };
            let matrix_origin = Origin {
                x: origin.x + el.x_mm,
                y_top: origin.y_top + el.y_mm,
            };
            self.matrix_frame(&mx, &matrix_origin, &grid);
            let cell_w = grid.w / grid.cols as f32;
            let cell_h = grid.h / grid.rows as f32;
            for r in 0..grid.rows {
                for c in 0..grid.cols {
                    let cell_origin = Origin {
                        x: matrix_origin.x + c as f32 * cell_w,
                        y_top: matrix_origin.y_top + r as f32 * cell_h,
                    };
                    self.elements(&el.children, ctx, &cell_origin);
                }
            }
            return;
        }

        // 显示条件：字段取值满足条件时才渲染该元素（含子元素）。
        if !el.visible_field.trim().is_empty() {
            let actual = ctx.value(&el.visible_field).trim();
            let expected = el.visible_value.trim();
            let visible = if expected.is_empty() {
                !actual.is_empty()
            } else {
                actual == expected
            };
            if !visible {
                return;
            }
        }

        // 「内容为空时隐藏」：文本/条码类元素在占位符取值为空时整体跳过，
        // 避免留白元素在证件/名单上出现空框。
        let skip_self = el.hide_if_empty
            && match el.etype.as_str() {
                "text" | "field" | "qrcode" | "barcode" => {
                    let content = if el.text.trim().is_empty() {
                        ctx.value(&el.field).to_string()
                    } else {
                        interpolate(&el.text, ctx)
                    };
                    content.trim().is_empty()
                }
                "image" => {
                    let name = if el.photo_name.trim().is_empty() {
                        ctx.value(&el.field).to_string()
                    } else {
                        interpolate(&el.photo_name, ctx)
                    };
                    name.trim().is_empty()
                }
                _ => false,
            };
        if skip_self {
            if !el.children.is_empty() {
                let child_origin = Origin {
                    x: origin.x + el.x_mm,
                    y_top: origin.y_top + el.y_mm,
                };
                self.elements(&el.children, ctx, &child_origin);
            }
            return;
        }

        match el.etype.as_str() {
            "rect" => draw_rect(self.layer, el, origin, self.page_h),
            "line" => {
                self.layer.set_outline_color(
                    color_of(&el.color).unwrap_or(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None))),
                );
                self.layer.set_outline_thickness(0.4);
                let y = self.page_h - (origin.y_top + el.y_mm);
                self.layer.add_line(Line {
                    points: vec![
                        (Point::new(Mm(origin.x + el.x_mm), Mm(y)), false),
                        (Point::new(Mm(origin.x + el.x_mm + el.w_mm), Mm(y)), false),
                    ],
                    is_closed: false,
                });
            }
            "qrcode" => {
                let text = interpolate(&el.text, ctx);
                if !text.trim().is_empty() {
                    draw_qr(self.layer, el, origin, &text, self.page_h);
                }
            }
            "barcode" => {
                let text = interpolate(&el.text, ctx);
                if !text.trim().is_empty() {
                    draw_barcode(self.layer, el, origin, &text, self.page_h);
                }
            }
            "image" => {
                // 照片名是模板字符串：先按 `{字段}` 占位符插值，再拼接候选后缀查文件。
                // 老模板未迁移时回退到 field 绑定值。
                let name = if el.photo_name.trim().is_empty() {
                    ctx.value(&el.field).to_string()
                } else {
                    interpolate(&el.photo_name, ctx)
                };
                let dir = if el.photo_dir.trim().is_empty() {
                    self.photo_dir
                } else {
                    Some(el.photo_dir.as_str())
                };
                draw_image(
                    self.doc,
                    self.layer,
                    el,
                    origin,
                    self.page_h,
                    resolve_photo(&name, dir, &el.photo_exts),
                );
            }
            _ => {
                let text = if el.text.trim().is_empty() {
                    ctx.value(&el.field).to_string()
                } else {
                    interpolate(&el.text, ctx)
                };
                draw_text(self.layer, self.font, &text, el, origin, self.page_h);
            }
        }

        // 子元素相对父元素左上角继续绘制
        if !el.children.is_empty() {
            let child_origin = Origin {
                x: origin.x + el.x_mm,
                y_top: origin.y_top + el.y_mm,
            };
            self.elements(&el.children, ctx, &child_origin);
        }
    }

    fn elements(&self, elements: &[TemplateElement], ctx: &Ctx<'_>, origin: &Origin) {
        for el in elements {
            self.element(el, ctx, origin);
        }
    }

    /// 页眉、页脚、页码与水印。每页绘制一次，支持页面分组字段（如考场号）。
    fn page_decoration(&self, body: &TemplateBody, ctx: &Ctx<'_>, page_w: f32, m: &Margins) {
        let content_w = (page_w - m.left - m.right).max(10.0);
        let chrome_size = body.chrome_font_size.max(4.0);
        let chrome_color = body.chrome_color.as_str();
        if body.page_border {
            let content_h = (self.page_h - m.top - m.bottom).max(10.0);
            let llx = m.left;
            let ury = self.page_h - m.top;
            self.layer.set_outline_color(
                color_of(&body.page_border_color)
                    .unwrap_or(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None))),
            );
            self.layer
                .set_outline_thickness(body.page_border_width_mm.max(0.05) * PT_PER_MM);
            self.layer.add_rect(
                Rect::new(Mm(llx), Mm(ury - content_h), Mm(llx + content_w), Mm(ury))
                    .with_mode(PaintMode::Stroke),
            );
        }
        let header = interpolate(&body.header_text, ctx);
        let header_right = interpolate(&body.header_right_text, ctx);
        let y = (m.top - 8.0).max(2.0);
        if !header.trim().is_empty() {
            // 有右侧附加文字时，主文字只占左侧 60%，右侧固定右对齐。
            let main_w = if header_right.trim().is_empty() {
                content_w
            } else {
                content_w * 0.6
            };
            self.chrome_text(
                &header,
                m.left,
                y,
                main_w,
                &body.header_align,
                (chrome_size, chrome_color),
            );
        }
        if !header_right.trim().is_empty() {
            self.chrome_text(
                &header_right,
                m.left + content_w * 0.6,
                y,
                content_w * 0.4,
                "right",
                (chrome_size, chrome_color),
            );
        }
        let footer = interpolate(&body.footer_text, ctx);
        if !footer.trim().is_empty() {
            let y = self.page_h - m.bottom + 1.5;
            self.chrome_text(
                &footer,
                m.left,
                y,
                content_w * 0.72,
                &body.footer_align,
                (chrome_size, chrome_color),
            );
        }
        if body.show_page_number && !(body.page_number_skip_first && ctx.page == 1) {
            let start = body.page_number_start.max(1);
            let total = ctx.pages.max(ctx.page) + start - 1;
            let text = format!("第 {} 页 / 共 {} 页", ctx.page + start - 1, total);
            let y = self.page_h - m.bottom + 1.5;
            let x = m.left + content_w * 0.72;
            self.chrome_text(
                &text,
                x,
                y,
                content_w * 0.28,
                "right",
                (chrome_size, chrome_color),
            );
        }
        let watermark = interpolate(&body.watermark, ctx);
        if !watermark.trim().is_empty() {
            if body.watermark_tile {
                self.draw_watermark_tiles(body, &watermark, page_w, self.page_h);
            } else {
                self.draw_watermark(body, &watermark, page_w);
            }
        }
    }

    /// 整页背景图：按 cover 方式铺满并居中，缺失或解码失败时静默跳过。
    fn background(&self, template: &str, ctx: &Ctx<'_>, page_w: f32, page_h: f32) {
        let name = interpolate(template, ctx);
        if name.trim().is_empty() {
            return;
        }
        let Some(path) = resolve_photo(&name, self.photo_dir, &[]) else {
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let Ok(img) = printpdf::image_crate::load_from_memory(&bytes) else {
            return;
        };
        let (pw, ph) = (img.width() as f32, img.height() as f32);
        if pw <= 0.0 || ph <= 0.0 {
            return;
        }
        let target_w = page_w * PT_PER_MM;
        let target_h = page_h * PT_PER_MM;
        let scale = (target_w / pw).max(target_h / ph);
        let draw_w = pw * scale;
        let draw_h = ph * scale;
        let x_mm = (target_w - draw_w) / 2.0 / PT_PER_MM;
        let y_mm = (target_h - draw_h) / 2.0 / PT_PER_MM;
        let xobj = ImageXObject::from_dynamic_image(&img);
        Image::from(xobj).add_to_layer(
            self.layer.clone(),
            ImageTransform {
                translate_x: Some(Mm(x_mm)),
                translate_y: Some(Mm(y_mm)),
                rotate: None,
                scale_x: Some(scale),
                scale_y: Some(scale),
                dpi: Some(IMAGE_DPI),
            },
        );
    }

    fn chrome_text(
        &self,
        text: &str,
        x_mm: f32,
        y_top: f32,
        w_mm: f32,
        align: &str,
        style: (f32, &str),
    ) {
        let (size, color) = style;
        let tw = text_width_mm(text, size);
        let x = match align {
            "center" => x_mm + (w_mm - tw) / 2.0,
            "right" => x_mm + w_mm - tw,
            _ => x_mm,
        };
        // 默认浅灰色，避免与正文抢视线；模板可自定义颜色。
        self.layer
            .set_fill_color(color_of(color).unwrap_or(Color::Rgb(Rgb::new(0.42, 0.45, 0.5, None))));
        let baseline = self.page_h - (y_top + size * MM_PER_PT * 0.8);
        self.layer
            .use_text(text, size, Mm(x), Mm(baseline), self.font);
    }

    /// 斜向水印：浅灰、居中、旋转 30 度，画在正文之上但不遮挡阅读。
    fn draw_watermark(&self, body: &TemplateBody, text: &str, page_w: f32) {
        let size = if body.watermark_font_size > 0.0 {
            body.watermark_font_size
        } else {
            32.0
        };
        let tw = text_width_mm(text, size);
        self.layer.save_graphics_state();
        self.layer.set_ctm(CurTransMat::TranslateRotate(
            Pt(page_w / 2.0 * PT_PER_MM),
            Pt(self.page_h / 2.0 * PT_PER_MM),
            body.watermark_angle,
        ));
        self.layer.set_fill_color(
            color_of(&body.watermark_color).unwrap_or(Color::Rgb(Rgb::new(0.85, 0.85, 0.85, None))),
        );
        self.layer.use_text(
            text,
            size,
            Mm(-tw / 2.0),
            Mm(-size * MM_PER_PT / 2.0),
            self.font,
        );
        self.layer.restore_graphics_state();
    }

    /// 平铺水印：按页面尺寸铺成网格，每个水印旋转 30°，常用于试卷与证照防复印。
    fn draw_watermark_tiles(&self, body: &TemplateBody, text: &str, page_w: f32, page_h: f32) {
        let size = if body.watermark_font_size > 0.0 {
            body.watermark_font_size
        } else {
            14.0
        };
        let tw = text_width_mm(text, size);
        let cols = ((page_w / 60.0).ceil() as i32).max(2);
        let rows = ((page_h / 50.0).ceil() as i32).max(3);
        self.layer.set_fill_color(
            color_of(&body.watermark_color).unwrap_or(Color::Rgb(Rgb::new(0.88, 0.88, 0.88, None))),
        );
        for row in 0..rows {
            for col in 0..cols {
                let cx = page_w * (col as f32 + 0.5) / cols as f32;
                let cy = page_h * (row as f32 + 0.5) / rows as f32;
                self.layer.save_graphics_state();
                self.layer.set_ctm(CurTransMat::TranslateRotate(
                    Pt(cx * PT_PER_MM),
                    Pt(cy * PT_PER_MM),
                    body.watermark_angle,
                ));
                self.layer.use_text(
                    text,
                    size,
                    Mm(-tw / 2.0),
                    Mm(-size * MM_PER_PT / 2.0),
                    self.font,
                );
                self.layer.restore_graphics_state();
            }
        }
    }
}

/// 默认照片后缀；模板未显式配置时按这个顺序尝试。
pub const DEFAULT_PHOTO_EXTS: &[&str] = &["jpg", "jpeg", "png"];

/// 在目录中按「名称 + 候选后缀」查找照片文件。
///
/// 查找顺序：
/// 1. 名称本身已经是完整路径或带后缀，先按原样尝试（绝对路径优先）；
/// 2. 名称 + 每个候选后缀，依次尝试小写 / 大写扩展名（兼容 Windows 导出）。
///
/// 目录为空时只按名称本身查找，避免把相对路径解析到不可预期的位置。
fn resolve_photo(raw: &str, dir: Option<&str>, exts: &[String]) -> Option<std::path::PathBuf> {
    let name = raw.trim();
    if name.is_empty() {
        return None;
    }
    let raw_path = std::path::Path::new(name);
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    let dir = dir.map(str::trim).filter(|d| !d.is_empty());

    // 1) 原样路径：绝对路径或相对当前工作目录
    candidates.push(raw_path.to_path_buf());
    if let Some(d) = dir {
        candidates.push(std::path::Path::new(d).join(name));
    }

    // 2) 名称 + 后缀；名称已带后缀时不再重复追加
    if raw_path.extension().is_none() {
        let exts: Vec<String> = if exts.is_empty() {
            DEFAULT_PHOTO_EXTS.iter().map(|e| e.to_string()).collect()
        } else {
            exts.iter()
                .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
                .filter(|e| {
                    !e.is_empty() && e.len() <= 8 && e.bytes().all(|b| b.is_ascii_alphanumeric())
                })
                .collect()
        };
        for ext in exts {
            for variant in [ext.clone(), ext.to_ascii_uppercase()] {
                let file = format!("{name}.{variant}");
                if let Some(d) = dir {
                    candidates.push(std::path::Path::new(d).join(&file));
                }
                candidates.push(std::path::PathBuf::from(file));
            }
        }
    }

    candidates.into_iter().find(|p| p.is_file())
}

fn page_size_mm(p: &PageSetup) -> (f32, f32) {
    let (w, h) = match p.size.as_str() {
        "A3" => (297.0, 420.0),
        "A5" => (148.0, 210.0),
        "Letter" => (215.9, 279.4),
        "Custom" if p.width_mm > 0.0 && p.height_mm > 0.0 => (p.width_mm, p.height_mm),
        _ => (210.0, 297.0),
    };
    if p.orientation == "landscape" {
        (h, w)
    } else {
        (w, h)
    }
}

/// 一页对应的数据：`value` 是页面分组字段的取值（如考场号），
/// `rows` 是本页包含的数据行下标。
struct PageSlice {
    value: String,
    rows: Vec<usize>,
}

/// 自然排序：`考场2` 排在 `考场10` 之前，纯数字按数值比较。
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        let mut cur = String::new();
        let mut cur_digit = None;
        for ch in s.chars() {
            let is_digit = ch.is_ascii_digit();
            if cur_digit != Some(is_digit) && !cur.is_empty() {
                out.push((cur_digit.unwrap_or(false), std::mem::take(&mut cur)));
            }
            cur.push(ch);
            cur_digit = Some(is_digit);
        }
        if !cur.is_empty() {
            out.push((cur_digit.unwrap_or(false), cur));
        }
        out
    }
    let (ca, cb) = (chunks(a.trim()), chunks(b.trim()));
    for i in 0..ca.len().max(cb.len()) {
        match (ca.get(i), cb.get(i)) {
            (Some((da, sa)), Some((db, sb))) => {
                let ord = if *da && *db {
                    match (sa.parse::<u128>(), sb.parse::<u128>()) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        _ => sa.cmp(sb),
                    }
                } else {
                    sa.cmp(sb)
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (None, None) => break,
        }
    }
    Ordering::Equal
}

/// 多级行比较：按（字段下标, 是否倒序）依次比较，全部相等时保持原顺序。
fn row_cmp(rows: &[Vec<String>], a: usize, b: usize, keys: &[(usize, bool)]) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    for (idx, desc) in keys {
        let sa = rows[a].get(*idx).map(|s| s.as_str()).unwrap_or("");
        let sb = rows[b].get(*idx).map(|s| s.as_str()).unwrap_or("");
        let ord = natural_cmp(sa, sb);
        if ord != Ordering::Equal {
            return if *desc { ord.reverse() } else { ord };
        }
    }
    Ordering::Equal
}

/// 按页面字段把行切成页；未指定字段时按每页格子数顺序切分。
/// 同组数据超过一页时顺延到下一页（页面字段值保持不变），避免丢数据。
fn build_matrix_slices(
    rows: &[Vec<String>],
    page_idx: Option<usize>,
    per_page: usize,
) -> Vec<PageSlice> {
    let per_page = per_page.max(1);
    let mut slices: Vec<PageSlice> = Vec::new();
    if let Some(pi) = page_idx {
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, row) in rows.iter().enumerate() {
            let key = row.get(pi).cloned().unwrap_or_default().trim().to_string();
            groups.entry(key).or_default().push(i);
        }
        let mut keys: Vec<String> = groups.keys().cloned().collect();
        keys.sort_by(|a, b| natural_cmp(a, b));
        for key in keys {
            let idxs = groups.remove(&key).unwrap_or_default();
            for chunk in idxs.chunks(per_page) {
                slices.push(PageSlice {
                    value: key.clone(),
                    rows: chunk.to_vec(),
                });
            }
        }
    } else {
        for chunk in (0..rows.len()).collect::<Vec<_>>().chunks(per_page) {
            slices.push(PageSlice {
                value: String::new(),
                rows: chunk.to_vec(),
            });
        }
    }
    slices
}

/// 计算矩阵每个格子的数据行：索引字段值 N 放进第 N 个格子；
/// 索引缺失、越界或冲突的数据顺延到剩余空格。
fn build_cells(
    rows: &[Vec<String>],
    row_nos: &[usize],
    seat_idx: Option<usize>,
    per_page: usize,
) -> Vec<Option<usize>> {
    let mut cells: Vec<Option<usize>> = vec![None; per_page];
    let mut leftovers: Vec<usize> = Vec::new();
    for &row_no in row_nos {
        if let Some(si) = seat_idx {
            let seat = rows
                .get(row_no)
                .and_then(|r| r.get(si))
                .and_then(|s| s.trim().parse::<i64>().ok());
            if let Some(v) = seat {
                let pos = v - 1;
                if pos >= 0 && (pos as usize) < per_page && cells[pos as usize].is_none() {
                    cells[pos as usize] = Some(row_no);
                    continue;
                }
            }
        }
        leftovers.push(row_no);
    }
    for row_no in leftovers {
        if let Some(slot) = cells.iter_mut().position(|c| c.is_none()) {
            cells[slot] = Some(row_no);
        }
    }
    cells
}

/// 纯渲染入口：数据已提前读出，因此在 CPU 密集的排版过程中不再占用数据库连接。
///
/// 分页规则：
/// - 未设置页面字段：单页模板一行一页，矩阵模板按格子数顺序分页（原有行为）；
/// - 设置了页面字段（如考场号）：按该字段分组，每组从一个新页开始，页面按取值自然排序；
/// - 设置了矩阵索引字段（如座位号）：格子位置由索引值决定（1 表示第 1 格），
///   非法或冲突的索引按剩余空格顺序补位，避免丢数据。
pub fn render_pdf_data(
    col_meta: &[DatasetColumn],
    rows: &[Vec<String>],
    body: &TemplateBody,
    kind: &str,
    opts: &RenderOpts,
) -> AppResult<Vec<u8>> {
    if col_meta.is_empty() {
        return Err(AppError::bad_request("数据集没有字段，无法生成 PDF"));
    }
    let index: HashMap<String, usize> = col_meta
        .iter()
        .enumerate()
        .map(|(i, c)| (c.name.clone(), i))
        .collect();

    // 模板根声明的必需字段（含子元素占位符冒泡上来的字段）必须在输入数据中存在。
    let required = body.all_required_fields();
    let missing: Vec<String> = required
        .iter()
        .filter(|f| !index.contains_key(f.as_str()))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(AppError::bad_request(format!(
            "PDF 模板需要字段：{}；输入数据缺少：{}",
            required.join("、"),
            missing.join("、")
        )));
    }

    let (page_w, page_h) = page_size_mm(&body.page);
    let m = &body.page.margin_mm;
    let content_x = m.left;
    let content_y_top = m.top;

    let (doc, first_page, first_layer) =
        PdfDocument::new("考务数据导出", Mm(page_w), Mm(page_h), "Layer 1");
    let font = doc
        .add_external_font(FONT_BYTES)
        .map_err(|e| AppError::internal(format!("加载中文字体失败: {e}")))?;

    let photo_dir = opts.photo_dir.clone();
    let total_rows = rows.len() as i64;
    let page_idx = index
        .get(body.page_field.trim())
        .copied()
        .filter(|_| !body.page_field.trim().is_empty());
    let seat_idx = index
        .get(body.matrix_index_field.trim())
        .copied()
        .filter(|_| !body.matrix_index_field.trim().is_empty());
    let sort_idx = index
        .get(body.sort_field.trim())
        .copied()
        .filter(|_| !body.sort_field.trim().is_empty());
    // 排序键：先按页面分组字段升序，再按模板排序字段（可倒序）。
    let sort_keys: Vec<(usize, bool)> = {
        let mut keys = Vec::new();
        if let Some(pi) = page_idx {
            keys.push((pi, false));
        }
        if let Some(si) = sort_idx {
            keys.push((si, body.sort_order == "desc"));
        }
        keys
    };

    if kind == "matrix" {
        let matrix_el = body.elements.iter().find(|e| e.etype == "matrix");
        let (cols, grid_rows) = matrix_el
            .and_then(|e| e.matrix.as_ref())
            .map(|m| (m.cols.max(1), m.rows.max(1)))
            .unwrap_or((1, 1));
        let per_page = (cols * grid_rows).max(1) as usize;

        // 1) 按页面字段分组，每组独立分页；页面按字段取值自然排序。
        let mut slices = build_matrix_slices(rows, page_idx, per_page);
        if slices.is_empty() {
            slices.push(PageSlice {
                value: String::new(),
                rows: Vec::new(),
            });
        }

        let pages = slices.len() as i64;
        let start_page = opts.only_page.unwrap_or(1).max(1);
        let end_page = if opts.only_page.is_some() {
            start_page
        } else {
            pages
        };

        let mut first_used = false;
        for page in start_page..=end_page.min(pages) {
            let slice = &slices[(page - 1) as usize];
            let (pi, li) = if !first_used {
                first_used = true;
                (first_page, first_layer)
            } else {
                doc.add_page(Mm(page_w), Mm(page_h), format!("P{page}"))
            };
            let layer = doc.get_page(pi).get_layer(li);
            let painter = Painter {
                doc: &doc,
                layer: &layer,
                font: &font,
                page_h,
                photo_dir: photo_dir.as_deref(),
            };
            let on_page = slice.rows.len() as i64;

            // 页面级元素只认识本页的分组值（如考场号），不会串入某个考生的数据。
            let mut page_row: Vec<String> = vec![String::new(); col_meta.len()];
            if let Some(pi) = page_idx {
                if pi < page_row.len() {
                    page_row[pi] = slice.value.clone();
                }
            }
            let page_ctx = Ctx {
                row: &page_row,
                index: &index,
                all_rows: rows,
                index_no: 0,
                no: slice.rows.first().map(|i| *i as i64 + 1).unwrap_or(0),
                page,
                pages,
                count: on_page,
            };
            let page_origin = Origin {
                x: content_x,
                y_top: content_y_top,
            };

            painter.background(&body.background_image, &page_ctx, page_w, page_h);

            // 页面级元素：标题、标题框等（矩阵本身单独处理）
            for el in body.elements.iter().filter(|e| e.etype != "matrix") {
                painter.element(el, &page_ctx, &page_origin);
            }

            if let Some(el) = matrix_el {
                let mx = el.matrix.clone().unwrap_or_default();
                let matrix_origin = Origin {
                    x: content_x + el.x_mm,
                    y_top: content_y_top + el.y_mm,
                };
                painter.matrix_frame(
                    &mx,
                    &matrix_origin,
                    &Grid {
                        w: el.w_mm,
                        h: el.h_mm,
                        cols,
                        rows: grid_rows,
                    },
                );
                let cell_w = el.w_mm / cols as f32;
                let cell_h = el.h_mm / grid_rows as f32;

                // 2) 计算每个格子的数据：座位号（索引字段）→ 格子位置；
                //    索引缺失/非法/重复的数据顺延到剩余空格，保证不丢人。
                let cells = build_cells(rows, &slice.rows, seat_idx, per_page);

                for (k, slot) in cells.iter().enumerate() {
                    let Some(row_no) = *slot else { continue };
                    let row = &rows[row_no];
                    let cx = k % cols as usize;
                    let cy = k / cols as usize;
                    let origin = Origin {
                        x: matrix_origin.x + cx as f32 * cell_w,
                        y_top: matrix_origin.y_top + cy as f32 * cell_h,
                    };
                    let ctx = Ctx {
                        row,
                        index: &index,
                        all_rows: rows,
                        index_no: k as i64 + 1,
                        no: row_no as i64 + 1,
                        page,
                        pages,
                        count: on_page,
                    };
                    painter.elements(&el.children, &ctx, &origin);
                }
            }
            painter.page_decoration(body, &page_ctx, page_w, &body.page.margin_mm);
        }
    } else if kind == "table" {
        // 明细表：数据按行高铺满元素区域，超出自动分页并重复表头。
        let table_el = body
            .elements
            .iter()
            .find(|e| e.etype == "table")
            .ok_or_else(|| AppError::bad_request("明细表模板需要插入一个「明细表」元素"))?;
        let setup = table_el.table.clone().unwrap_or_default();
        if setup.columns.is_empty() {
            return Err(AppError::bad_request("明细表至少需要配置一列"));
        }
        for col in &setup.columns {
            let f = col.field.trim();
            if f.is_empty() {
                return Err(AppError::bad_request("明细表存在未绑定字段的列"));
            }
            if !index.contains_key(f) {
                return Err(AppError::bad_request(format!(
                    "明细表字段 [{f}] 在输入数据中不存在"
                )));
            }
        }
        let row_h = setup.row_height_mm.max(3.0);
        let header_h = if setup.show_header {
            setup.header_height_mm.max(3.0)
        } else {
            0.0
        };
        let summary = table_summary_row(&setup, rows, &index);
        let summary_h = if summary.is_some() { row_h } else { 0.0 };
        let per_page = (((table_el.h_mm - header_h - summary_h) / row_h).floor() as usize).max(1);
        let widths = table_column_widths(&setup, table_el.w_mm.max(10.0));

        // 设置页面字段时按该字段分组，每组另起一页；组内再按行高分页，
        // 组内顺序由模板排序字段决定。
        let order: Vec<usize> = if sort_keys.is_empty() {
            (0..rows.len()).collect()
        } else {
            let mut v: Vec<usize> = (0..rows.len()).collect();
            v.sort_by(|&a, &b| row_cmp(rows, a, b, &sort_keys));
            v
        };
        let mut page_groups: Vec<(Vec<usize>, String)> = Vec::new();
        let mut current: Vec<usize> = Vec::new();
        let mut group_value = String::new();
        for idx in order {
            let group = page_idx
                .and_then(|pi| rows[idx].get(pi).cloned())
                .unwrap_or_default();
            if !current.is_empty()
                && (current.len() >= per_page || (page_idx.is_some() && group != group_value))
            {
                page_groups.push((std::mem::take(&mut current), group_value.clone()));
            }
            if current.is_empty() {
                group_value = group;
            }
            current.push(idx);
        }
        if !current.is_empty() {
            page_groups.push((current, group_value));
        }
        if page_groups.is_empty() {
            page_groups.push((Vec::new(), String::new()));
        }
        let pages = page_groups.len() as i64;
        let start_page = opts.only_page.unwrap_or(1).max(1);
        let end_page = if opts.only_page.is_some() {
            start_page
        } else {
            pages
        };
        let mut first_used = false;
        for page in start_page..=end_page.min(pages) {
            let (pi, li) = if !first_used {
                first_used = true;
                (first_page, first_layer)
            } else {
                doc.add_page(Mm(page_w), Mm(page_h), format!("P{page}"))
            };
            let layer = doc.get_page(pi).get_layer(li);
            let painter = Painter {
                doc: &doc,
                layer: &layer,
                font: &font,
                page_h,
                photo_dir: photo_dir.as_deref(),
            };
            let (indices, group) = &page_groups[(page - 1) as usize];
            let page_rows: Vec<Vec<String>> = indices.iter().map(|i| rows[*i].clone()).collect();
            // 页面级元素只认识本页的分组值（如考场号 / 学校）。
            let mut page_row: Vec<String> = vec![String::new(); col_meta.len()];
            if let Some(pi) = page_idx {
                if pi < page_row.len() {
                    page_row[pi] = group.clone();
                }
            }
            let page_ctx = Ctx {
                row: &page_row,
                index: &index,
                all_rows: rows,
                index_no: 0,
                no: indices.first().map(|i| *i as i64 + 1).unwrap_or(0),
                page,
                pages,
                count: indices.len() as i64,
            };
            let page_origin = Origin {
                x: content_x,
                y_top: content_y_top,
            };
            painter.background(&body.background_image, &page_ctx, page_w, page_h);
            for el in body.elements.iter().filter(|e| e.etype != "table") {
                painter.element(el, &page_ctx, &page_origin);
            }
            let table_origin = Origin {
                x: content_x + table_el.x_mm,
                y_top: content_y_top + table_el.y_mm,
            };
            painter.table(&setup, table_el, &widths, &page_rows, &index, &table_origin);
            if page == pages {
                if let Some(summary) = &summary {
                    // 汇总行固定在数据区底部，不随最后一页行数浮动。
                    let summary_y = table_origin.y_top + header_h + per_page as f32 * row_h;
                    painter.table_summary(
                        &setup,
                        table_el,
                        &widths,
                        summary,
                        (table_origin.x, summary_y, row_h),
                    );
                }
            }
            painter.page_decoration(body, &page_ctx, page_w, &body.page.margin_mm);
        }
    } else {
        // 单页模板：默认一行一页；设置页面字段后按该字段自然排序，使同考场的页面聚在一起。
        let order: Vec<usize> = if sort_keys.is_empty() {
            (0..rows.len()).collect()
        } else {
            let mut v: Vec<usize> = (0..rows.len()).collect();
            v.sort_by(|&a, &b| row_cmp(rows, a, b, &sort_keys));
            v
        };
        let pages = order.len().max(1) as i64;
        let start_page = opts.only_page.unwrap_or(1).max(1);
        let end_page = if opts.only_page.is_some() {
            start_page
        } else {
            pages
        };
        let mut first_used = false;
        for page in start_page..=end_page.min(pages.max(1)) {
            let (pi, li) = if !first_used {
                first_used = true;
                (first_page, first_layer)
            } else {
                doc.add_page(Mm(page_w), Mm(page_h), format!("P{page}"))
            };
            let layer = doc.get_page(pi).get_layer(li);
            let painter = Painter {
                doc: &doc,
                layer: &layer,
                font: &font,
                page_h,
                photo_dir: photo_dir.as_deref(),
            };
            let row_no = order.get((page - 1) as usize).copied();
            let row: &[String] = row_no.map(|i| rows[i].as_slice()).unwrap_or(&[]);
            let ctx = Ctx {
                row,
                index: &index,
                all_rows: rows,
                index_no: page,
                no: row_no.map(|i| i as i64 + 1).unwrap_or(0),
                page,
                pages,
                count: total_rows,
            };
            let origin = Origin {
                x: content_x,
                y_top: content_y_top,
            };
            painter.background(&body.background_image, &ctx, page_w, page_h);
            painter.elements(&body.elements, &ctx, &origin);
            painter.page_decoration(body, &ctx, page_w, &body.page.margin_mm);
        }
    }

    let mut writer = std::io::BufWriter::new(std::io::Cursor::new(Vec::<u8>::new()));
    doc.save(&mut writer)
        .map_err(|e| AppError::internal(format!("PDF 写入失败: {e}")))?;
    let buf = writer
        .into_inner()
        .map_err(|e| AppError::internal(format!("PDF 写入失败: {e}")))?
        .into_inner();
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TableColumn;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("exam_pdfgen_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn photo_name_tries_configured_extensions_in_order() {
        let dir = temp_dir("exts");
        std::fs::write(dir.join("张三.png"), b"x").expect("write png");
        std::fs::write(dir.join("张三.JPG"), b"x").expect("write jpg");

        let exts = vec!["jpg".to_string(), "png".to_string()];
        let found = resolve_photo("张三", dir.to_str(), &exts).expect("found by ext");
        assert_eq!(found.file_name().unwrap().to_string_lossy(), "张三.JPG");

        let only_png = vec!["png".to_string()];
        let found = resolve_photo("张三", dir.to_str(), &only_png).expect("found png");
        assert_eq!(found.file_name().unwrap().to_string_lossy(), "张三.png");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn photo_name_with_extension_is_used_as_is() {
        let dir = temp_dir("named");
        std::fs::write(dir.join("报名照.png"), b"x").expect("write");
        let found =
            resolve_photo("报名照.png", dir.to_str(), &["jpg".to_string()]).expect("exact name");
        assert!(found.ends_with("报名照.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_page_size_uses_configured_dimensions() {
        let p = PageSetup {
            size: "Custom".into(),
            orientation: "portrait".into(),
            margin_mm: Margins {
                top: 15.0,
                right: 12.0,
                bottom: 12.0,
                left: 12.0,
            },
            width_mm: 210.0,
            height_mm: 140.0,
        };
        assert_eq!(page_size_mm(&p), (210.0, 140.0));
        let mut landscape = p.clone();
        landscape.orientation = "landscape".into();
        assert_eq!(page_size_mm(&landscape), (140.0, 210.0));
    }

    #[test]
    fn row_cmp_sorts_by_page_then_numeric_field() {
        let rows = vec![
            vec!["2".into(), "张三".into(), "90".into()],
            vec!["1".into(), "李四".into(), "100".into()],
            vec!["1".into(), "王五".into(), "80".into()],
        ];
        let mut order: Vec<usize> = (0..rows.len()).collect();
        order.sort_by(|&a, &b| row_cmp(&rows, a, b, &[(0, false), (2, true)]));
        assert_eq!(order, vec![1, 2, 0]);
    }

    #[test]
    fn contain_photo_keeps_aspect_ratio_and_centers() {
        let mut el = TemplateElement::empty("img", "image");
        el.w_mm = 30.0;
        el.h_mm = 40.0;
        el.photo_fit = "contain".into();
        let (sx, sy, dx, dy) = photo_layout(&el, 100.0, 100.0);
        assert!((sx - sy).abs() < 1e-6, "等比缩放");
        assert!(dx.abs() < 1e-4, "水平居中");
        assert!((dy - 5.0).abs() < 1e-4, "垂直居中留白");

        el.photo_fit = "stretch".into();
        let (sx2, sy2, dx2, dy2) = photo_layout(&el, 100.0, 100.0);
        assert!((sx2 - sy2).abs() > 1e-3, "拉伸时横纵比例不同");
        assert!(dx2.abs() < 1e-4 && dy2.abs() < 1e-4);
    }

    #[test]
    fn photo_missing_returns_none_instead_of_failing() {
        let dir = temp_dir("missing");
        assert!(resolve_photo("", dir.to_str(), &[]).is_none());
        assert!(resolve_photo("不存在", dir.to_str(), &["jpg".to_string()]).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn required_fields_bubble_up_from_matrix_cells() {
        let mut body = TemplateBody {
            page_field: "考场号".into(),
            matrix_index_field: "座位号".into(),
            required_fields: vec!["身份证号".into()],
            ..Default::default()
        };
        let mut matrix = TemplateElement::empty("m", "matrix");
        let mut text = TemplateElement::empty("t", "text");
        text.text = "{姓名} / {{准考证号}} / {{page}}".into();
        matrix.children.push(text);
        body.elements.push(matrix);
        assert_eq!(
            body.all_required_fields(),
            vec!["身份证号", "考场号", "座位号", "姓名", "准考证号"]
        );
    }

    #[test]
    fn matrix_pages_follow_page_field_natural_order() {
        let rows = vec![
            vec!["1".into(), "张三".into(), "2".into()],
            vec!["1".into(), "李四".into(), "1".into()],
            vec!["2".into(), "王五".into(), "3".into()],
            vec!["10".into(), "赵六".into(), "1".into()],
        ];
        let slices = build_matrix_slices(&rows, Some(0), 4);
        let values: Vec<&str> = slices.iter().map(|s| s.value.as_str()).collect();
        assert_eq!(values, vec!["1", "2", "10"]);
        assert_eq!(slices[0].rows, vec![0, 1]);
        assert_eq!(slices[2].rows, vec![3]);

        let cells = build_cells(&rows, &slices[0].rows, Some(2), 4);
        // 座位号 1/2 → 第 0/1 格
        assert_eq!(cells, vec![Some(1), Some(0), None, None]);
    }

    #[test]
    fn duplicate_or_invalid_seat_falls_back_to_free_cells() {
        let rows = vec![
            vec!["张三".into(), "1".into()],
            vec!["李四".into(), "1".into()],
            vec!["王五".into(), "99".into()],
        ];
        let cells = build_cells(&rows, &[0, 1, 2], Some(1), 4);
        assert_eq!(cells, vec![Some(0), Some(1), Some(2), None]);
    }

    #[test]
    fn render_rejects_input_missing_required_fields() {
        let mut body = TemplateBody::default();
        let mut el = TemplateElement::empty("t", "text");
        el.text = "{姓名}".into();
        body.elements.push(el);
        let cols = vec![DatasetColumn {
            name: "考场号".into(),
            dtype: "text".into(),
        }];
        let err = render_pdf_data(&cols, &[], &body, "single", &RenderOpts::default())
            .expect_err("missing field");
        assert!(err.to_string().contains("姓名"), "实际错误：{err}");
    }

    #[test]
    fn header_footer_and_watermark_fields_are_required() {
        let body = TemplateBody {
            header_text: "{考场号}".into(),
            footer_text: "{{page}} / {{pages}} · {考试名称}".into(),
            watermark: "{考点}".into(),
            show_page_number: true,
            ..Default::default()
        };
        assert_eq!(
            body.all_required_fields(),
            vec!["考场号", "考试名称", "考点"]
        );
    }

    #[test]
    fn qrcode_element_adds_required_field_and_renders() {
        let cols = vec![DatasetColumn {
            name: "准考证号".into(),
            dtype: "text".into(),
        }];
        let mut body = TemplateBody::default();
        let mut qr = TemplateElement::empty("q", "qrcode");
        qr.text = "{准考证号}".into();
        qr.w_mm = 30.0;
        qr.h_mm = 30.0;
        body.elements.push(qr);
        assert_eq!(body.all_required_fields(), vec!["准考证号"]);
        let rows = vec![vec!["2026001".into()]];
        let pdf = render_pdf_data(&cols, &rows, &body, "single", &RenderOpts::default())
            .expect("render qr");
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn chrome_font_size_and_color_render() {
        let cols = vec![DatasetColumn {
            name: "姓名".into(),
            dtype: "text".into(),
        }];
        let body = TemplateBody {
            header_text: "考试名单".into(),
            footer_text: "内部资料".into(),
            chrome_font_size: 12.0,
            chrome_color: "#1E40AF".into(),
            show_page_number: true,
            ..Default::default()
        };
        let rows = vec![vec!["张三".into()]];
        let pdf = render_pdf_data(&cols, &rows, &body, "single", &RenderOpts::default())
            .expect("render chrome");
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn page_border_renders_without_error() {
        let cols = vec![DatasetColumn {
            name: "姓名".into(),
            dtype: "text".into(),
        }];
        let body = TemplateBody {
            page_border: true,
            page_border_width_mm: 0.6,
            page_border_color: "#1E40AF".into(),
            ..Default::default()
        };
        let rows = vec![vec!["张三".into()]];
        let pdf = render_pdf_data(&cols, &rows, &body, "single", &RenderOpts::default())
            .expect("render page border");
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn tiled_watermark_renders_repeated_text() {
        let cols = vec![DatasetColumn {
            name: "姓名".into(),
            dtype: "text".into(),
        }];
        let body = TemplateBody {
            watermark: "内部资料".into(),
            watermark_tile: true,
            ..Default::default()
        };
        let rows = vec![vec!["张三".into()]];
        let pdf = render_pdf_data(&cols, &rows, &body, "single", &RenderOpts::default())
            .expect("render tiled watermark");
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn background_image_covers_page_and_missing_file_is_ignored() {
        const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\nIDATx\x9cc\x00\x01\x00\x00\x05\x00\x01\r\n-\xb4\x00\x00\x00\x00IEND\xaeB`\x82";
        let dir = temp_dir("bg");
        std::fs::write(dir.join("bg.png"), PNG).expect("write bg");
        let cols = vec![DatasetColumn {
            name: "姓名".into(),
            dtype: "text".into(),
        }];
        let rows = vec![vec!["张三".into()]];
        let body = TemplateBody {
            background_image: "bg.png".into(),
            ..Default::default()
        };
        let opts = RenderOpts {
            photo_dir: dir.to_str().map(|s| s.to_string()),
            only_page: None,
        };
        let pdf = render_pdf_data(&cols, &rows, &body, "single", &opts).expect("render bg");
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 1000);

        // 背景图缺失时静默跳过，不影响整份文档
        let missing = TemplateBody {
            background_image: "not-exist.png".into(),
            ..Default::default()
        };
        let pdf = render_pdf_data(&cols, &rows, &missing, "single", &opts).expect("render missing");
        assert!(pdf.starts_with(b"%PDF"));

        // 背景图路径中的占位符同样计入必需字段
        let required = TemplateBody {
            background_image: "{底图}".into(),
            ..Default::default()
        };
        assert_eq!(required.all_required_fields(), vec!["底图"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrap_text_breaks_long_lines_and_keeps_explicit_newlines() {
        let lines = wrap_text("考生须知考生须知", 10.0, 20.0);
        assert!(lines.len() >= 2, "{lines:?}");
        assert!(lines.iter().all(|line| text_width_mm(line, 10.0) <= 20.01));
        assert_eq!(wrap_text("甲\n乙", 10.0, 100.0), vec!["甲", "乙"]);
        assert_eq!(wrap_text("不限宽", 10.0, 0.0), vec!["不限宽"]);
    }

    #[test]
    fn rotated_text_element_renders_without_error() {
        let cols = vec![DatasetColumn {
            name: "姓名".into(),
            dtype: "text".into(),
        }];
        let mut body = TemplateBody::default();
        let mut text = TemplateElement::empty("t", "text");
        text.text = "{姓名}".into();
        text.w_mm = 30.0;
        text.h_mm = 60.0;
        text.rotate = 90.0;
        body.elements.push(text);
        let rows = vec![vec!["张三".into()]];
        let pdf = render_pdf_data(&cols, &rows, &body, "single", &RenderOpts::default())
            .expect("render rotated text");
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn wrapped_text_element_renders_within_height() {
        let cols = vec![DatasetColumn {
            name: "须知".into(),
            dtype: "text".into(),
        }];
        let mut body = TemplateBody::default();
        let mut text = TemplateElement::empty("t", "text");
        text.text = "{须知}".into();
        text.w_mm = 20.0;
        text.h_mm = 12.0;
        text.wrap = true;
        body.elements.push(text);
        let rows = vec![vec!["考生须携带准考证和身份证按时入场".into()]];
        let pdf = render_pdf_data(&cols, &rows, &body, "single", &RenderOpts::default())
            .expect("render wrapped text");
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn barcode_encodes_code128_and_renders() {
        for (index, pattern) in CODE128_PATTERNS.iter().enumerate() {
            let expected_len = if index == 106 { 7 } else { 6 };
            let expected_sum = if index == 106 { 13 } else { 11 };
            assert_eq!(pattern.len(), expected_len, "pattern {index}");
            let sum: u32 = pattern.bytes().map(|b| (b - b'0') as u32).sum();
            assert_eq!(sum, expected_sum, "pattern {index}");
        }
        assert_eq!(code128b_values("A"), Some(vec![104, 33, 34, 106]));
        assert_eq!(code128b_values("中文"), None);

        let cols = vec![DatasetColumn {
            name: "准考证号".into(),
            dtype: "text".into(),
        }];
        let mut body = TemplateBody::default();
        let mut barcode = TemplateElement::empty("b", "barcode");
        barcode.text = "{准考证号}".into();
        barcode.w_mm = 60.0;
        barcode.h_mm = 14.0;
        body.elements.push(barcode);
        assert_eq!(body.all_required_fields(), vec!["准考证号"]);
        let rows = vec![vec!["2026001".into()]];
        let pdf = render_pdf_data(&cols, &rows, &body, "single", &RenderOpts::default())
            .expect("render barcode");
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn placeholders_support_default_values() {
        let rows = vec![vec!["".to_string(), "张三".to_string()]];
        let index: HashMap<String, usize> =
            [("备注".to_string(), 0usize), ("姓名".to_string(), 1usize)]
                .into_iter()
                .collect();
        let ctx = Ctx {
            row: &rows[0],
            index: &index,
            all_rows: &rows,
            index_no: 1,
            no: 1,
            page: 1,
            pages: 2,
            count: 1,
        };
        assert_eq!(ctx.lookup("备注|无"), "无");
        assert_eq!(ctx.lookup("姓名|无"), "张三");
        assert_eq!(ctx.lookup("page|第 1 页"), "1");

        let mut body = TemplateBody::default();
        let mut text = TemplateElement::empty("t", "text");
        text.text = "{备注|无}".into();
        body.elements.push(text);
        assert_eq!(body.all_required_fields(), vec!["备注"]);
    }

    #[test]
    fn statistics_placeholders_aggregate_all_rows() {
        let rows = vec![
            vec!["张三".to_string(), "90".to_string(), "1".to_string()],
            vec!["李四".to_string(), "70".to_string(), "1".to_string()],
            vec!["王五".to_string(), "".to_string(), "2".to_string()],
        ];
        let index: HashMap<String, usize> = [
            ("姓名".to_string(), 0usize),
            ("成绩".to_string(), 1usize),
            ("考场号".to_string(), 2usize),
        ]
        .into_iter()
        .collect();
        let ctx = Ctx {
            row: &rows[0],
            index: &index,
            all_rows: &rows,
            index_no: 1,
            no: 1,
            page: 1,
            pages: 1,
            count: 3,
        };
        assert_eq!(ctx.lookup("total"), "3");
        assert_eq!(ctx.lookup("count:姓名"), "3");
        assert_eq!(ctx.lookup("count:成绩"), "2");
        assert_eq!(ctx.lookup("distinct:考场号"), "2");
        assert_eq!(ctx.lookup("sum:成绩"), "160");
        assert_eq!(ctx.lookup("avg:成绩"), "80");
        assert_eq!(ctx.lookup("min:成绩"), "70");
        assert_eq!(ctx.lookup("max:成绩"), "90");
        assert_eq!(ctx.lookup("std:成绩"), "10");
        assert_eq!(ctx.lookup("rate:成绩:80"), "50");
        // 未知函数仍按普通字段取值，未知字段返回空串。
        assert_eq!(ctx.lookup("姓名"), "张三");
        assert_eq!(ctx.lookup("不存在"), "");
    }

    #[test]
    fn statistics_placeholder_fields_bubble_into_required_fields() {
        let body = TemplateBody {
            header_text: "总分 {{sum:成绩}}".into(),
            footer_text: "及格率 {{rate:成绩:60}}".into(),
            watermark: "共 {{total}} 人 · {{distinct:考场号}} 个考场".into(),
            ..Default::default()
        };
        assert_eq!(body.all_required_fields(), vec!["成绩", "考场号"]);
    }

    #[test]
    fn render_includes_page_decoration_without_error() {
        let cols = vec![
            DatasetColumn {
                name: "姓名".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "考场号".into(),
                dtype: "text".into(),
            },
        ];
        let mut body = TemplateBody {
            page_field: "考场号".into(),
            header_text: "第 {考场号} 考场".into(),
            header_right_text: "打印 {{date}}".into(),
            footer_text: "考试数据导出".into(),
            header_align: "center".into(),
            footer_align: "right".into(),
            watermark: "内部资料".into(),
            watermark_color: "#CCCCCC".into(),
            watermark_angle: 45.0,
            watermark_font_size: 28.0,
            show_page_number: true,
            ..Default::default()
        };
        let mut text = TemplateElement::empty("t", "text");
        text.text = "{姓名}".into();
        body.elements.push(text);
        let rows = vec![
            vec!["张三".into(), "1".into()],
            vec!["李四".into(), "2".into()],
        ];
        let pdf =
            render_pdf_data(&cols, &rows, &body, "single", &RenderOpts::default()).expect("render");
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn render_table_paginates_rows_and_repeats_header() {
        let cols = vec![
            DatasetColumn {
                name: "姓名".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "成绩".into(),
                dtype: "number".into(),
            },
        ];
        let mut table = TemplateElement::empty("tb", "table");
        table.x_mm = 0.0;
        table.y_mm = 10.0;
        table.w_mm = 180.0;
        // 表头 8mm、行高 8mm：30mm 高可容纳 2 行数据，5 条数据应分成 3 页。
        table.h_mm = 30.0;
        table.font_size = 9.0;
        table.table = Some(TableSetup {
            columns: vec![
                TableColumn {
                    field: "姓名".into(),
                    label: "姓名".into(),
                    width_mm: 60.0,
                    align: "left".into(),
                    summary: String::new(),
                },
                TableColumn {
                    field: "成绩".into(),
                    label: "成绩".into(),
                    width_mm: 0.0,
                    align: "right".into(),
                    summary: "sum".into(),
                },
            ],
            header_height_mm: 8.0,
            row_height_mm: 8.0,
            show_header: true,
            show_grid: true,
            summary_label: "合计".into(),
            wrap: true,
            header_fill: "#EEEEEE".into(),
            row_band: true,
        });
        let mut body = TemplateBody::default();
        body.elements.push(table);
        let rows = vec![
            vec!["张三".into(), "90".into()],
            vec!["李四".into(), "85".into()],
            vec!["王五".into(), "78".into()],
            vec!["赵六".into(), "66".into()],
            vec!["孙七".into(), "59".into()],
        ];
        let pdf = render_pdf_data(&cols, &rows, &body, "table", &RenderOpts::default())
            .expect("render table");
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    #[ignore = "2 万行明细表分页渲染压力测试，按需运行"]
    fn stress_table_pdf_20k_rows() {
        const N: usize = 20_000;
        let names = [
            "考场号",
            "姓名",
            "准考证号",
            "语文",
            "数学",
            "英语",
            "总成绩",
        ];
        let cols: Vec<DatasetColumn> = names
            .iter()
            .map(|n| DatasetColumn {
                name: (*n).into(),
                dtype: "text".into(),
            })
            .collect();
        let rows: Vec<Vec<String>> = (0..N)
            .map(|i| {
                let chinese = 60 + i % 41;
                let math = 50 + i % 51;
                let english = 40 + i % 61;
                vec![
                    format!("{}", i / 30 + 1),
                    format!("考生{i:06}"),
                    format!("2026{:06}", i + 1),
                    chinese.to_string(),
                    math.to_string(),
                    english.to_string(),
                    (chinese + math + english).to_string(),
                ]
            })
            .collect();

        let mut table = TemplateElement::empty("tb", "table");
        table.x_mm = 0.0;
        table.y_mm = 10.0;
        table.w_mm = 180.0;
        table.h_mm = 240.0;
        table.font_size = 8.0;
        table.table = Some(TableSetup {
            columns: names
                .iter()
                .map(|n| TableColumn {
                    field: (*n).into(),
                    label: (*n).into(),
                    width_mm: if *n == "姓名" { 40.0 } else { 0.0 },
                    align: "center".into(),
                    summary: String::new(),
                })
                .collect(),
            header_height_mm: 8.0,
            row_height_mm: 8.0,
            show_header: true,
            show_grid: true,
            summary_label: String::new(),
            wrap: false,
            header_fill: "#EEEEEE".into(),
            row_band: true,
        });
        let mut body = TemplateBody {
            header_text: "成绩明细表".into(),
            footer_text: "{{page}} / {{pages}}".into(),
            show_page_number: true,
            ..Default::default()
        };
        body.elements.push(table);

        let start = std::time::Instant::now();
        let pdf = render_pdf_data(&cols, &rows, &body, "table", &RenderOpts::default())
            .expect("渲染 2 万行明细表");
        let elapsed = start.elapsed();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 100_000, "PDF 体积异常: {} bytes", pdf.len());
        assert!(elapsed.as_secs() < 180, "2 万行渲染耗时过长: {elapsed:?}");
        println!(
            "2 万行明细表 PDF 体积: {} MB, 耗时: {:?}",
            pdf.len() / 1024 / 1024,
            elapsed
        );
    }

    #[test]
    fn render_table_groups_pages_by_page_field() {
        let cols = vec![
            DatasetColumn {
                name: "学校".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "姓名".into(),
                dtype: "text".into(),
            },
        ];
        let mut table = TemplateElement::empty("tb", "table");
        table.w_mm = 160.0;
        table.h_mm = 60.0;
        table.table = Some(TableSetup {
            columns: vec![
                TableColumn {
                    field: "学校".into(),
                    label: "学校".into(),
                    width_mm: 0.0,
                    align: "left".into(),
                    summary: String::new(),
                },
                TableColumn {
                    field: "姓名".into(),
                    label: "姓名".into(),
                    width_mm: 0.0,
                    align: "left".into(),
                    summary: String::new(),
                },
            ],
            row_height_mm: 8.0,
            header_height_mm: 8.0,
            ..Default::default()
        });
        let mut body = TemplateBody {
            page_field: "学校".into(),
            header_text: "{学校} 报名名单".into(),
            ..Default::default()
        };
        body.elements.push(table);
        let rows = vec![
            vec!["乙中学".into(), "张三".into()],
            vec!["甲中学".into(), "李四".into()],
            vec!["乙中学".into(), "王五".into()],
        ];
        let pdf = render_pdf_data(&cols, &rows, &body, "table", &RenderOpts::default())
            .expect("render grouped table");
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn table_summary_row_computes_sum_avg_and_count() {
        let setup = TableSetup {
            columns: vec![
                TableColumn {
                    field: "姓名".into(),
                    label: "姓名".into(),
                    width_mm: 0.0,
                    align: "left".into(),
                    summary: String::new(),
                },
                TableColumn {
                    field: "金额".into(),
                    label: "金额".into(),
                    width_mm: 0.0,
                    align: "right".into(),
                    summary: "sum".into(),
                },
                TableColumn {
                    field: "成绩".into(),
                    label: "成绩".into(),
                    width_mm: 0.0,
                    align: "right".into(),
                    summary: "avg".into(),
                },
                TableColumn {
                    field: "人数".into(),
                    label: "人数".into(),
                    width_mm: 0.0,
                    align: "right".into(),
                    summary: "count".into(),
                },
            ],
            summary_label: "合计".into(),
            ..Default::default()
        };
        let index: HashMap<String, usize> = [
            ("姓名".to_string(), 0usize),
            ("金额".to_string(), 1),
            ("成绩".to_string(), 2),
            ("人数".to_string(), 3),
        ]
        .into_iter()
        .collect();
        let rows = vec![
            vec!["张三".into(), "100".into(), "80".into(), "1".into()],
            vec!["李四".into(), "200".into(), "90".into(), "".into()],
        ];
        let summary = table_summary_row(&setup, &rows, &index).expect("summary");
        assert_eq!(summary, vec!["合计", "300", "85", "1"]);
    }

    #[test]
    fn render_table_requires_columns() {
        let cols = vec![DatasetColumn {
            name: "姓名".into(),
            dtype: "text".into(),
        }];
        let mut table = TemplateElement::empty("tb", "table");
        table.w_mm = 100.0;
        table.h_mm = 50.0;
        table.table = Some(TableSetup::default());
        let mut body = TemplateBody::default();
        body.elements.push(table);
        let rows = vec![vec!["张三".into()]];
        let err = render_pdf_data(&cols, &rows, &body, "table", &RenderOpts::default())
            .expect_err("empty columns rejected");
        assert_eq!(err.to_string(), "明细表至少需要配置一列");
    }

    #[test]
    fn render_supports_page_number_offset_and_empty_hiding() {
        let cols = vec![DatasetColumn {
            name: "姓名".into(),
            dtype: "text".into(),
        }];
        let mut body = TemplateBody {
            show_page_number: true,
            page_number_start: 3,
            page_number_skip_first: true,
            ..Default::default()
        };
        let mut name = TemplateElement::empty("t", "text");
        name.text = "{姓名}".into();
        body.elements.push(name);
        let mut empty = TemplateElement::empty("e", "text");
        empty.text = "{姓名}".into();
        empty.hide_if_empty = true;
        body.elements.push(empty);
        let mut conditional = TemplateElement::empty("c", "text");
        conditional.text = "仅张三显示".into();
        conditional.visible_field = "姓名".into();
        conditional.visible_value = "张三".into();
        body.elements.push(conditional);
        // 第二行姓名为空，隐藏元素应整行跳过而不报错。
        let rows = vec![vec!["张三".into()], vec!["".into()]];
        let pdf =
            render_pdf_data(&cols, &rows, &body, "single", &RenderOpts::default()).expect("render");
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_paginates_matrix_by_room_and_seat() {
        let mut body = TemplateBody {
            page_field: "考场号".into(),
            matrix_index_field: "座位号".into(),
            ..Default::default()
        };
        let mut matrix = TemplateElement::empty("m", "matrix");
        matrix.x_mm = 10.0;
        matrix.y_mm = 10.0;
        matrix.w_mm = 120.0;
        matrix.h_mm = 100.0;
        matrix.matrix = Some(MatrixSetup {
            rows: 2,
            cols: 2,
            gap_x_mm: 0.0,
            gap_y_mm: 0.0,
            cell_h_mm: 30.0,
            origin_y_mm: 0.0,
            show_grid: true,
            show_border: true,
        });
        let mut text = TemplateElement::empty("t", "text");
        text.text = "{姓名}({考场号}-{座位号})".into();
        matrix.children.push(text);
        body.elements.push(matrix);
        body.normalize("matrix");
        let cols = vec![
            DatasetColumn {
                name: "考场号".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "座位号".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "姓名".into(),
                dtype: "text".into(),
            },
        ];
        let rows = vec![
            vec!["1".into(), "2".into(), "张三".into()],
            vec!["1".into(), "1".into(), "李四".into()],
            vec!["2".into(), "1".into(), "王五".into()],
        ];
        let bytes =
            render_pdf_data(&cols, &rows, &body, "matrix", &RenderOpts::default()).expect("render");
        assert!(bytes.starts_with(b"%PDF"));
        assert!(bytes.len() > 1000);
    }

    #[test]
    fn legacy_photo_element_is_migrated_to_name_template() {
        let mut body = TemplateBody::default();
        let mut el = TemplateElement::empty("e1", "image");
        el.field = "照片".to_string();
        body.elements.push(el);
        body.normalize("single");
        assert_eq!(body.elements[0].photo_name, "{照片}");
    }
}
