//! xlsx / xls / ods / csv 解析。csv 自动识别 UTF-8 与 GBK 编码。

use crate::error::{AppError, AppResult};
use calamine::{Data, Reader};
use std::path::Path;

pub struct Parsed {
    pub sheet_names: Vec<String>,
    pub sheet: String,
    #[allow(dead_code)]
    pub header_row: usize,
    /// 原始二维数据（不含表头行），已做尾部空列裁剪。
    pub rows: Vec<Vec<Option<String>>>,
}

const MAX_ROWS: usize = 200_000;

fn cell_to_string(cell: &Data) -> Option<String> {
    match cell {
        Data::Empty => None,
        Data::Int(i) => Some(i.to_string()),
        Data::Float(f) => Some(format_float(*f)),
        Data::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(s.clone())
            }
        }
        Data::Bool(b) => Some(if *b { "TRUE".into() } else { "FALSE".into() }),
        Data::DateTime(dt) => match dt.as_datetime() {
            Some(n) => {
                let is_midnight = n.time() == chrono::NaiveTime::MIN;
                Some(if is_midnight {
                    n.date().format("%Y-%m-%d").to_string()
                } else {
                    n.format("%Y-%m-%d %H:%M:%S").to_string()
                })
            }
            None => Some(format_float(dt.as_f64())),
        },
        Data::DateTimeIso(s) => Some(s.clone()),
        Data::DurationIso(s) => Some(s.clone()),
        Data::Error(_) => None,
    }
}

pub fn format_float(f: f64) -> String {
    if (f - f.round()).abs() < 1e-9 && f.abs() < 1e15 {
        format!("{}", f.round() as i64)
    } else {
        let s = format!("{f}");
        s
    }
}

fn is_xlsx_like(ext: &str) -> bool {
    matches!(ext, "xlsx" | "xlsm" | "xlam" | "xls" | "xla" | "xlsb" | "ods")
}

pub fn parse_file(path: &Path, ext: &str) -> AppResult<Parsed> {
    let ext = ext.to_ascii_lowercase();
    if ext == "csv" || ext == "txt" || ext == "tsv" {
        return parse_csv(path, ext == "tsv");
    }
    if !is_xlsx_like(&ext) {
        return Err(AppError::bad_request(format!("不支持的文件类型: .{ext}")));
    }
    parse_spreadsheet(path)
}

fn parse_spreadsheet(path: &Path) -> AppResult<Parsed> {
    let mut wb = calamine::open_workbook_auto(path)
        .map_err(|e| AppError::bad_request(format!("无法打开表格文件: {e}")))?;
    let sheet_names = wb.sheet_names().to_vec();
    if sheet_names.is_empty() {
        return Err(AppError::bad_request("文件中没有工作表"));
    }
    let sheet = sheet_names[0].clone();
    let range = wb
        .worksheet_range(&sheet)
        .map_err(|e| AppError::bad_request(format!("读取工作表失败: {e}")))?;

    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    for r in range.rows() {
        let vals: Vec<Option<String>> = r.iter().map(cell_to_string).collect();
        rows.push(vals);
    }
    Ok(Parsed { sheet_names, sheet, header_row: 1, rows })
}

fn parse_csv(path: &Path, is_tsv: bool) -> AppResult<Parsed> {
    let bytes = std::fs::read(path)?;
    let text = decode_text(&bytes);
    let delim = if is_tsv { b'\t' } else { b',' };
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| AppError::bad_request(format!("CSV 解析失败: {e}")))?;
        let vals: Vec<Option<String>> = rec
            .iter()
            .map(|v| {
                let t = v.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                }
            })
            .collect();
        rows.push(vals);
        if rows.len() > MAX_ROWS {
            break;
        }
    }
    Ok(Parsed { sheet_names: vec!["CSV".to_string()], sheet: "CSV".to_string(), header_row: 1, rows })
}

/// UTF-8 优先，失败则按 GBK 解码（国内导出的 CSV 大量是 GBK）。
fn decode_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).to_string();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::GBK.decode(bytes);
            cow.to_string()
        }
    }
}

/// 从二维原始数据中切出表头与数据体。
/// `header_row` 为 1-based 行号；返回 (表头, 数据行)。
pub fn split_header(
    rows: Vec<Vec<Option<String>>>,
    header_row: usize,
) -> (Vec<String>, Vec<Vec<Option<String>>>) {
    if rows.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let hr = header_row.max(1).min(rows.len());
    let header_raw = &rows[hr - 1];
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut header: Vec<String> = Vec::with_capacity(width);
    for i in 0..width {
        let v = header_raw.get(i).and_then(|x| x.clone()).unwrap_or_default();
        let t = v.trim().to_string();
        header.push(if t.is_empty() {
            format!("列{}", i + 1)
        } else {
            t
        });
    }
    let mut body: Vec<Vec<Option<String>>> = Vec::new();
    for r in rows.into_iter().skip(hr) {
        // 整行为空的跳过
        if r.iter().all(|c| c.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true)) {
            continue;
        }
        let mut vals: Vec<Option<String>> = Vec::with_capacity(width);
        for i in 0..width {
            vals.push(r.get(i).cloned().flatten());
        }
        body.push(vals);
    }
    (header, body)
}
