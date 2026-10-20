//! 类型推断与通用小工具。

/// 名称中带这些片段的列一律按文本处理（身份证号、准考证号这类数字串
/// 如果存成 REAL 会丢精度，例如 18 位身份证号变成科学计数法）。
pub fn name_forces_text(name: &str) -> bool {
    const HINTS: &[&str] = &[
        "证号", "身份证", "考号", "编号", "编码", "账号", "手机", "电话", "银行卡", "工号", "学号",
    ];
    HINTS.iter().any(|h| name.contains(h))
}

pub fn looks_like_image(v: &str) -> bool {
    let lower = v.to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".bmp", ".webp", ".gif", ".tif", ".tiff"]
        .iter()
        .any(|e| lower.ends_with(e))
}

/// 安全数字：能被 f64 解析，且不会因 REAL 亲和性而丢失信息。
fn safe_number(v: &str) -> bool {
    let t = v.trim();
    if t.is_empty() || t.len() > 15 {
        return false;
    }
    if t.starts_with('0') && t.len() > 1 && !t.starts_with("0.") {
        return false;
    }
    t.parse::<f64>().is_ok()
}

fn looks_like_date(v: &str) -> bool {
    let t = v.trim();
    if t.len() < 8 || t.len() > 19 {
        return false;
    }
    chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d").is_ok()
        || chrono::NaiveDate::parse_from_str(t, "%Y/%m/%d").is_ok()
        || chrono::NaiveDate::parse_from_str(t, "%Y年%m月%d日").is_ok()
        || chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S").is_ok()
}

pub fn infer_dtype(name: &str, values: &[String]) -> String {
    if name_forces_text(name) {
        return "text".to_string();
    }
    let sample: Vec<&String> = values.iter().filter(|v| !v.trim().is_empty()).take(200).collect();
    if sample.is_empty() {
        return "text".to_string();
    }
    if sample.iter().all(|v| looks_like_image(v)) {
        return "image".to_string();
    }
    if sample.iter().all(|v| safe_number(v)) {
        return "number".to_string();
    }
    if sample.iter().all(|v| looks_like_date(v) || safe_number(v)) {
        return "date".to_string();
    }
    "text".to_string()
}

/// 在超长文本中截断，用于日志。
pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}...")
    }
}

/// 解析颜色 `#RRGGBB` 或 `#RGB`，失败返回 None。
pub fn parse_hex_color(s: &str) -> Option<(f32, f32, f32)> {
    let t = s.trim().trim_start_matches('#');
    let hex = match t.len() {
        6 => t.to_string(),
        3 => t.chars().flat_map(|c| [c, c]).collect(),
        _ => return None,
    };
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0))
}
