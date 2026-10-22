//! PDF 模板接口与渲染。
//!
//! 设计约定：
//! - 模板正文以 JSON 存在 `pdf_templates.template`，读取时严格解析；
//!   损坏的数据必须报错，不能静默降级成空模板（否则保存一次就永久丢内容）。
//! - 渲染是 CPU 密集任务：先在阻塞线程里读出数据并释放连接，再单独做排版，
//!   避免一个大 PDF 长时间占用连接池。

use crate::db::{self, new_id, now};
use crate::error::{AppError, AppResult};
use crate::models::{
    DatasetColumn, DuplicateInput, PdfTemplate, PdfTemplateInput, PreviewInput, RenderInput,
    TemplateBody,
};
use crate::pdfgen::{self, RenderOpts};
use crate::state::AppState;
use crate::store;
use crate::util;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, Response, StatusCode};
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

const TPL_COLS: &str =
    "id, name, kind, description, template, scope, project_id, source_id, created_at, updated_at";

fn parse_body(raw: &str) -> rusqlite::Result<TemplateBody> {
    serde_json::from_str(raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn row_to_template(r: &rusqlite::Row<'_>) -> rusqlite::Result<(PdfTemplate, String)> {
    let raw: String = r.get(4)?;
    let kind: String = r.get(2)?;
    let mut body = parse_body(&raw)?;
    body.normalize(&kind);
    Ok((
        PdfTemplate {
            id: r.get(0)?,
            name: r.get(1)?,
            kind,
            description: r.get(3)?,
            page: body.page,
            matrix: body.matrix,
            elements: body.elements,
            scope: r.get(5)?,
            project_id: r.get(6)?,
            source_id: r.get(7)?,
            created_at: r.get(8)?,
            updated_at: r.get(9)?,
        },
        raw,
    ))
}

fn fetch_template(conn: &Connection, id: &str) -> AppResult<(PdfTemplate, String)> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TPL_COLS} FROM pdf_templates WHERE id = ?1"
    ))?;
    stmt.query_row(params![id], row_to_template)
        .optional()?
        .ok_or_else(|| AppError::not_found("PDF 模板不存在"))
}

#[derive(Debug, Deserialize)]
pub struct TplQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TplQuery>,
) -> AppResult<Json<Vec<PdfTemplate>>> {
    let templates = state
        .db(move |conn| {
            let mut sql = format!("SELECT {TPL_COLS} FROM pdf_templates");
            let mut binds: Vec<String> = Vec::new();
            if let Some(pid) = q.project_id.as_deref().filter(|s| !s.trim().is_empty()) {
                sql.push_str(" WHERE project_id = ?1");
                binds.push(pid.to_string());
            } else if let Some(sc) = q.scope.as_deref().filter(|s| !s.trim().is_empty()) {
                sql.push_str(" WHERE scope = ?1");
                binds.push(sc.to_string());
            }
            sql.push_str(" ORDER BY updated_at DESC, id ASC");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(binds.iter()), |r| {
                row_to_template(r).map(|(t, _)| t)
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?;
    Ok(Json(templates))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<PdfTemplate>> {
    let (t, _) = state.db(move |conn| fetch_template(conn, &id)).await?;
    Ok(Json(t))
}

fn validate_name(raw: &str) -> AppResult<String> {
    let name = raw.trim().to_string();
    if name.is_empty() {
        return Err(AppError::validation("模板名称不能为空"));
    }
    if name.chars().count() > 120 {
        return Err(AppError::validation("模板名称不能超过 120 个字符"));
    }
    Ok(name)
}

fn validate_kind(raw: &str) -> AppResult<String> {
    let kind = raw.trim().to_ascii_lowercase();
    if kind.is_empty() {
        return Ok("single".to_string());
    }
    if !matches!(kind.as_str(), "single" | "matrix") {
        return Err(AppError::validation(format!("不支持的模板类型 [{kind}]")));
    }
    Ok(kind)
}

fn body_from_input(input: &PdfTemplateInput, kind: &str) -> TemplateBody {
    let mut body = TemplateBody {
        page: input.page.clone(),
        matrix: input.matrix.clone(),
        elements: input.elements.clone(),
    };
    body.normalize(kind);
    body
}

fn resolve_scope(scope: Option<&str>, project_id: &str) -> String {
    match scope.map(|s| s.trim()) {
        Some("project") if !project_id.is_empty() => "project".to_string(),
        Some("global") => "global".to_string(),
        _ if !project_id.is_empty() => "project".to_string(),
        _ => "global".to_string(),
    }
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Json(input): Json<PdfTemplateInput>,
) -> AppResult<Json<PdfTemplate>> {
    let name = validate_name(&input.name)?;
    let kind = validate_kind(&input.kind)?;
    let body = body_from_input(&input, &kind);
    let project_id = input.project_id.clone().unwrap_or_default();
    let scope = resolve_scope(input.scope.as_deref(), &project_id);
    let description = input.description;
    let body_json = serde_json::to_string(&body)?;
    let id = new_id();
    let ts = now();

    let template = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                tx.execute(
                    "INSERT INTO pdf_templates
                       (id, name, kind, description, template, scope, project_id, source_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '', ?8, ?8)",
                    params![id, name, kind, description, body_json, scope, project_id, ts],
                )?;
                fetch_template(tx, &id)
            })
        })
        .await?
        .0;
    Ok(Json(template))
}

/// 把（通常是全局库里的）PDF 模板复制成一个考试项目的本地副本。
pub async fn duplicate(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<DuplicateInput>,
) -> AppResult<Json<PdfTemplate>> {
    let project_id = input.project_id.trim().to_string();
    if project_id.is_empty() {
        return Err(AppError::validation("必须指定目标考试项目"));
    }
    let requested_name = input.name;
    let new = new_id();

    let template = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                let (src, raw) = fetch_template(tx, &id)?;
                let name = requested_name
                    .map(|n| n.trim().to_string())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("{}（本项目）", src.name));
                if name.chars().count() > 120 {
                    return Err(AppError::validation("模板名称不能超过 120 个字符"));
                }
                tx.execute(
                    "INSERT INTO pdf_templates
                       (id, name, kind, description, template, scope, project_id, source_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'project', ?6, ?7, ?8, ?8)",
                    params![new, name, src.kind, src.description, raw, project_id, id, now()],
                )?;
                fetch_template(tx, &new)
            })
        })
        .await?
        .0;
    Ok(Json(template))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<PdfTemplateInput>,
) -> AppResult<Json<PdfTemplate>> {
    let name = validate_name(&input.name)?;
    let kind = validate_kind(&input.kind)?;
    let body = body_from_input(&input, &kind);
    let description = input.description;
    let body_json = serde_json::to_string(&body)?;

    let template = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                tx.execute(
                    "UPDATE pdf_templates
                     SET name = ?1, kind = ?2, description = ?3, template = ?4, updated_at = ?5
                     WHERE id = ?6",
                    params![name, kind, description, body_json, now(), id],
                )?;
                fetch_template(tx, &id)
            })
        })
        .await?
        .0;
    Ok(Json(template))
}

/// 删除模板时把引用它的考试项目解绑，避免项目详情指向已删除的模板。
pub async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                fetch_template(tx, &id)?;
                tx.execute(
                    "UPDATE exam_projects SET pdf_template_id = '', updated_at = ?1
                     WHERE pdf_template_id = ?2",
                    params![now(), id],
                )?;
                tx.execute("DELETE FROM pdf_templates WHERE id = ?1", params![id])?;
                Ok(())
            })
        })
        .await?;
    Ok(Json(json!({ "ok": true })))
}

fn pdf_response(bytes: Vec<u8>, filename: &str) -> AppResult<Response<Body>> {
    let encoded = util::urlencode(filename);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/pdf")
        .header(
            header::CONTENT_DISPOSITION,
            format!("inline; filename=\"export.pdf\"; filename*=UTF-8''{encoded}"),
        )
        .body(Body::from(bytes))
        .map_err(|e| AppError::internal_with_context("构建 PDF 响应失败", e))
}

/// 读出渲染所需的全部数据；返回后连接立即归还连接池。
fn load_render_data(
    conn: &Connection,
    dataset_id: &str,
    limit: Option<i64>,
) -> AppResult<(Vec<DatasetColumn>, Vec<Vec<String>>)> {
    store::get_dataset(conn, dataset_id)?;
    store::read_all_strings(conn, dataset_id, limit)
}

pub async fn render_saved(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<RenderInput>,
) -> AppResult<Response<Body>> {
    let RenderInput {
        dataset_id,
        limit,
        page,
        photo_dir,
    } = input;
    let (body, kind, name, cols, rows) = state
        .db(move |conn| {
            let (t, raw) = fetch_template(conn, &id)?;
            let mut body = parse_body(&raw)
                .map_err(|e| AppError::internal_with_context("PDF 模板数据损坏", e))?;
            body.normalize(&t.kind);
            let (cols, rows) = load_render_data(conn, &dataset_id, limit)?;
            Ok((body, t.kind, t.name, cols, rows))
        })
        .await?;

    let opts = RenderOpts {
        photo_dir,
        only_page: page,
    };
    let bytes = state
        .blocking(move || pdfgen::render_pdf_data(&cols, &rows, &body, &kind, &opts))
        .await?;
    pdf_response(bytes, &format!("{name}.pdf"))
}

pub async fn render_preview(
    State(state): State<Arc<AppState>>,
    Json(input): Json<PreviewInput>,
) -> AppResult<Response<Body>> {
    let PreviewInput {
        template,
        dataset_id,
        limit,
        photo_dir,
    } = input;
    let kind = validate_kind(&template.kind)?;
    let body = body_from_input(&template, &kind);
    let name = template.name;
    let limit = limit.or(Some(60));

    let (cols, rows) = state
        .db(move |conn| load_render_data(conn, &dataset_id, limit))
        .await?;

    let opts = RenderOpts {
        photo_dir,
        only_page: Some(1),
    };
    let bytes = state
        .blocking(move || pdfgen::render_pdf_data(&cols, &rows, &body, &kind, &opts))
        .await?;
    pdf_response(bytes, &format!("{name}.pdf"))
}
