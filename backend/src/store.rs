//! 数据集存储层：所有表格数据都存放在 SQLite 表 `data_<id>` 中，
//! 物理列名固定为 c0、c1、c2...，显示名/类型存放在 dataset_columns。

use crate::db::{new_id, now, table_name};
use crate::error::{AppError, AppResult};
use crate::models::{Dataset, DatasetColumn, RowPage};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, Connection};

fn sql_type(dtype: &str) -> &'static str {
    match dtype {
        "number" => "REAL",
        _ => "TEXT",
    }
}

/// 列名去重：重名的追加 (2)、(3)...
pub fn unique_names(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(names.len());
    for raw in names {
        let base = {
            let t = raw.trim();
            if t.is_empty() {
                "未命名列".to_string()
            } else {
                t.to_string()
            }
        };
        if !out.contains(&base) {
            out.push(base);
            continue;
        }
        let mut n = 2;
        loop {
            let cand = format!("{base}({n})");
            if !out.contains(&cand) {
                out.push(cand);
                break;
            }
            n += 1;
        }
    }
    out
}

pub fn get_columns(conn: &Connection, dataset_id: &str) -> AppResult<Vec<DatasetColumn>> {
    let mut stmt = conn.prepare(
        "SELECT name, dtype FROM dataset_columns WHERE dataset_id = ?1 ORDER BY idx ASC",
    )?;
    let rows = stmt.query_map(params![dataset_id], |r| {
        Ok(DatasetColumn { name: r.get(0)?, dtype: r.get(1)? })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn set_columns(conn: &Connection, dataset_id: &str, cols: &[DatasetColumn]) -> AppResult<()> {
    conn.execute("DELETE FROM dataset_columns WHERE dataset_id = ?1", params![dataset_id])?;
    let mut stmt = conn.prepare(
        "INSERT INTO dataset_columns (dataset_id, idx, name, dtype) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (i, c) in cols.iter().enumerate() {
        stmt.execute(params![dataset_id, i as i64, c.name, c.dtype])?;
    }
    Ok(())
}

/// 按给定下标保留数据列，其余列连同原始数据一起从物理表中删除。
/// `keep_indices` 对应保留前 `cols` 的下标；`cols` 为保留后的列元数据。
pub fn retain_columns(
    conn: &Connection,
    dataset_id: &str,
    cols: &[DatasetColumn],
    keep_indices: &[usize],
) -> AppResult<()> {
    if keep_indices.is_empty() {
        return Err(AppError::bad_request("不能删除全部字段"));
    }
    let tname = table_name(dataset_id);
    let tmp = format!("{tname}_keep");
    conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{tmp}\";"))?;
    let col_defs: Vec<String> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| format!("\"c{i}\" {}", sql_type(&c.dtype)))
        .collect();
    conn.execute_batch(&format!("CREATE TABLE \"{tmp}\" ({});", col_defs.join(", ")))?;
    let dst: Vec<String> = (0..keep_indices.len()).map(|i| format!("\"c{i}\"")).collect();
    let src: Vec<String> = keep_indices.iter().map(|i| format!("\"c{i}\"")).collect();
    conn.execute(
        &format!(
            "INSERT INTO \"{tmp}\" ({}) SELECT {} FROM \"{tname}\"",
            dst.join(", "),
            src.join(", ")
        ),
        [],
    )?;
    conn.execute_batch(&format!(
        "DROP TABLE \"{tname}\"; ALTER TABLE \"{tmp}\" RENAME TO \"{tname}\";"
    ))?;
    Ok(())
}

pub fn get_dataset(conn: &Connection, id: &str) -> AppResult<Dataset> {
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, source_file, origin, project_id, row_count, created_at, updated_at
         FROM datasets WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    let row = rows
        .next()?
        .ok_or_else(|| AppError::not_found(format!("数据集不存在: {id}")))?;
    let mut ds = Dataset {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        source_file: row.get(3)?,
        origin: row.get(4)?,
        project_id: row.get(5)?,
        row_count: row.get(6)?,
        columns: Vec::new(),
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    };
    ds.columns = get_columns(conn, id)?;
    Ok(ds)
}

/// 列出数据集；`project_id` 为 Some 时仅返回该考试项目下的数据集。
pub fn list_datasets(conn: &Connection, project_id: Option<&str>) -> AppResult<Vec<Dataset>> {
    let ids: Vec<String> = if let Some(pid) = project_id {
        let mut stmt =
            conn.prepare("SELECT id FROM datasets WHERE project_id = ?1 ORDER BY created_at DESC")?;
        let collected = stmt
            .query_map(params![pid], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        collected
    } else {
        let mut stmt = conn.prepare("SELECT id FROM datasets ORDER BY created_at DESC")?;
        let collected = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        collected
    };
    let mut out = Vec::new();
    for id in ids {
        out.push(get_dataset(conn, &id)?);
    }
    Ok(out)
}

/// 创建数据集表（物理列 c0..cN）并批量写入数据。
pub fn create_dataset(
    conn: &Connection,
    name: &str,
    kind: &str,
    source_file: &str,
    origin: &str,
    project_id: &str,
    columns: &[DatasetColumn],
    rows: &[Vec<Option<String>>],
) -> AppResult<Dataset> {
    let id = new_id();
    let tname = table_name(&id);
    let col_defs: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(i, c)| format!("\"c{i}\" {}", sql_type(&c.dtype)))
        .collect();
    conn.execute_batch(&format!(
        "CREATE TABLE \"{tname}\" ({});",
        col_defs.join(", ")
    ))?;

    if !rows.is_empty() && !columns.is_empty() {
        let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "INSERT INTO \"{tname}\" ({}) VALUES ({})",
            (0..columns.len())
                .map(|i| format!("\"c{i}\""))
                .collect::<Vec<_>>()
                .join(", "),
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        for row in rows {
            let vals: Vec<SqlValue> = (0..columns.len())
                .map(|i| match row.get(i).and_then(|v| v.clone()) {
                    Some(s) if !s.is_empty() => SqlValue::Text(s),
                    _ => SqlValue::Null,
                })
                .collect();
            stmt.execute(rusqlite::params_from_iter(vals.iter()))?;
        }
    }

    let ts = now();
    conn.execute(
        "INSERT INTO datasets (id, name, kind, source_file, origin, project_id, row_count, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![id, name, kind, source_file, origin, project_id, rows.len() as i64, ts],
    )?;
    set_columns(conn, &id, columns)?;
    get_dataset(conn, &id)
}

/// 由一条 SELECT 语句（流程编译结果）落地成一个新的数据集。
pub fn create_dataset_from_select(
    conn: &Connection,
    name: &str,
    kind: &str,
    origin: &str,
    project_id: &str,
    columns: &[DatasetColumn],
    select_sql: &str,
) -> AppResult<Dataset> {
    let id = new_id();
    let tname = table_name(&id);
    let col_defs: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(i, c)| format!("\"c{i}\" {}", sql_type(&c.dtype)))
        .collect();
    conn.execute_batch(&format!(
        "CREATE TABLE \"{tname}\" ({});",
        col_defs.join(", ")
    ))?;
    let dest_cols: Vec<String> = (0..columns.len()).map(|i| format!("\"c{i}\"")).collect();
    let sql = format!(
        "INSERT INTO \"{tname}\" ({}) {select_sql}",
        dest_cols.join(", ")
    );
    conn.execute(&sql, []).map_err(|e| {
        AppError::bad_request(format!("流程结果写入失败: {e}"))
    })?;
    let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM \"{tname}\""), [], |r| r.get(0))?;

    let ts = now();
    conn.execute(
        "INSERT INTO datasets (id, name, kind, source_file, origin, project_id, row_count, created_at, updated_at)
         VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, ?7, ?7)",
        params![id, name, kind, origin, project_id, n, ts],
    )?;
    set_columns(conn, &id, columns)?;
    get_dataset(conn, &id)
}

pub fn delete_dataset(conn: &Connection, id: &str) -> AppResult<()> {
    let tname = table_name(id);
    conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{tname}\";"))?;
    conn.execute("DELETE FROM dataset_columns WHERE dataset_id = ?1", params![id])?;
    conn.execute("DELETE FROM datasets WHERE id = ?1", params![id])?;
    Ok(())
}
pub fn count_rows(conn: &Connection, id: &str) -> AppResult<i64> {
    let tname = table_name(id);
    let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM \"{tname}\""), [], |r| r.get(0))?;
    Ok(n)
}

fn json_value(v: rusqlite::types::ValueRef<'_>) -> serde_json::Value {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(i) => serde_json::Value::from(i),
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        ValueRef::Text(t) => serde_json::Value::String(String::from_utf8_lossy(t).to_string()),
        ValueRef::Blob(_) => serde_json::Value::String("<binary>".to_string()),
    }
}

/// 读取数据集分页内容。
pub fn read_rows(
    conn: &Connection,
    id: &str,
    offset: i64,
    limit: i64,
) -> AppResult<RowPage> {
    let columns = get_columns(conn, id)?;
    let total = count_rows(conn, id)?;
    let tname = table_name(id);
    let mut stmt = conn.prepare(&format!(
        "SELECT * FROM \"{tname}\" LIMIT ?1 OFFSET ?2"
    ))?;
    let ncol = columns.len();
    let mut out: Vec<Vec<serde_json::Value>> = Vec::new();
    let mut rows = stmt.query(params![limit, offset])?;
    while let Some(row) = rows.next()? {
        let mut vals = Vec::with_capacity(ncol);
        let actual = row.as_ref().column_count();
        for i in 0..ncol {
            if i < actual {
                vals.push(json_value(row.get_ref(i)?));
            } else {
                vals.push(serde_json::Value::Null);
            }
        }
        out.push(vals);
    }
    Ok(RowPage { columns, rows: out, total })
}

/// 读取全部数据（用于 PDF 生成），返回 (列, 行) —— 行内均为字符串。
pub fn read_all_strings(
    conn: &Connection,
    id: &str,
    limit: Option<i64>,
) -> AppResult<(Vec<DatasetColumn>, Vec<Vec<String>>)> {
    let columns = get_columns(conn, id)?;
    let tname = table_name(id);
    let lim = limit.unwrap_or(100_000).max(1);
    let mut stmt = conn.prepare(&format!("SELECT * FROM \"{tname}\" LIMIT {lim}"))?;
    let ncol = columns.len();
    let mut out = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let actual = row.as_ref().column_count();
        let mut vals = Vec::with_capacity(ncol);
        for i in 0..ncol {
            let v = if i < actual {
                row.get_ref(i)?
            } else {
                rusqlite::types::ValueRef::Null
            };
            vals.push(match v {
                rusqlite::types::ValueRef::Null => String::new(),
                rusqlite::types::ValueRef::Integer(i) => i.to_string(),
                rusqlite::types::ValueRef::Real(f) => {
                    if (f - f.round()).abs() < 1e-9 {
                        format!("{}", f.round() as i64)
                    } else {
                        format!("{f}")
                    }
                }
                rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).to_string(),
                rusqlite::types::ValueRef::Blob(_) => String::new(),
            });
        }
        out.push(vals);
    }
    Ok((columns, out))
}
