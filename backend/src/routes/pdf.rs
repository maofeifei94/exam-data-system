use crate::db::{new_id, now};
use crate::error::{AppError, AppResult};
use crate::models::{
    DuplicateInput, PdfTemplate, PdfTemplateInput, PreviewInput, RenderInput, TemplateBody,
};
use crate::pdfgen::{self, RenderOpts};
use crate::state::AppState;
use crate::store;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, Response, StatusCode};
use axum::Json;
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

const TPL_COLS: &str = "id, name, kind, description, template, scope, project_id, source_id, created_at, updated_at";

fn row_to_template(r: &rusqlite::Row<'_>) -> rusqlite::Result<(PdfTemplate, String)> {
    let raw: String = r.get(4)?;
    let body: TemplateBody = serde_json::from_str(&raw).unwrap_or_default();
    Ok((
        PdfTemplate {
            id: r.get(0)?,
            name: r.get(1)?,
            kind: r.get(2)?,
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

fn fetch_template(conn: &rusqlite::Connection, id: &str) -> AppResult<(PdfTemplate, String)> {
    let mut stmt = conn.prepare(&format!("SELECT {TPL_COLS} FROM pdf_templates WHERE id = ?1"))?;
    stmt.query_row(params![id], |r| row_to_template(r))
        .map_err(|_| AppError::not_found("PDF 模板不存在"))
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
    let conn = state.lock()?;
    let mut sql = format!("SELECT {TPL_COLS} FROM pdf_templates");
    let mut binds: Vec<String> = Vec::new();
    if let Some(pid) = q.project_id.as_deref().filter(|s| !s.trim().is_empty()) {
        sql.push_str(" WHERE project_id = ?1");
        binds.push(pid.to_string());
    } else if let Some(sc) = q.scope.as_deref().filter(|s| !s.trim().is_empty()) {
        sql.push_str(" WHERE scope = ?1");
        binds.push(sc.to_string());
    }
    sql.push_str(" ORDER BY updated_at DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(binds.iter()), |r| row_to_template(r).map(|(t, _)| t))?;
    Ok(Json(rows.collect::<Result<Vec<_>, _>>()?))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<PdfTemplate>> {
    let conn = state.lock()?;
    let (t, _) = fetch_template(&conn, &id)?;
    Ok(Json(t))
}

fn body_from_input(input: &PdfTemplateInput) -> TemplateBody {
    TemplateBody {
        page: input.page.clone(),
        matrix: input.matrix.clone(),
        elements: input.elements.clone(),
    }
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
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::bad_request("模板名称不能为空"));
    }
    let body = body_from_input(&input);
    let project_id = input.project_id.clone().unwrap_or_default();
    let scope = resolve_scope(input.scope.as_deref(), &project_id);
    let id = new_id();
    let ts = now();
    let conn = state.lock()?;
    conn.execute(
        "INSERT INTO pdf_templates (id, name, kind, description, template, scope, project_id, source_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '', ?8, ?8)",
        params![
            id,
            name,
            input.kind,
            input.description,
            serde_json::to_string(&body)?,
            scope,
            project_id,
            ts
        ],
    )?;
    let (t, _) = fetch_template(&conn, &id)?;
    Ok(Json(t))
}

/// 把（通常是全局库里的）PDF 模板复制成一个考试项目的本地副本。
pub async fn duplicate(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<DuplicateInput>,
) -> AppResult<Json<PdfTemplate>> {
    let project_id = input.project_id.trim().to_string();
    if project_id.is_empty() {
        return Err(AppError::bad_request("必须指定目标考试项目"));
    }
    let conn = state.lock()?;
    let (src, raw) = fetch_template(&conn, &id)?;
    let new = new_id();
    let name = input
        .name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("{}（本项目）", src.name));
    let ts = now();
    conn.execute(
        "INSERT INTO pdf_templates (id, name, kind, description, template, scope, project_id, source_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'project', ?6, ?7, ?8, ?8)",
        params![new, name, src.kind, src.description, raw, project_id, id, ts],
    )?;
    let (t, _) = fetch_template(&conn, &new)?;
    Ok(Json(t))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<PdfTemplateInput>,
) -> AppResult<Json<PdfTemplate>> {
    let body = body_from_input(&input);
    let conn = state.lock()?;
    let n = conn.execute(
        "UPDATE pdf_templates SET name = ?1, kind = ?2, description = ?3, template = ?4, updated_at = ?5
         WHERE id = ?6",
        params![
            input.name.trim(),
            input.kind,
            input.description,
            serde_json::to_string(&body)?,
            now(),
            id
        ],
    )?;
    if n == 0 {
        return Err(AppError::not_found("PDF 模板不存在"));
    }
    let (t, _) = fetch_template(&conn, &id)?;
    Ok(Json(t))
}

pub async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let conn = state.lock()?;
    conn.execute("DELETE FROM pdf_templates WHERE id = ?1", params![id])?;
    Ok(Json(json!({ "ok": true })))
}

fn pdf_response(bytes: Vec<u8>, filename: &str) -> AppResult<Response<Body>> {
    let encoded: String = filename
        .bytes()
        .map(|b| {
            let c = b as char;
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                c.to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/pdf")
        .header(
            header::CONTENT_DISPOSITION,
            format!("inline; filename=\"export.pdf\"; filename*=UTF-8''{encoded}"),
        )
        .body(Body::from(bytes))
        .map_err(|e| AppError::internal(format!("{e}")))
}

pub async fn render_saved(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<RenderInput>,
) -> AppResult<Response<Body>> {
    let conn = state.lock()?;
    let (t, raw) = fetch_template(&conn, &id)?;
    let body: TemplateBody = serde_json::from_str(&raw).unwrap_or_default();
    let _ = store::get_dataset(&conn, &input.dataset_id)?;
    let opts = RenderOpts {
        limit: input.limit,
        photo_dir: input.photo_dir.clone(),
        only_page: input.page,
    };
    let bytes = pdfgen::render_pdf(&conn, &body, &t.kind, &input.dataset_id, &opts)?;
    pdf_response(bytes, &format!("{}.pdf", t.name))
}

pub async fn render_preview(
    State(state): State<Arc<AppState>>,
    Json(input): Json<PreviewInput>,
) -> AppResult<Response<Body>> {
    let conn = state.lock()?;
    let body = body_from_input(&input.template);
    let _ = store::get_dataset(&conn, &input.dataset_id)?;
    let opts = RenderOpts {
        limit: input.limit.or(Some(60)),
        photo_dir: input.photo_dir.clone(),
        only_page: Some(1),
    };
    let bytes = pdfgen::render_pdf(&conn, &body, &input.template.kind, &input.dataset_id, &opts)?;
    pdf_response(bytes, &format!("{}.pdf", input.template.name))
}
