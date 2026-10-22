//! xlsx / xls / ods / csv 解析。csv 自动识别 UTF-8 与 GBK 编码。

use crate::error::{AppError, AppResult};
use calamine::{Data, Reader};
use std::path::Path;

/// 原始二维数据：外层是行，内层是单元格。
pub type RawRows = Vec<Vec<Option<String>>>;

#[derive(Debug)]
pub struct Parsed {
    /// 原始二维数据（含表头行），已做尾部空列裁剪。
    pub rows: RawRows,
}

const MAX_ROWS: usize = 200_000;

/// 超过上限时明确报错，绝不静默截断数据 —— 数据导入少几万行比失败更危险。
fn too_many_rows() -> AppError {
    AppError::validation(format!("表格行数超过 {MAX_ROWS} 行上限，请拆分后再导入"))
}

/// 去掉所有行末尾的空列，避免 Excel 里残留的格式列被当成真实字段。
fn trim_trailing_empty(mut rows: RawRows) -> RawRows {
    let width = rows
        .iter()
        .map(|r| {
            r.iter()
                .rposition(|c| c.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false))
                .map(|p| p + 1)
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(0);
    for r in rows.iter_mut() {
        r.truncate(width);
    }
    rows
}

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
    matches!(
        ext,
        "xlsx" | "xlsm" | "xlam" | "xls" | "xla" | "xlsb" | "ods"
    )
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
    let sheet = wb
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| AppError::bad_request("文件中没有工作表"))?;
    let range = wb
        .worksheet_range(&sheet)
        .map_err(|e| AppError::bad_request(format!("读取工作表失败: {e}")))?;

    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    for r in range.rows() {
        if rows.len() >= MAX_ROWS {
            return Err(too_many_rows());
        }
        let vals: Vec<Option<String>> = r.iter().map(cell_to_string).collect();
        rows.push(vals);
    }
    Ok(Parsed {
        rows: trim_trailing_empty(rows),
    })
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
        // 继续读只会得到更多行，直接停止并报错，避免无界内存占用。
        if rows.len() >= MAX_ROWS {
            return Err(too_many_rows());
        }
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
    }
    Ok(Parsed {
        rows: trim_trailing_empty(rows),
    })
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
pub fn split_header(rows: RawRows, header_row: usize) -> AppResult<(Vec<String>, RawRows)> {
    if rows.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    if header_row == 0 || header_row > rows.len() {
        return Err(AppError::validation(format!(
            "表头行号 {header_row} 超出数据范围（共 {} 行）",
            rows.len()
        )));
    }
    let hr = header_row;
    let header_raw = &rows[hr - 1];
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut header: Vec<String> = Vec::with_capacity(width);
    for i in 0..width {
        let v = header_raw
            .get(i)
            .and_then(|x| x.clone())
            .unwrap_or_default();
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
        if r.iter()
            .all(|c| c.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true))
        {
            continue;
        }
        let mut vals: Vec<Option<String>> = Vec::with_capacity(width);
        for i in 0..width {
            vals.push(r.get(i).cloned().flatten());
        }
        body.push(vals);
    }
    Ok((header, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("exam_imp_{}_{name}", crate::db::new_id()));
        std::fs::write(&path, bytes).expect("write temp file");
        path
    }

    #[test]
    fn parses_gbk_encoded_csv() {
        let (encoded, _, _) = encoding_rs::GBK.encode("姓名,成绩\n张三,90\n");
        let path = temp_file("gbk.csv", &encoded);
        let parsed = parse_file(&path, "csv").expect("parse gbk csv");
        let _ = std::fs::remove_file(&path);

        let (header, rows) = split_header(parsed.rows, 1).expect("split header");
        assert_eq!(header, vec!["姓名".to_string(), "成绩".to_string()]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].as_deref(), Some("张三"));
    }

    #[test]
    fn trims_trailing_empty_columns_and_skips_blank_rows() {
        let path = temp_file("blank.csv", "A,B,,\n1,2,,\n,,\n3,4,,\n".as_bytes());
        let parsed = parse_file(&path, "csv").expect("parse csv");
        let _ = std::fs::remove_file(&path);

        // Excel 里残留的空列不应变成字段
        assert!(parsed.rows.iter().all(|r| r.len() == 2));
        let (header, body) = split_header(parsed.rows, 1).expect("split header");
        assert_eq!(header, vec!["A".to_string(), "B".to_string()]);
        // 整行为空的行不会进入数据体
        assert_eq!(body.len(), 2);
    }

    #[test]
    fn header_row_out_of_range_is_rejected() {
        let rows = vec![vec![Some("A".to_string())], vec![Some("1".to_string())]];
        let err = split_header(rows, 99).expect_err("header row must be validated");
        assert_eq!(err.code, "validation_error");
    }

    #[test]
    fn empty_rows_yield_empty_header() {
        let (header, body) = split_header(Vec::new(), 1).expect("empty is ok");
        assert!(header.is_empty());
        assert!(body.is_empty());
    }

    #[test]
    fn unsupported_extension_is_rejected() {
        let path = temp_file("data.bin", b"whatever");
        let err = parse_file(&path, "bin").expect_err("unsupported ext");
        let _ = std::fs::remove_file(&path);
        assert_eq!(err.code, "bad_request");
    }
}
