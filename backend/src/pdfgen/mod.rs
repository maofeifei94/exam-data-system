//! PDF 生成：把「可编辑模板 + 数据行」渲染成 PDF。
//!
//! 模板元素坐标系：以内容区（页面去掉页边距后的区域）左上角为原点，单位毫米。
//! - single 模式：一行数据一页，元素相对页面内容区。
//! - matrix 模式：网格铺满页面，元素相对单元格左上角；
//!   `page_level = true` 的元素相对页面内容区，每页只画一次（用于标题）。
//! 占位符：`{字段名}` 取该行字段值，`{{index}}` 本页序号，`{{no}}` 全局行号，
//! `{{page}}` / `{{pages}}` / `{{count}}`。

use crate::error::{AppError, AppResult};
use crate::models::{MatrixSetup, PageSetup, TemplateBody, TemplateElement};
use crate::store;
use crate::util;
use printpdf::path::PaintMode;
use printpdf::*;
use rusqlite::Connection;
use std::collections::HashMap;

const FONT_BYTES: &[u8] = include_bytes!("../../assets/NotoSansSC.ttf");
const MM_PER_PT: f32 = 0.352778;
const PT_PER_MM: f32 = 2.834646;
const IMAGE_DPI: f32 = 72.0;

#[derive(Debug, Default, Clone)]
pub struct RenderOpts {
    pub limit: Option<i64>,
    pub photo_dir: Option<String>,
    pub only_page: Option<i64>,
}

struct Ctx<'a> {
    row: &'a [String],
    index: &'a HashMap<String, usize>,
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
        match key.trim() {
            "index" => self.index_no.to_string(),
            "no" => self.no.to_string(),
            "page" => self.page.to_string(),
            "pages" => self.pages.to_string(),
            "count" => self.count.to_string(),
            other => self.value(other).to_string(),
        }
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

fn text_width_mm(s: &str, size_pt: f32) -> f32 {
    let mut units = 0.0f32;
    for ch in s.chars() {
        let c = ch as u32;
        units += if c >= 0x2E80 {
            1.0
        } else if c < 0x80 {
            0.53
        } else {
            0.85
        };
    }
    units * size_pt * MM_PER_PT
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
    let tw = text_width_mm(text, el.font_size);
    let x = match el.align.as_str() {
        "center" => x0 + (w - tw) / 2.0,
        "right" => x0 + w - tw,
        _ => x0,
    };
    let top_offset = ((el.h_mm - fs_mm) / 2.0).max(0.0);
    let baseline = page_h - (origin.y_top + el.y_mm + top_offset + fs_mm * 0.80);
    layer.set_fill_color(color_of(&el.color).unwrap_or(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None))));
    layer.use_text(text, el.font_size, Mm(x), Mm(baseline), font);
    if el.bold {
        // 没有独立粗体字体，用微小偏移重绘模拟加粗
        layer.use_text(text, el.font_size, Mm(x + 0.18), Mm(baseline), font);
    }
}

fn draw_rect(
    layer: &PdfLayerReference,
    el: &TemplateElement,
    origin: &Origin,
    page_h: f32,
) {
    let llx = origin.x + el.x_mm;
    let ury = page_h - (origin.y_top + el.y_mm);
    let lly = ury - el.h_mm;
    let urx = llx + el.w_mm;
    let has_fill = !el.fill.trim().is_empty();
    if has_fill {
        layer.set_fill_color(color_of(&el.fill).unwrap());
    }
    if el.border {
        layer.set_outline_color(color_of(&el.color).unwrap_or(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None))));
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
                let (pw, ph) = (img.width() as f32, img.height() as f32);
                if pw > 0.0 && ph > 0.0 {
                    let xobj = ImageXObject::from_dynamic_image(&img);
                    let scale_x = (el.w_mm * PT_PER_MM) / pw;
                    let scale_y = (el.h_mm * PT_PER_MM) / ph;
                    Image::from(xobj).add_to_layer(
                        layer.clone(),
                        ImageTransform {
                            translate_x: Some(Mm(llx)),
                            translate_y: Some(Mm(lly)),
                            rotate: None,
                            scale_x: Some(scale_x),
                            scale_y: Some(scale_y),
                            dpi: Some(IMAGE_DPI),
                        },
                    );
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
        placeholder.fill = if el.fill.trim().is_empty() { "#F2F2F2".to_string() } else { el.fill.clone() };
        draw_rect(layer, &placeholder, origin, page_h);
    }
}

fn draw_matrix_frame(
    layer: &PdfLayerReference,
    mx: &MatrixSetup,
    origin: &Origin,
    w: f32,
    h: f32,
    cols: i64,
    rows: i64,
    page_h: f32,
) {
    let cell_w = w / cols as f32;
    let cell_h = h / rows as f32;
    // 边框：整个矩阵的外轮廓
    if mx.show_border {
        let llx = origin.x;
        let ury = page_h - origin.y_top;
        let lly = ury - h;
        let urx = llx + w;
        layer.set_outline_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
        layer.set_outline_thickness(0.35);
        layer.add_rect(Rect::new(Mm(llx), Mm(lly), Mm(urx), Mm(ury)).with_mode(PaintMode::Stroke));
    }
    // 网格：仅内部的分隔线，单元格之间无间隔
    if mx.show_grid {
        layer.set_outline_color(Color::Rgb(Rgb::new(0.45, 0.45, 0.45, None)));
        layer.set_outline_thickness(0.22);
        for i in 1..cols {
            let x = origin.x + cell_w * i as f32;
            layer.add_line(Line {
                points: vec![
                    (Point::new(Mm(x), Mm(page_h - origin.y_top)), false),
                    (Point::new(Mm(x), Mm(page_h - origin.y_top - h)), false),
                ],
                is_closed: false,
            });
        }
        for j in 1..rows {
            let y = page_h - (origin.y_top + cell_h * j as f32);
            layer.add_line(Line {
                points: vec![
                    (Point::new(Mm(origin.x), Mm(y)), false),
                    (Point::new(Mm(origin.x + w), Mm(y)), false),
                ],
                is_closed: false,
            });
        }
    }
}

fn draw_element(
    doc: &PdfDocumentReference,
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    el: &TemplateElement,
    ctx: &Ctx,
    origin: &Origin,
    page_h: f32,
    photo_dir: Option<&str>,
) {
    if el.etype == "matrix" {
        let mx = el.matrix.clone().unwrap_or_default();
        let cols = mx.cols.max(1);
        let rows = mx.rows.max(1);
        let matrix_origin = Origin { x: origin.x + el.x_mm, y_top: origin.y_top + el.y_mm };
        draw_matrix_frame(layer, &mx, &matrix_origin, el.w_mm, el.h_mm, cols, rows, page_h);
        let cell_w = el.w_mm / cols as f32;
        let cell_h = el.h_mm / rows as f32;
        for r in 0..rows {
            for c in 0..cols {
                let cell_origin = Origin {
                    x: matrix_origin.x + c as f32 * cell_w,
                    y_top: matrix_origin.y_top + r as f32 * cell_h,
                };
                draw_elements(doc, layer, font, &el.children, ctx, &cell_origin, page_h, photo_dir);
            }
        }
        return;
    }

    match el.etype.as_str() {
        "rect" => draw_rect(layer, el, origin, page_h),
        "line" => {
            layer.set_outline_color(color_of(&el.color).unwrap_or(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None))));
            layer.set_outline_thickness(0.4);
            let y = page_h - (origin.y_top + el.y_mm);
            layer.add_line(Line {
                points: vec![
                    (Point::new(Mm(origin.x + el.x_mm), Mm(y)), false),
                    (Point::new(Mm(origin.x + el.x_mm + el.w_mm), Mm(y)), false),
                ],
                is_closed: false,
            });
        }
        "image" => {
            let raw = ctx.value(&el.field);
            draw_image(doc, layer, el, origin, page_h, resolve_photo(&raw, photo_dir));
        }
        _ => {
            let text = if el.text.trim().is_empty() {
                ctx.value(&el.field).to_string()
            } else {
                interpolate(&el.text, ctx)
            };
            draw_text(layer, font, &text, el, origin, page_h);
        }
    }

    // 子元素相对父元素左上角继续绘制
    if !el.children.is_empty() {
        let child_origin = Origin { x: origin.x + el.x_mm, y_top: origin.y_top + el.y_mm };
        draw_elements(doc, layer, font, &el.children, ctx, &child_origin, page_h, photo_dir);
    }
}

fn draw_elements(
    doc: &PdfDocumentReference,
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    elements: &[TemplateElement],
    ctx: &Ctx,
    origin: &Origin,
    page_h: f32,
    photo_dir: Option<&str>,
) {
    for el in elements {
        draw_element(doc, layer, font, el, ctx, origin, page_h, photo_dir);
    }
}

fn resolve_photo(raw: &str, dir: Option<&str>) -> Option<std::path::PathBuf> {
    let v = raw.trim();
    if v.is_empty() {
        return None;
    }
    let p = std::path::PathBuf::from(v);
    if p.is_absolute() && p.exists() {
        return Some(p);
    }
    if let Some(d) = dir {
        let d = d.trim();
        if !d.is_empty() {
            let joined = std::path::Path::new(d).join(v);
            if joined.exists() {
                return Some(joined);
            }
        }
    }
    if p.exists() {
        return Some(p);
    }
    None
}

fn page_size_mm(p: &PageSetup) -> (f32, f32) {
    let (w, h) = match p.size.as_str() {
        "A3" => (297.0, 420.0),
        "A5" => (148.0, 210.0),
        "Letter" => (215.9, 279.4),
        _ => (210.0, 297.0),
    };
    if p.orientation == "landscape" {
        (h, w)
    } else {
        (w, h)
    }
}

pub fn render_pdf(
    conn: &Connection,
    body: &TemplateBody,
    kind: &str,
    dataset_id: &str,
    opts: &RenderOpts,
) -> AppResult<Vec<u8>> {
    let (col_meta, rows) = store::read_all_strings(conn, dataset_id, opts.limit)?;
    if col_meta.is_empty() {
        return Err(AppError::bad_request("数据集没有字段，无法生成 PDF"));
    }
    let index: HashMap<String, usize> = col_meta
        .iter()
        .enumerate()
        .map(|(i, c)| (c.name.clone(), i))
        .collect();

    let (page_w, page_h) = page_size_mm(&body.page);
    let m = &body.page.margin_mm;
    let content_x = m.left;
    let content_y_top = m.top;

    let (doc, first_page, first_layer) = PdfDocument::new("考务数据导出", Mm(page_w), Mm(page_h), "Layer 1");
    let font = doc
        .add_external_font(FONT_BYTES)
        .map_err(|e| AppError::internal(format!("加载中文字体失败: {e}")))?;

    let photo_dir = opts.photo_dir.clone();
    let total_rows = rows.len() as i64;

    if kind == "matrix" {
        let matrix_el = body.elements.iter().find(|e| e.etype == "matrix");
        let (cols, grid_rows) = matrix_el
            .and_then(|e| e.matrix.as_ref())
            .map(|m| (m.cols.max(1), m.rows.max(1)))
            .unwrap_or((1, 1));
        let per_page = cols * grid_rows;
        let pages = if total_rows == 0 {
            1
        } else {
            (total_rows + per_page - 1) / per_page
        };
        let start_page = opts.only_page.unwrap_or(1).max(1);
        let end_page = if opts.only_page.is_some() { start_page } else { pages };

        let mut first_used = false;
        for page in start_page..=end_page.min(pages) {
            let (pi, li) = if !first_used {
                first_used = true;
                (first_page, first_layer)
            } else {
                doc.add_page(Mm(page_w), Mm(page_h), format!("P{page}"))
            };
            let layer = doc.get_page(pi).get_layer(li);
            let first = (page - 1) * per_page;
            let on_page = (total_rows - first).clamp(0, per_page);

            let page_ctx = Ctx {
                row: &[],
                index: &index,
                index_no: 0,
                no: first,
                page,
                pages,
                count: on_page,
            };
            let page_origin = Origin { x: content_x, y_top: content_y_top };

            // 页面级元素：标题、标题框等（矩阵本身单独处理）
            for el in body.elements.iter().filter(|e| e.etype != "matrix") {
                draw_element(&doc, &layer, &font, el, &page_ctx, &page_origin, page_h, photo_dir.as_deref());
            }

            if let Some(el) = matrix_el {
                let mx = el.matrix.clone().unwrap_or_default();
                let matrix_origin = Origin { x: content_x + el.x_mm, y_top: content_y_top + el.y_mm };
                draw_matrix_frame(&layer, &mx, &matrix_origin, el.w_mm, el.h_mm, cols, grid_rows, page_h);
                let cell_w = el.w_mm / cols as f32;
                let cell_h = el.h_mm / grid_rows as f32;

                // 每条数据填入一个单元格，子元素被复制到所有格子
                for k in 0..on_page {
                    let row_no = first + k;
                    let row = &rows[row_no as usize];
                    let cx = k % cols;
                    let cy = k / cols;
                    let origin = Origin {
                        x: matrix_origin.x + cx as f32 * cell_w,
                        y_top: matrix_origin.y_top + cy as f32 * cell_h,
                    };
                    let ctx = Ctx {
                        row,
                        index: &index,
                        index_no: k + 1,
                        no: row_no + 1,
                        page,
                        pages,
                        count: on_page,
                    };
                    draw_elements(&doc, &layer, &font, &el.children, &ctx, &origin, page_h, photo_dir.as_deref());
                }
            }
        }
    } else {
        let pages = total_rows.max(if total_rows == 0 { 1 } else { 0 });
        let start_page = opts.only_page.unwrap_or(1).max(1);
        let end_page = if opts.only_page.is_some() { start_page } else { pages };
        let mut first_used = false;
        for page in start_page..=end_page.min(pages.max(1)) {
            let (pi, li) = if !first_used {
                first_used = true;
                (first_page, first_layer)
            } else {
                doc.add_page(Mm(page_w), Mm(page_h), format!("P{page}"))
            };
            let layer = doc.get_page(pi).get_layer(li);
            let row_no = page - 1;
            let row: &[String] = if (row_no as usize) < rows.len() {
                &rows[row_no as usize]
            } else {
                &[]
            };
            let ctx = Ctx {
                row,
                index: &index,
                index_no: page,
                no: page,
                page,
                pages,
                count: total_rows,
            };
            let origin = Origin { x: content_x, y_top: content_y_top };
            draw_elements(&doc, &layer, &font, &body.elements, &ctx, &origin, page_h, photo_dir.as_deref());
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
