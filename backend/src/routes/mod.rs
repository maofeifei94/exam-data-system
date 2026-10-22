pub mod datasets;
pub mod fields;
pub mod flows;
pub mod pdf;
pub mod projects;

use crate::state::AppState;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde_json::json;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(health))
        // 考试项目
        .route("/api/projects", get(projects::list).post(projects::create))
        .route(
            "/api/projects/:id",
            get(projects::get_one)
                .put(projects::update)
                .delete(projects::remove),
        )
        // 数据集
        .route("/api/datasets", get(datasets::list))
        .route("/api/datasets/import", post(datasets::import))
        .route(
            "/api/datasets/:id",
            get(datasets::get_one).delete(datasets::remove),
        )
        .route("/api/datasets/:id/rows", get(datasets::rows))
        .route("/api/datasets/:id/export", get(datasets::export_csv))
        .route(
            "/api/datasets/:id/apply_mapping",
            post(datasets::apply_mapping),
        )
        // 标准字段与别名
        .route(
            "/api/standard-fields",
            get(fields::list_standard).post(fields::create_standard),
        )
        .route(
            "/api/standard-fields/:id",
            put(fields::update_standard).delete(fields::delete_standard),
        )
        .route(
            "/api/field-groups",
            get(fields::list_groups).post(fields::create_group),
        )
        .route(
            "/api/field-groups/:id",
            put(fields::update_group).delete(fields::delete_group),
        )
        .route(
            "/api/aliases",
            get(fields::list_aliases).post(fields::save_aliases),
        )
        .route("/api/aliases/:id", delete(fields::delete_alias))
        .route("/api/aliases/suggest", post(fields::suggest))
        // 流程
        .route("/api/operators", get(flows::operators))
        .route("/api/flows", get(flows::list).post(flows::create))
        .route("/api/flows/validate", post(flows::validate))
        .route(
            "/api/flows/:id",
            get(flows::get_one).put(flows::update).delete(flows::remove),
        )
        .route("/api/flows/:id/run", post(flows::run))
        .route("/api/flows/:id/runs", get(flows::runs))
        .route("/api/flows/:id/duplicate", post(flows::duplicate))
        // PDF 模板
        .route("/api/pdf-templates", get(pdf::list).post(pdf::create))
        .route(
            "/api/pdf-templates/:id",
            get(pdf::get_one).put(pdf::update).delete(pdf::remove),
        )
        .route("/api/pdf-templates/:id/render", post(pdf::render_saved))
        .route("/api/pdf-templates/:id/duplicate", post(pdf::duplicate))
        .route("/api/pdf-templates/preview", post(pdf::render_preview))
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}
