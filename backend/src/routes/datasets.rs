use crate::db;
use crate::error::{AppError, AppResult};
use crate::importer;
use crate::models::{
    ApplyMappingInput, Dataset, DatasetColumn, ImportResult, MappingPair, RowPage,
};
use crate::state::AppState;
use crate::store;
use crate::util;
use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{header, Response, StatusCode};
use axum::Json;
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectQuery {
    #[serde(default)]
    pub project_id: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ProjectQuery>,
) -> AppResult<Json<Vec<Dataset>>> {
    let pid = q.project_id.filter(|s| !s.trim().is_empty());
    let datasets = state
        .db(move |conn| store::list_datasets(conn, pid.as_deref()))
        .await?;
    Ok(Json(datasets))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<Dataset>> {
    let ds = state.db(move |conn| store::get_dataset(conn, &id)).await?;
    Ok(Json(ds))
}

pub async fn rows(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<PageQuery>,
) -> AppResult<Json<RowPage>> {
    let limit = q.limit.unwrap_or(100).clamp(1, 5000);
    let offset = q.offset.unwrap_or(0).max(0);
    let page = state
        .db(move |conn| store::read_rows(conn, &id, offset, limit))
        .await?;
    Ok(Json(page))
}

pub async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    state
        .db(move |conn| store::delete_dataset(conn, &id))
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn export_csv(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Response<Body>> {
    let (name, data) = state
        .db(move |conn| {
            let ds = store::get_dataset(conn, &id)?;
            let (cols, rows) = store::read_all_strings(conn, &id, None)?;
            let mut wtr = csv::Writer::from_writer(Vec::<u8>::new());
            wtr.write_record(cols.iter().map(|c| c.name.as_str()))
                .map_err(|e| AppError::internal_with_context("CSV 生成失败", e))?;
            for r in rows.iter() {
                wtr.write_record(r.iter().map(|s| s.as_str()))
                    .map_err(|e| AppError::internal_with_context("CSV 生成失败", e))?;
            }
            let body = wtr
                .into_inner()
                .map_err(|e| AppError::internal_with_context("CSV 生成失败", e))?;
            Ok((ds.name, body))
        })
        .await?;

    // 加 BOM，方便 Excel 直接双击打开
    let mut out = vec![0xEF, 0xBB, 0xBF];
    out.extend_from_slice(&data);
    let filename = format!("{}.csv", name);
    let encoded = util::urlencode(&filename);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"export.csv\"; filename*=UTF-8''{encoded}"),
        )
        .body(Body::from(out))
        .map_err(|e| AppError::internal_with_context("构建下载响应失败", e))
}

/// 解析完成、等待入库的导入数据。
struct PreparedUpload {
    name: String,
    filename: String,
    project_id: String,
    columns: Vec<DatasetColumn>,
    rows: Vec<Vec<Option<String>>>,
}

pub async fn import(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> AppResult<Json<ImportResult>> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut filename = String::new();
    let mut name: Option<String> = None;
    let mut header_row: i64 = 1;
    let mut project_id = String::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(format!("表单解析失败: {e}")))?
    {
        let fname = field.name().unwrap_or_default().to_string();
        match fname.as_str() {
            "file" => {
                filename = field.file_name().unwrap_or("upload").to_string();
                file_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| AppError::bad_request(format!("读取上传文件失败: {e}")))?
                        .to_vec(),
                );
            }
            "name" => name = Some(field.text().await.unwrap_or_default()),
            "header_row" => {
                let t = field.text().await.unwrap_or_default();
                header_row = t.trim().parse::<i64>().unwrap_or(1).max(1);
            }
            "project_id" => project_id = field.text().await.unwrap_or_default().trim().to_string(),
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let bytes = file_bytes.ok_or_else(|| AppError::bad_request("没有收到文件"))?;
    let ext = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext.is_empty() {
        return Err(AppError::bad_request("无法识别文件类型（缺少扩展名）"));
    }

    // 文件解析是 CPU/IO 密集操作，放到阻塞线程池，避免卡住 async 运行时。
    let prepared = state
        .blocking(move || parse_upload(bytes, filename, ext, name, header_row, project_id))
        .await?;

    let result = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                let dataset = store::create_dataset_tx(
                    tx,
                    store::NewDataset {
                        name: &prepared.name,
                        kind: "import",
                        source_file: &prepared.filename,
                        origin: "文件导入",
                        project_id: &prepared.project_id,
                        columns: &prepared.columns,
                        rows: &prepared.rows,
                    },
                )?;
                let names: Vec<String> = prepared.columns.iter().map(|c| c.name.clone()).collect();
                let mapping = store::suggest_columns(tx, &names)?;
                let preview = store::read_rows(tx, &dataset.id, 0, 20)?;
                Ok(ImportResult {
                    dataset,
                    mapping,
                    preview,
                })
            })
        })
        .await?;
    Ok(Json(result))
}

fn parse_upload(
    bytes: Vec<u8>,
    filename: String,
    ext: String,
    name: Option<String>,
    header_row: i64,
    project_id: String,
) -> AppResult<PreparedUpload> {
    let tmp = std::env::temp_dir().join(format!("exam_up_{}.{}", db::new_id(), ext));
    std::fs::write(&tmp, &bytes)
        .map_err(|e| AppError::internal_with_context("写入临时文件失败", e))?;
    let parsed = importer::parse_file(&tmp, &ext);
    // 无论解析成功与否都要清理临时文件。
    let _ = std::fs::remove_file(&tmp);
    let parsed = parsed?;

    let (header, body) = importer::split_header(parsed.rows, header_row as usize)?;
    if header.is_empty() {
        return Err(AppError::bad_request("表格为空或没有表头"));
    }
    let display_names = store::unique_names(&header);
    let columns: Vec<DatasetColumn> = display_names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let col_vals: Vec<String> = body
                .iter()
                .take(300)
                .map(|r| r.get(i).cloned().flatten().unwrap_or_default())
                .collect();
            DatasetColumn {
                name: n.clone(),
                dtype: util::infer_dtype(n, &col_vals),
            }
        })
        .collect();

    let ds_name = {
        let n = name.unwrap_or_default();
        let t = n.trim();
        if t.is_empty() {
            std::path::Path::new(&filename)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("导入数据")
                .to_string()
        } else {
            t.to_string()
        }
    };

    Ok(PreparedUpload {
        name: ds_name,
        filename,
        project_id,
        columns,
        rows: body,
    })
}

pub async fn apply_mapping(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<ApplyMappingInput>,
) -> AppResult<Json<Dataset>> {
    let dataset = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                let mut columns = store::get_columns(tx, &id)?;
                let mut pairs: Vec<MappingPair> = Vec::new();
                for m in input.mappings.iter() {
                    let from = m.from.trim().to_string();
                    let to = m.to.trim().to_string();
                    if from.is_empty() || to.is_empty() || from == to {
                        continue;
                    }
                    if !columns.iter().any(|c| c.name == from) {
                        return Err(AppError::bad_request(format!("字段 [{from}] 不存在")));
                    }
                    pairs.push(MappingPair { from, to });
                }
                for p in &pairs {
                    if let Some(c) = columns.iter_mut().find(|c| c.name == p.from) {
                        c.name = p.to.clone();
                    }
                }
                let names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
                if store::unique_names(&names) != names {
                    return Err(AppError::bad_request("映射后会产成重名字段，请检查"));
                }

                // 数据导入约定：只保留流程「必要字段校验」声明的字段，其余原始列连同数据删除。
                let keep_fields: Vec<String> = input
                    .keep_fields
                    .iter()
                    .map(|f| f.trim().to_string())
                    .filter(|f| !f.is_empty())
                    .collect();
                if !keep_fields.is_empty() {
                    let missing: Vec<String> = keep_fields
                        .iter()
                        .filter(|f| !columns.iter().any(|c| &c.name == *f))
                        .cloned()
                        .collect();
                    if !missing.is_empty() {
                        return Err(AppError::bad_request(format!(
                            "必要字段在导入数据中不存在：{}",
                            missing.join("、")
                        )));
                    }
                    let mut kept: Vec<DatasetColumn> = Vec::new();
                    let mut keep_indices: Vec<usize> = Vec::new();
                    for (idx, column) in columns.iter().enumerate() {
                        if keep_fields.contains(&column.name) {
                            keep_indices.push(idx);
                            kept.push(column.clone());
                        }
                    }
                    store::retain_columns_tx(tx, &id, &kept, &keep_indices)?;
                    columns = kept;
                }

                store::set_columns(tx, &id, &columns)?;
                tx.execute(
                    "UPDATE datasets SET updated_at = ?1 WHERE id = ?2",
                    params![db::now(), id],
                )?;

                if input.persist_aliases {
                    for p in &pairs {
                        tx.execute(
                            "INSERT INTO alias_mappings (id, alias, standard, updated_at) VALUES (?1, ?2, ?3, ?4)
                             ON CONFLICT(alias) DO UPDATE SET standard = excluded.standard, updated_at = excluded.updated_at",
                            params![db::new_id(), p.from, p.to, db::now()],
                        )?;
                    }
                }
                store::get_dataset(tx, &id)
            })
        })
        .await?;
    Ok(Json(dataset))
}
