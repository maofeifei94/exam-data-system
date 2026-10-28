//! 数据集存储层：所有表格数据都存放在 SQLite 表 `data_<id>` 中，
//! 物理列名固定为 c0、c1、c2...，显示名/类型存放在 dataset_columns。
//!
//! 事务约定：
//! - 涉及「建表 + 写数据 + 写元数据」的复合操作统一使用 `*_tx` 变体，
//!   由外层 `db::with_tx` 控制提交；对外暴露的 `&mut Connection` 版本会自动开事务。
//! - 事务函数内部绝不再次 `BEGIN`，避免嵌套事务报错。

use crate::db::{self, new_id, now, table_name};
use crate::error::{AppError, AppResult};
use crate::models::{ColumnSuggestion, Dataset, DatasetColumn, RowPage};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, Connection};
use std::collections::HashMap;

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
        Ok(DatasetColumn {
            name: r.get(0)?,
            dtype: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn set_columns(conn: &Connection, dataset_id: &str, cols: &[DatasetColumn]) -> AppResult<()> {
    conn.execute(
        "DELETE FROM dataset_columns WHERE dataset_id = ?1",
        params![dataset_id],
    )?;
    let mut stmt = conn.prepare(
        "INSERT INTO dataset_columns (dataset_id, idx, name, dtype) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (i, c) in cols.iter().enumerate() {
        stmt.execute(params![dataset_id, i as i64, c.name, c.dtype])?;
    }
    Ok(())
}

/// 按给定下标保留数据列，其余列连同原始数据一起从物理表中删除。
#[allow(dead_code)] // 便捷事务包装：路由层在更大的事务中直接调用 `_tx` 版本。
pub fn retain_columns(
    conn: &mut Connection,
    dataset_id: &str,
    cols: &[DatasetColumn],
    keep_indices: &[usize],
) -> AppResult<()> {
    db::with_tx(conn, |tx| {
        retain_columns_tx(tx, dataset_id, cols, keep_indices)
    })
}

/// `retain_columns` 的事务内版本：调用方负责提交/回滚。
pub fn retain_columns_tx(
    conn: &Connection,
    dataset_id: &str,
    cols: &[DatasetColumn],
    keep_indices: &[usize],
) -> AppResult<()> {
    if keep_indices.is_empty() {
        return Err(AppError::bad_request("不能删除全部字段"));
    }
    let tname = table_name(dataset_id)?;
    let tmp = format!("{tname}_keep");
    conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{tmp}\";"))?;
    let col_defs: Vec<String> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| format!("\"c{i}\" {}", sql_type(&c.dtype)))
        .collect();
    conn.execute_batch(&format!(
        "CREATE TABLE \"{tmp}\" ({});",
        col_defs.join(", ")
    ))?;
    let dst: Vec<String> = (0..keep_indices.len())
        .map(|i| format!("\"c{i}\""))
        .collect();
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

fn dataset_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Dataset> {
    Ok(Dataset {
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
    })
}

const DATASET_COLS: &str =
    "id, name, kind, source_file, origin, project_id, row_count, created_at, updated_at";

pub fn get_dataset(conn: &Connection, id: &str) -> AppResult<Dataset> {
    use rusqlite::OptionalExtension;
    let mut stmt = conn.prepare(&format!(
        "SELECT {DATASET_COLS} FROM datasets WHERE id = ?1"
    ))?;
    let mut ds = stmt
        .query_row(params![id], dataset_from_row)
        .optional()?
        .ok_or_else(|| AppError::not_found(format!("数据集不存在: {id}")))?;
    ds.columns = get_columns(conn, id)?;
    Ok(ds)
}

/// 列出数据集；`project_id` 为 Some 时仅返回该考试项目下的数据集。
///
/// 用一次 LEFT JOIN 取回数据集与列，避免「N 个数据集 → N+1 次查询」。
pub fn list_datasets(conn: &Connection, project_id: Option<&str>) -> AppResult<Vec<Dataset>> {
    let mut sql = String::from(
        "SELECT d.id, d.name, d.kind, d.source_file, d.origin, d.project_id,
                d.row_count, d.created_at, d.updated_at, c.name, c.dtype
         FROM datasets d
         LEFT JOIN dataset_columns c ON c.dataset_id = d.id",
    );
    let mut binds: Vec<String> = Vec::new();
    if let Some(pid) = project_id {
        sql.push_str(" WHERE d.project_id = ?1");
        binds.push(pid.to_string());
    }
    sql.push_str(" ORDER BY d.created_at DESC, c.idx ASC");

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(binds.iter()))?;
    let mut out: Vec<Dataset> = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        if out.last().map(|d| d.id != id).unwrap_or(true) {
            out.push(Dataset {
                id,
                name: row.get(1)?,
                kind: row.get(2)?,
                source_file: row.get(3)?,
                origin: row.get(4)?,
                project_id: row.get(5)?,
                row_count: row.get(6)?,
                columns: Vec::new(),
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            });
        }
        let col_name: Option<String> = row.get(9)?;
        if let (Some(name), Some(ds)) = (col_name, out.last_mut()) {
            ds.columns.push(DatasetColumn {
                name,
                dtype: row.get(10)?,
            });
        }
    }
    Ok(out)
}

/// 新建数据集的输入参数。
pub struct NewDataset<'a> {
    pub name: &'a str,
    pub kind: &'a str,
    pub source_file: &'a str,
    pub origin: &'a str,
    pub project_id: &'a str,
    pub columns: &'a [DatasetColumn],
    pub rows: &'a [Vec<Option<String>>],
}

/// 创建数据集表（物理列 c0..cN）并批量写入数据。
#[allow(dead_code)] // 便捷事务包装：路由层在更大的事务中直接调用 `_tx` 版本。
pub fn create_dataset(conn: &mut Connection, input: NewDataset<'_>) -> AppResult<Dataset> {
    db::with_tx(conn, |tx| create_dataset_tx(tx, input))
}

/// `create_dataset` 的事务内版本。
pub fn create_dataset_tx(conn: &Connection, input: NewDataset<'_>) -> AppResult<Dataset> {
    let NewDataset {
        name,
        kind,
        source_file,
        origin,
        project_id,
        columns,
        rows,
    } = input;
    let id = new_id();
    let tname = table_name(&id)?;
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
#[allow(dead_code)] // 便捷事务包装：流程引擎在更大的事务中直接调用 `_tx` 版本。
pub fn create_dataset_from_select(
    conn: &mut Connection,
    name: &str,
    kind: &str,
    origin: &str,
    project_id: &str,
    columns: &[DatasetColumn],
    select_sql: &str,
) -> AppResult<Dataset> {
    db::with_tx(conn, |tx| {
        create_dataset_from_select_tx(tx, name, kind, origin, project_id, columns, select_sql)
    })
}

/// `create_dataset_from_select` 的事务内版本。
pub fn create_dataset_from_select_tx(
    conn: &Connection,
    name: &str,
    kind: &str,
    origin: &str,
    project_id: &str,
    columns: &[DatasetColumn],
    select_sql: &str,
) -> AppResult<Dataset> {
    let id = new_id();
    let tname = table_name(&id)?;
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
    conn.execute(&sql, [])
        .map_err(|e| AppError::bad_request(format!("流程结果写入失败: {e}")))?;
    let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM \"{tname}\""), [], |r| {
        r.get(0)
    })?;

    let ts = now();
    conn.execute(
        "INSERT INTO datasets (id, name, kind, source_file, origin, project_id, row_count, created_at, updated_at)
         VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, ?7, ?7)",
        params![id, name, kind, origin, project_id, n, ts],
    )?;
    set_columns(conn, &id, columns)?;
    get_dataset(conn, &id)
}

/// 删除数据集：物理表、列元数据、数据集记录与关联运行记录必须一起成功或一起回滚。
pub fn delete_dataset(conn: &mut Connection, id: &str) -> AppResult<()> {
    let tname = table_name(id)?;
    db::with_tx(conn, |tx| {
        tx.execute_batch(&format!("DROP TABLE IF EXISTS \"{tname}\";"))?;
        tx.execute(
            "DELETE FROM dataset_columns WHERE dataset_id = ?1",
            params![id],
        )?;
        let n = tx.execute("DELETE FROM datasets WHERE id = ?1", params![id])?;
        tx.execute(
            "DELETE FROM flow_runs WHERE dataset_id = ?1 OR output_dataset_id = ?1",
            params![id],
        )?;
        if n == 0 {
            return Err(AppError::not_found(format!("数据集不存在: {id}")));
        }
        Ok(())
    })
}

pub fn count_rows(conn: &Connection, id: &str) -> AppResult<i64> {
    let tname = table_name(id)?;
    let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM \"{tname}\""), [], |r| {
        r.get(0)
    })?;
    Ok(n)
}

pub(crate) fn json_value(v: rusqlite::types::ValueRef<'_>) -> serde_json::Value {
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
pub fn read_rows(conn: &Connection, id: &str, offset: i64, limit: i64) -> AppResult<RowPage> {
    let columns = get_columns(conn, id)?;
    let total = count_rows(conn, id)?;
    let tname = table_name(id)?;
    let mut stmt = conn.prepare(&format!("SELECT * FROM \"{tname}\" LIMIT ?1 OFFSET ?2"))?;
    let ncol = columns.len();
    let mut out: Vec<Vec<serde_json::Value>> = Vec::new();
    let mut rows = stmt.query(params![limit.max(0), offset.max(0)])?;
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
    Ok(RowPage {
        columns,
        rows: out,
        total,
    })
}

/// 读取全部数据（用于 PDF 生成），返回 (列, 行) —— 行内均为字符串。
pub fn read_all_strings(
    conn: &Connection,
    id: &str,
    limit: Option<i64>,
) -> AppResult<(Vec<DatasetColumn>, Vec<Vec<String>>)> {
    let columns = get_columns(conn, id)?;
    let tname = table_name(id)?;
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

/// 推断导入列的映射建议。
///
/// - `project_id` 为空：使用全局标准字段字典；
/// - 传入项目 id：只使用该项目的 `project_fields`，全局字段必须先复制到项目才会被匹配，
///   避免项目数据集隐式引用全局字段。
pub fn suggest_columns(
    conn: &Connection,
    columns: &[String],
    project_id: Option<&str>,
) -> AppResult<Vec<ColumnSuggestion>> {
    let standards: Vec<String> = match project_id.filter(|id| !id.is_empty()) {
        Some(id) => {
            let mut stmt = conn.prepare("SELECT name FROM project_fields WHERE project_id = ?1")?;
            let rows = stmt.query_map(params![id], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        }
        None => {
            let mut stmt = conn.prepare("SELECT name FROM standard_fields")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        }
    };

    // 别名只在其目标标准名存在于当前作用域时才生效，防止把列映射到项目里没有的字段。
    let alias_map: HashMap<String, String> = match project_id.filter(|id| !id.is_empty()) {
        Some(id) => {
            let mut stmt = conn.prepare(
                "SELECT alias, standard FROM project_alias_mappings WHERE project_id = ?1",
            )?;
            let rows = stmt.query_map(params![id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|(_, standard)| standards.iter().any(|name| name == standard))
                .collect()
        }
        None => {
            let mut stmt = conn.prepare("SELECT alias, standard FROM alias_mappings")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            rows.collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|(_, standard)| standards.iter().any(|name| name == standard))
                .collect()
        }
    };

    Ok(columns
        .iter()
        .map(|c| {
            if standards.iter().any(|s| s == c) {
                ColumnSuggestion {
                    source: c.clone(),
                    standard: Some(c.clone()),
                    matched_by: "exact_standard".into(),
                }
            } else if let Some(std) = alias_map.get(c) {
                ColumnSuggestion {
                    source: c.clone(),
                    standard: Some(std.clone()),
                    matched_by: "alias".into(),
                }
            } else {
                ColumnSuggestion {
                    source: c.clone(),
                    standard: None,
                    matched_by: "none".into(),
                }
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(db::SCHEMA).expect("schema");
        conn
    }

    fn columns(names: &[&str]) -> Vec<DatasetColumn> {
        names
            .iter()
            .map(|n| DatasetColumn {
                name: (*n).to_string(),
                dtype: "text".to_string(),
            })
            .collect()
    }

    fn dataset_input<'a>(
        name: &'a str,
        columns: &'a [DatasetColumn],
        rows: &'a [Vec<Option<String>>],
    ) -> NewDataset<'a> {
        NewDataset {
            name,
            kind: "import",
            source_file: "",
            origin: "单元测试",
            project_id: "",
            columns,
            rows,
        }
    }

    #[test]
    fn table_name_rejects_sql_injection_payload() {
        let err = table_name("x\" OR 1=1 --").expect_err("payload must be rejected");
        assert_eq!(err.code, "bad_request");
        assert!(table_name("abc123").is_ok());
    }

    #[test]
    fn create_read_and_delete_dataset() {
        let mut conn = memory_db();
        let cols = columns(&["姓名", "成绩"]);
        let rows = vec![
            vec![Some("张三".to_string()), Some("90".to_string())],
            vec![Some("李四".to_string()), Some("80".to_string())],
        ];
        let ds = create_dataset(&mut conn, dataset_input("测试", &cols, &rows)).expect("create");
        assert_eq!(ds.row_count, 2);
        assert_eq!(ds.columns.len(), 2);

        let page = read_rows(&conn, &ds.id, 0, 10).expect("read");
        assert_eq!(page.total, 2);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0][0], serde_json::json!("张三"));

        let listed = list_datasets(&conn, None).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].columns.len(), 2);

        delete_dataset(&mut conn, &ds.id).expect("delete");
        assert!(get_dataset(&conn, &ds.id).is_err());
        assert!(listed_persisted(&conn) == 0);
    }

    fn listed_persisted(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM datasets", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn failed_transaction_rolls_back_all_writes() {
        let mut conn = memory_db();
        let err = db::with_tx(&mut conn, |tx| -> AppResult<()> {
            let cols = columns(&["A"]);
            create_dataset_tx(tx, dataset_input("回滚", &cols, &[]))?;
            Err(AppError::bad_request("故意失败"))
        })
        .expect_err("must fail");
        assert_eq!(err.code, "bad_request");
        // 数据集元数据与物理表都必须回滚干净
        assert_eq!(listed_persisted(&conn), 0);
        // 只统计真实数据表 data_<id>；下划线需转义，否则会匹配到 datasets 元数据表。
        let leftovers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name LIKE 'data\\_%' ESCAPE '\\'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn unique_names_fills_blank_and_deduplicates() {
        let names: Vec<String> = ["姓名", "", " 姓名 ", "成绩", "成绩"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            unique_names(&names),
            vec!["姓名", "未命名列", "姓名(2)", "成绩", "成绩(2)"]
        );
    }

    #[test]
    fn suggest_columns_distinguishes_exact_alias_and_unknown() {
        let conn = memory_db();
        conn.execute(
            "INSERT INTO standard_fields (id, name, label) VALUES ('s1', '成绩', '成绩')",
            [],
        )
        .expect("standard");
        conn.execute(
            "INSERT INTO alias_mappings (id, alias, standard, updated_at)
             VALUES ('a1', '分数', '成绩', '2026-01-01 00:00:00')",
            [],
        )
        .expect("alias");

        let names = vec!["成绩".to_string(), "分数".to_string(), "备注".to_string()];
        let got = suggest_columns(&conn, &names, None).expect("suggest");

        assert_eq!(got[0].matched_by, "exact_standard");
        assert_eq!(got[1].matched_by, "alias");
        assert_eq!(got[1].standard.as_deref(), Some("成绩"));
        assert_eq!(got[2].matched_by, "none");
        assert!(got[2].standard.is_none());
    }

    #[test]
    fn suggest_columns_uses_only_project_fields_when_scoped() {
        let conn = memory_db();
        conn.execute(
            "INSERT INTO standard_fields (id, name, label) VALUES ('s1', '成绩', '成绩')",
            [],
        )
        .expect("standard");
        conn.execute(
            "INSERT INTO alias_mappings (id, alias, standard, updated_at)
             VALUES ('a1', '分数', '成绩', '2026-01-01 00:00:00')",
            [],
        )
        .expect("alias");
        // 项目只复制了「姓名」，没有「成绩」。
        conn.execute(
            "INSERT INTO project_fields
               (id, project_id, name, label, created_at, updated_at)
             VALUES ('p1', 'proj1', '姓名', '姓名', 'now', 'now')",
            [],
        )
        .expect("project field");

        // 项目自己的别名（复制全局字段时带过来的）应当生效。
        conn.execute(
            "INSERT INTO project_alias_mappings (id, project_id, alias, standard, updated_at)
             VALUES ('pa1', 'proj1', '名字', '姓名', 'now')",
            [],
        )
        .expect("project alias");

        let names = vec![
            "成绩".to_string(),
            "分数".to_string(),
            "姓名".to_string(),
            "名字".to_string(),
        ];
        let got = suggest_columns(&conn, &names, Some("proj1")).expect("suggest");

        // 全局字段与全局别名都不应穿透到项目作用域。
        assert_eq!(got[0].matched_by, "none");
        assert!(got[0].standard.is_none());
        assert_eq!(got[1].matched_by, "none");
        assert!(got[1].standard.is_none());
        assert_eq!(got[2].matched_by, "exact_standard");
        // 项目别名可以命中本项目字段。
        assert_eq!(got[3].matched_by, "alias");
        assert_eq!(got[3].standard.as_deref(), Some("姓名"));
    }

    #[test]
    fn read_rows_clamps_negative_paging_and_handles_offset_past_end() {
        let mut conn = memory_db();
        let cols = columns(&["A"]);
        let rows = vec![vec![Some("1".to_string())], vec![Some("2".to_string())]];
        let ds = create_dataset(&mut conn, dataset_input("分页", &cols, &rows)).expect("create");

        // 偏移超过总行数：返回空页，但 total 仍然是完整行数。
        let empty = read_rows(&conn, &ds.id, 10, -5).expect("read tail");
        assert_eq!(empty.total, 2);
        assert!(empty.rows.is_empty());

        // 负偏移夹到 0，返回第一行。
        let first = read_rows(&conn, &ds.id, -3, 1).expect("read first");
        assert_eq!(first.rows.len(), 1);
        assert_eq!(first.rows[0][0], serde_json::json!("1"));
    }

    #[test]
    fn delete_missing_dataset_returns_not_found() {
        let mut conn = memory_db();
        let err = delete_dataset(&mut conn, "deadbeef").expect_err("must fail");
        assert_eq!(err.code, "not_found");
    }

    #[test]
    fn retain_columns_keeps_selected_columns() {
        let mut conn = memory_db();
        let cols = columns(&["A", "B", "C"]);
        let rows = vec![vec![
            Some("1".to_string()),
            Some("2".to_string()),
            Some("3".to_string()),
        ]];
        let ds = create_dataset(&mut conn, dataset_input("裁剪", &cols, &rows)).expect("create");
        let kept = vec![cols[0].clone(), cols[2].clone()];
        retain_columns(&mut conn, &ds.id, &kept, &[0, 2]).expect("retain");
        set_columns(&conn, &ds.id, &kept).expect("set columns");

        let page = read_rows(&conn, &ds.id, 0, 10).expect("read");
        assert_eq!(page.columns.len(), 2);
        assert_eq!(page.rows[0][0], serde_json::json!("1"));
        assert_eq!(page.rows[0][1], serde_json::json!("3"));
    }
}
