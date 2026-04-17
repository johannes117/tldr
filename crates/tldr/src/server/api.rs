// MVP uses plain REST. TODO(future): JSON-RPC + WebSocket for realtime.
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

use super::ServerCtx;
use crate::{config, diff, draft, editor, github, state, worktree};

pub fn router() -> Router<ServerCtx> {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/session", get(get_session))
        .route("/session/open-pr", post(post_open_pr))
        .route("/session/shutdown", post(post_shutdown))
        .route("/pr/:n", get(get_pr))
        .route("/pr/:n/diff", get(get_diff))
        .route("/pr/:n/diff/file", get(get_diff_file))
        .route("/pr/:n/collaborators", get(get_collaborators))
        .route("/pr/:n/draft", get(get_draft).put(put_draft))
        .route("/pr/:n/comments", post(post_comment))
        .route("/pr/:n/files/*path/state", put(put_file_state))
        .route("/pr/:n/submit", post(post_submit))
        .route("/pr/:n/symbols", get(get_symbols))
        .route("/pr/:n/index-status", get(get_index_status))
        .route("/editor/open", post(post_editor_open))
}

#[derive(Deserialize)]
struct EditorOpenBody {
    path: String,
    line: Option<u32>,
    col: Option<u32>,
}

async fn post_editor_open(
    State(ctx): State<ServerCtx>,
    Json(b): Json<EditorOpenBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let cfg = config::Config::load().map_err(err)?;
    let ed = cfg.editor.ok_or_else(|| (StatusCode::BAD_REQUEST, "no editor configured; run `tldr editor <name>`".to_string()))?;
    editor::launch(&ed, &ctx.worktree, Some(&b.path), b.line, b.col)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}

async fn get_session(State(ctx): State<ServerCtx>) -> impl IntoResponse {
    let prs: Vec<u64> = ctx.prs.lock().await.iter().map(|p| p.number).collect();
    Json(json!({
        "pid": std::process::id(),
        "port": ctx.port,
        "started_at": ctx.started_at,
        "slug": format!("{}", ctx.slug),
        "active_prs": prs,
    }))
}

#[derive(Deserialize)]
struct OpenPrBody { pr_number: u64 }

async fn post_open_pr(State(ctx): State<ServerCtx>, Json(b): Json<OpenPrBody>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    {
        let prs = ctx.prs.lock().await;
        if prs.iter().any(|p| p.number == b.pr_number) {
            return Ok(Json(json!({"ok": true, "already": true})));
        }
    }
    let client = github::Client::new(ctx.token.clone());
    let meta = client.fetch_pr(&ctx.slug, b.pr_number).await.map_err(err)?;
    worktree::fetch_pr_ref(&ctx.repo.root, b.pr_number).map_err(err)?;
    let wt = state::worktree_path(&ctx.slug, b.pr_number).map_err(err)?;
    worktree::ensure_worktree(&ctx.repo.root, &wt, b.pr_number).map_err(err)?;
    let draft_path = state::draft_path(&ctx.slug, b.pr_number).map_err(err)?;
    if !draft_path.exists() {
        let d = draft::load(&draft_path, b.pr_number).map_err(err)?;
        draft::save(&draft_path, &d).map_err(err)?;
    }
    ctx.prs.lock().await.push(meta.clone());
    ctx.write_session().await.ok();

    // Spawn background indexing for base + head.
    let indexer = ctx.indexer.clone();
    let slug_str = format!("{}", ctx.slug);
    let worktree = wt.clone();
    let repo_root = ctx.repo.root.clone();
    let meta_clone = meta.clone();
    tokio::task::spawn_blocking(move || {
        let merge_base = worktree::merge_base(&repo_root, &meta_clone.base_sha, &meta_clone.head_sha).ok();
        if let Err(e) = indexer.record_pr(
            &slug_str, meta_clone.number, &meta_clone.head_sha, &meta_clone.base_sha,
            merge_base.as_deref(), Some(&meta_clone.title), meta_clone.body.as_deref(), meta_clone.author.as_deref(),
        ) { tracing::warn!(error = %e, "record_pr failed"); }
        tracing::info!(target: "index.phase", phase = "start", sha = %meta_clone.base_sha, "indexing base");
        if let Err(e) = indexer.index_tree(&slug_str, &meta_clone.base_sha, &worktree) {
            tracing::warn!(error = %e, "index base failed");
        }
        tracing::info!(target: "index.phase", phase = "start", sha = %meta_clone.head_sha, "indexing head");
        if let Err(e) = indexer.index_tree(&slug_str, &meta_clone.head_sha, &worktree) {
            tracing::warn!(error = %e, "index head failed");
        }
    });

    Ok(Json(json!({"ok": true, "pr": b.pr_number})))
}

async fn post_shutdown(State(ctx): State<ServerCtx>) -> impl IntoResponse {
    let notify = ctx.shutdown.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        notify.notify_waiters();
    });
    Json(json!({"ok": true}))
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

#[derive(Deserialize)]
struct DiffFileQuery { path: String, #[serde(default)] expand: bool }

async fn get_diff_file(
    State(ctx): State<ServerCtx>,
    Path(_n): Path<u64>,
    Query(q): Query<DiffFileQuery>,
) -> Result<Json<diff::FileDiff>, (StatusCode, String)> {
    let pr = ctx.pr.lock().await.clone();
    let base = worktree::merge_base(&ctx.repo.root, &pr.base_sha, &pr.head_sha).map_err(err)?;
    let _ = q.expand;
    let f = diff::compute_file(&ctx.worktree, &base, &pr.head_sha, &q.path)
        .map_err(err)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "file not found".to_string()))?;
    Ok(Json(f))
}

async fn get_collaborators(
    State(ctx): State<ServerCtx>,
    Path(_n): Path<u64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let client = github::Client::new(ctx.token.clone());
    let url = format!(
        "https://api.github.com/repos/{}/{}/collaborators?per_page=100",
        ctx.slug.owner, ctx.slug.name
    );
    let arr: serde_json::Value = github::rest_json(&client, reqwest::Method::GET, &url, None)
        .await
        .unwrap_or(serde_json::Value::Array(vec![]));
    let logins: Vec<String> = arr
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.get("login").and_then(|x| x.as_str()).map(String::from)).collect())
        .unwrap_or_default();
    Ok(Json(json!({ "logins": logins })))
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

async fn get_symbols(
    State(ctx): State<ServerCtx>,
    Path(_n): Path<u64>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let path = q.get("path").cloned().ok_or((StatusCode::BAD_REQUEST, "path required".into()))?;
    let side = q.get("side").map(String::as_str).unwrap_or("head");
    let pr = ctx.pr.lock().await.clone();
    let sha = if side == "base" { pr.base_sha.clone() } else { pr.head_sha.clone() };
    let slug_str = format!("{}", ctx.slug);
    let syms = ctx.indexer.symbols_for_file(&slug_str, &sha, &path).map_err(err)?;
    Ok(Json(json!({ "symbols": syms, "commit_sha": sha, "path": path, "side": side })))
}

async fn get_index_status(
    State(ctx): State<ServerCtx>,
    Path(_n): Path<u64>,
) -> Json<serde_json::Value> {
    let s = ctx.indexer.status();
    Json(json!({
        "phase": s.phase,
        "files_done": s.files_done,
        "files_total": s.files_total,
        "symbols_count": s.symbols_count,
        "errors": s.errors,
    }))
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
