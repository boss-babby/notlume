//! Page sync. The server stores opaque JSON per page with a last-write
//! timestamp; clients merge with `nw_core::sync::merge_pages`, so the server
//! never has to understand the document model.

use crate::auth::{now_ms, AuthUser};
use crate::error::AppError;
use crate::AppState;
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use nw_api_types::{PullResponse, PushRequest, SyncPage};
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;

const MAX_PAGES_PER_PUSH: usize = 1000;

#[derive(Deserialize)]
struct PullQuery {
    #[serde(default)]
    since: i64,
}

async fn pull(State(s): State<AppState>, user: AuthUser, Query(q): Query<PullQuery>) -> Result<Json<PullResponse>, AppError> {
    let rows = sqlx::query(
        "SELECT id, updated_ms, deleted, data FROM pages WHERE user_id = $1 AND updated_ms > $2 ORDER BY updated_ms LIMIT 5000",
    )
    .bind(user.user_id)
    .bind(q.since)
    .fetch_all(&s.db)
    .await?;
    let pages = rows
        .iter()
        .map(|r| SyncPage {
            id: r.get::<Uuid, _>("id"),
            updated: r.get::<i64, _>("updated_ms") as u64,
            deleted: r.get("deleted"),
            data: r.get::<serde_json::Value, _>("data"),
        })
        .collect();
    Ok(Json(PullResponse { pages, server_time: now_ms() as u64 }))
}

async fn push(State(s): State<AppState>, user: AuthUser, Json(req): Json<PushRequest>) -> Result<Json<serde_json::Value>, AppError> {
    if req.pages.len() > MAX_PAGES_PER_PUSH {
        return Err(AppError::BadRequest("too many pages in one push".into()));
    }
    let mut tx = s.db.begin().await?;
    for p in &req.pages {
        // Only accept a write that is at least as new as what is stored.
        sqlx::query(
            "INSERT INTO pages (user_id, id, updated_ms, deleted, data) VALUES ($1,$2,$3,$4,$5)
             ON CONFLICT (user_id, id) DO UPDATE
               SET updated_ms = EXCLUDED.updated_ms, deleted = EXCLUDED.deleted, data = EXCLUDED.data
               WHERE pages.updated_ms <= EXCLUDED.updated_ms",
        )
        .bind(user.user_id)
        .bind(p.id)
        .bind(p.updated as i64)
        .bind(p.deleted)
        .bind(&p.data)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(serde_json::json!({ "ok": true, "accepted": req.pages.len() })))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/pull", get(pull)).route("/push", post(push))
}
