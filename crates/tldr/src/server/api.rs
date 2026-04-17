// MVP uses plain REST. TODO(future): JSON-RPC + WebSocket for realtime.
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::ServerCtx;
use crate::{diff, draft, github, state, worktree};

pub fn router() -> Router<ServerCtx> {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/pr/:n", get(get_pr))
        .route("/pr/:n/diff", get(get_diff))
        .route("/pr/:n/draft", get(get_draft).put(put_draft))
        .route("/pr/:n/comments", post(post_comment))
        .route("/pr/:n/files/*path/state", put(put_file_state))
        .route("/pr/:n/submit", post(post_submit))
}

fn err<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn get_pr(State(ctx): State<ServerCtx>, Path(n): Path<u64>) -> impl IntoResponse {
    let pr = ctx.pr.lock().await.clone();
    Json(json!({
        "pr": pr,
        "worktree": ctx.worktree,
        "slug": format!("{}", ctx.slug),
        "_requested": n,
    }))
}

async fn get_diff(State(ctx): State<ServerCtx>, Path(n): Path<u64>) -> Result<Json<diff::Diff>, (StatusCode, String)> {
    let _ = n;
    let pr = ctx.pr.lock().await.clone();
    let base = worktree::merge_base(&ctx.repo.root, &pr.base_sha, &pr.head_sha).map_err(err)?;
    let d = diff::compute(&ctx.worktree, &base, &pr.head_sha).map_err(err)?;
    Ok(Json(d))
}

async fn get_draft(State(ctx): State<ServerCtx>, Path(n): Path<u64>) -> Result<Json<draft::Draft>, (StatusCode, String)> {
    let path = state::draft_path(&ctx.slug, n).map_err(err)?;
    Ok(Json(draft::load(&path, n).map_err(err)?))
}

async fn put_draft(State(ctx): State<ServerCtx>, Path(n): Path<u64>, Json(mut body): Json<draft::Draft>) -> Result<Json<draft::Draft>, (StatusCode, String)> {
    body.pr = n;
    let path = state::draft_path(&ctx.slug, n).map_err(err)?;
    draft::save(&path, &body).map_err(err)?;
    Ok(Json(body))
}

#[derive(Deserialize)]
struct NewComment {
    path: String,
    line: u32,
    side: Option<String>,
    body: String,
}

async fn post_comment(State(ctx): State<ServerCtx>, Path(n): Path<u64>, Json(c): Json<NewComment>) -> Result<Json<draft::Draft>, (StatusCode, String)> {
    let path = state::draft_path(&ctx.slug, n).map_err(err)?;
    let mut d = draft::load(&path, n).map_err(err)?;
    d.comments.push(draft::DraftComment {
        id: draft::new_comment_id(),
        path: c.path,
        line: c.line,
        side: c.side.unwrap_or_else(|| "RIGHT".into()),
        body: c.body,
        created_at: chrono::Utc::now().to_rfc3339(),
    });
    draft::save(&path, &d).map_err(err)?;
    Ok(Json(d))
}

#[derive(Deserialize)]
struct FileStateBody { viewed: Option<bool>, collapsed: Option<bool> }

async fn put_file_state(
    State(ctx): State<ServerCtx>,
    Path((n, p)): Path<(u64, String)>,
    Json(b): Json<FileStateBody>,
) -> Result<Json<draft::Draft>, (StatusCode, String)> {
    let path = state::draft_path(&ctx.slug, n).map_err(err)?;
    let mut d = draft::load(&path, n).map_err(err)?;
    let entry = d.file_state.entry(p).or_default();
    if let Some(v) = b.viewed { entry.viewed = v; }
    if let Some(c) = b.collapsed { entry.collapsed = c; }
    draft::save(&path, &d).map_err(err)?;
    Ok(Json(d))
}

#[derive(Serialize)]
struct SubmitResp { ok: bool, result: serde_json::Value }

async fn post_submit(State(ctx): State<ServerCtx>, Path(n): Path<u64>) -> Result<Json<SubmitResp>, (StatusCode, String)> {
    let path = state::draft_path(&ctx.slug, n).map_err(err)?;
    let d = draft::load(&path, n).map_err(err)?;
    let pr = ctx.pr.lock().await.clone();
    let client = github::Client::new(ctx.token.clone());
    let result = github::review::submit(&client, &ctx.slug, &pr.node_id, &pr.head_sha, &d).await.map_err(err)?;
    // clear draft on success (keep on failure)
    let _ = std::fs::remove_file(&path);
    Ok(Json(SubmitResp { ok: true, result }))
}
