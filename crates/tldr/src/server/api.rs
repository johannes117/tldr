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

use super::{ReviewerIdentity, ServerCtx};
use crate::watcher::{PrWatcher, WatchRegistry, WatcherDeps};
use crate::{config, coverage, diff, draft, editor, github, lsp, reviewer_framing, state, worktree};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{Stream, StreamExt};
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;

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
        .route("/pr/:n/framing", get(get_framing))
        .route("/pr/:n/events", get(get_pr_events))
        .route("/pr/:n/status", get(get_pr_status))
        .route("/pr/:n/coverage", get(get_coverage))
        .route("/pr/:n/call-graph", get(get_call_graph))
        .route("/pr/:n/blast/:sym", get(get_blast_radius))
        .route("/editor/open", post(post_editor_open))
        .route("/pr/:n/walkthrough", get(super::walkthrough::get_walkthrough))
        .route("/pr/:n/walkthrough/generate", post(super::walkthrough::post_generate))
        .route("/pr/:n/walkthrough/usage", get(super::walkthrough::get_usage))
        .route("/ai/status", get(super::walkthrough::get_status))
        .route("/ai/confirm", post(super::walkthrough::post_confirm))
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

    // Spawn PR watcher.
    {
        let mut watchers = ctx.watchers.lock().await;
        if !watchers.contains_key(&b.pr_number) {
            let reg = std::sync::Arc::new(WatchRegistry::new());
            {
                let mut s = reg.status.write().await;
                s.head_sha = meta.head_sha.clone();
                s.pr_state = meta.state.clone();
            }
            let deps = WatcherDeps {
                token: ctx.token.clone(),
                slug: ctx.slug.clone(),
                pr_number: b.pr_number as u32,
                repo_root: ctx.repo.root.clone(),
                worktree: wt.clone(),
                indexer: ctx.indexer.clone(),
                tx: reg.tx.clone(),
                status: reg.status.clone(),
            };
            let w = PrWatcher::spawn(deps);
            *reg.watcher.lock().await = Some(w);
            watchers.insert(b.pr_number, reg);
        }
    }

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

async fn resolve_reviewer(ctx: &ServerCtx) -> ReviewerIdentity {
    {
        let guard = ctx.reviewer.lock().await;
        if let Some(r) = guard.as_ref() {
            return r.clone();
        }
    }
    let client = github::Client::new(ctx.token.clone());
    let mut ident = ReviewerIdentity::default();
    if let Ok(v) = github::rest_json::<serde_json::Value>(
        &client, reqwest::Method::GET, "https://api.github.com/user", None,
    ).await {
        ident.login = v.get("login").and_then(|x| x.as_str()).map(|s| s.to_string());
        ident.email = v.get("email").and_then(|x| x.as_str()).map(|s| s.to_string());
    }
    if ident.email.is_none() {
        if let Ok(arr) = github::rest_json::<serde_json::Value>(
            &client, reqwest::Method::GET, "https://api.github.com/user/emails", None,
        ).await {
            if let Some(list) = arr.as_array() {
                let primary = list.iter().find(|e| e.get("primary").and_then(|b| b.as_bool()).unwrap_or(false))
                    .or_else(|| list.first());
                if let Some(e) = primary {
                    ident.email = e.get("email").and_then(|x| x.as_str()).map(|s| s.to_string());
                }
            }
        }
    }
    if ident.email.is_none() {
        if let Ok(out) = std::process::Command::new("git")
            .args(["-C", ctx.repo.root.to_string_lossy().as_ref(), "config", "user.email"])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() { ident.email = Some(s); }
            }
        }
    }
    *ctx.reviewer.lock().await = Some(ident.clone());
    ident
}

async fn get_framing(
    State(ctx): State<ServerCtx>,
    Path(_n): Path<u64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let pr = ctx.pr.lock().await.clone();
    let base = worktree::merge_base(&ctx.repo.root, &pr.base_sha, &pr.head_sha).map_err(err)?;
    let d = diff::compute(&ctx.worktree, &base, &pr.head_sha).map_err(err)?;
    let files: Vec<String> = d.files.iter().map(|f| f.path.clone()).collect();
    let ident = resolve_reviewer(&ctx).await;
    let frames = reviewer_framing::frame_files(
        &ctx.repo.root,
        ident.login.as_deref(),
        ident.email.as_deref(),
        &files,
    ).await;
    Ok(Json(json!({
        "reviewer": { "login": ident.login, "email": ident.email },
        "files": frames,
    })))
}

async fn get_pr_events(
    State(ctx): State<ServerCtx>,
    Path(n): Path<u64>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let reg = {
        let w = ctx.watchers.lock().await;
        w.get(&n).cloned()
    }.ok_or((StatusCode::NOT_FOUND, "watcher not started".into()))?;
    let rx = reg.tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|r| async move {
        match r {
            Ok(ev) => match serde_json::to_string(&ev) {
                Ok(s) => Some(Ok::<_, Infallible>(Event::default().data(s))),
                Err(_) => None,
            },
            Err(_) => None,
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn get_pr_status(
    State(ctx): State<ServerCtx>,
    Path(n): Path<u64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let reg = {
        let w = ctx.watchers.lock().await;
        w.get(&n).cloned()
    }.ok_or((StatusCode::NOT_FOUND, "watcher not started".into()))?;
    let s = reg.status.read().await.clone();
    Ok(Json(json!({
        "head_sha": s.head_sha,
        "comment_count": s.comment_count,
        "ci_state": s.ci_state,
        "pr_state": s.pr_state,
    })))
}

async fn get_coverage(
    State(ctx): State<ServerCtx>,
    Path(_n): Path<u64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let pr = ctx.pr.lock().await.clone();
    let repo_cfg = config::load_repo_config(&ctx.worktree);
    let cov_cfg = repo_cfg.as_ref().and_then(|c| c.coverage.clone()).unwrap_or_default();
    let excludes: Vec<regex::Regex> = cov_cfg
        .exclude_patterns.clone().unwrap_or_default().into_iter()
        .filter_map(|p| regex::Regex::new(&p).ok()).collect();

    let mut source: Option<&'static str> = None;
    let mut files: HashMap<String, coverage::FileCoverage> = HashMap::new();

    if let Some(rel) = cov_cfg.lcov_path.as_deref() {
        let p = ctx.worktree.join(rel);
        if let Ok(s) = std::fs::read_to_string(&p) {
            if let Ok(parsed) = coverage::parse_any(&s, Some(rel)) {
                files = parsed; source = Some("local");
            }
        }
    }
    if source.is_none() {
        if let Some((_fmt, name, text)) = coverage::detect_local(&ctx.worktree) {
            if let Ok(parsed) = coverage::parse_any(&text, Some(&name)) {
                files = parsed; source = Some("local");
            }
        }
    }
    if source.is_none() {
        if let Ok(Some(raw)) = coverage::fetch_ci_artifact(&ctx.token, &ctx.slug.owner, &ctx.slug.name, &pr.head_sha).await {
            if let Ok(parsed) = coverage::parse_any(&raw.text, None) {
                files = parsed; source = Some("ci");
            }
        }
    }

    let base = worktree::merge_base(&ctx.repo.root, &pr.base_sha, &pr.head_sha).map_err(err)?;
    let d = diff::compute(&ctx.worktree, &base, &pr.head_sha).ok();

    let mut out_files = serde_json::Map::new();
    let mut total_new_uncovered: u64 = 0;
    let mut files_with_uncovered: u64 = 0;

    if let Some(d) = &d {
        for f in &d.files {
            if excludes.iter().any(|r| r.is_match(&f.path)) { continue; }
            let fc = match_file_cov(&files, &f.path);
            let added_lines: Vec<u32> = f.hunks.iter()
                .flat_map(|h| h.lines.iter().filter_map(|l| if l.kind == "add" { l.new_line } else { None }))
                .collect();
            let mut lines_obj = serde_json::Map::new();
            let mut added_covered: u64 = 0;
            let mut added_uncovered: u64 = 0;
            let mut has_uncovered = false;
            if let Some(fc) = fc {
                for ln in &added_lines {
                    let hit = fc.lines.get(ln).copied().unwrap_or(coverage::Hit::NotInstrumented);
                    lines_obj.insert(ln.to_string(), serde_json::Value::String(coverage::line_str(hit).into()));
                    match hit {
                        coverage::Hit::Covered => added_covered += 1,
                        coverage::Hit::Uncovered => { added_uncovered += 1; has_uncovered = true; }
                        coverage::Hit::NotInstrumented => {}
                    }
                }
            }
            if has_uncovered { files_with_uncovered += 1; }
            total_new_uncovered += added_uncovered;
            let percent_after = fc.map(|fc| cov_percent(&fc.lines));
            out_files.insert(f.path.clone(), json!({
                "lines": lines_obj,
                "delta": {
                    "added_covered": added_covered,
                    "added_uncovered": added_uncovered,
                    "percent_before": serde_json::Value::Null,
                    "percent_after": percent_after,
                }
            }));
        }
    }

    Ok(Json(json!({
        "files": out_files,
        "summary": {
            "new_uncovered_lines": total_new_uncovered,
            "files_with_uncovered": files_with_uncovered,
        },
        "source": source,
    })))
}

fn match_file_cov<'a>(
    files: &'a HashMap<String, coverage::FileCoverage>,
    path: &str,
) -> Option<&'a coverage::FileCoverage> {
    if let Some(f) = files.get(path) { return Some(f); }
    for (k, v) in files {
        if k.ends_with(path) || path.ends_with(k.as_str()) { return Some(v); }
    }
    None
}

fn cov_percent(lines: &HashMap<u32, coverage::Hit>) -> Option<f64> {
    let mut inst = 0u64;
    let mut cov = 0u64;
    for h in lines.values() {
        match h {
            coverage::Hit::Covered => { inst += 1; cov += 1; }
            coverage::Hit::Uncovered => { inst += 1; }
            coverage::Hit::NotInstrumented => {}
        }
    }
    if inst == 0 { None } else { Some((cov as f64) * 100.0 / (inst as f64)) }
}

// ============================================================
// Call-graph diff (§8.2) and blast radius (§8.3)
// ============================================================

async fn get_call_graph(
    State(ctx): State<ServerCtx>,
    Path(_n): Path<u64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    use crate::indexer::Symbol;
    let pr = ctx.pr.lock().await.clone();
    let slug = format!("{}", ctx.slug);
    let base = worktree::merge_base(&ctx.repo.root, &pr.base_sha, &pr.head_sha).map_err(err)?;
    let d = diff::compute(&ctx.worktree, &base, &pr.head_sha).map_err(err)?;
    let changed_paths: Vec<String> = d.files.iter().map(|f| f.path.clone()).collect();

    let mut head_syms: Vec<Symbol> = Vec::new();
    for p in &changed_paths {
        if let Ok(mut s) = ctx.indexer.symbols_for_file(&slug, &pr.head_sha, p) { head_syms.append(&mut s); }
    }

    let mut changed_lines: HashMap<String, Vec<u32>> = HashMap::new();
    for f in &d.files {
        let mut v = Vec::new();
        for h in &f.hunks {
            for l in &h.lines {
                if l.kind == "add" || l.kind == "del" {
                    if let Some(n) = l.new_line.or(l.old_line) { v.push(n as u32); }
                }
            }
        }
        changed_lines.insert(f.path.clone(), v);
    }
    let is_changed = |sym: &Symbol| -> bool {
        changed_lines.get(&sym.file_path).map(|ls| {
            ls.iter().any(|l| *l >= sym.start_line && *l <= sym.end_line)
        }).unwrap_or(false)
    };

    let nodes: Vec<serde_json::Value> = head_syms.iter().filter(|s| s.kind == "func" || s.kind == "method").map(|s| {
        json!({
            "id": s.qualified_name,
            "qualified_name": s.qualified_name,
            "path": s.file_path,
            "line": s.start_line,
            "changed": is_changed(s),
        })
    }).collect();

    let head_edges = ctx.indexer.call_edges(&slug, &pr.head_sha).unwrap_or_default();
    let base_edges = ctx.indexer.call_edges(&slug, &pr.base_sha).unwrap_or_default();
    let mut edges = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for e in &head_edges {
        let key = (e.from.clone(), e.to.clone());
        if !seen.insert(key) { continue; }
        let matched = base_edges.iter().find(|b| b.from == e.from && b.to == e.to);
        let status = match matched {
            None => "added",
            Some(b) => if !b.site_text.is_empty() && !e.site_text.is_empty() && b.site_text != e.site_text { "args-changed" } else { "same" },
        };
        edges.push(json!({
            "from": e.from, "to": e.to, "status": status,
            "call_sites": [{"path": e.path, "line": e.line}],
        }));
    }
    for e in &base_edges {
        let key = (e.from.clone(), e.to.clone());
        if !seen.insert(key) { continue; }
        edges.push(json!({
            "from": e.from, "to": e.to, "status": "removed",
            "call_sites": [{"path": e.path, "line": e.line}],
        }));
    }
    Ok(Json(json!({ "nodes": nodes, "edges": edges })))
}

#[derive(Deserialize)]
struct BlastQuery { path: Option<String>, line: Option<u32> }

async fn get_blast_radius(
    State(ctx): State<ServerCtx>,
    Path((_n, sym)): Path<(u64, String)>,
    Query(q): Query<BlastQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let pr = ctx.pr.lock().await.clone();
    let slug = format!("{}", ctx.slug);
    let head = ctx.indexer.symbol_by_qualified_name(&slug, &pr.head_sha, &sym).map_err(err)?;
    let sym_path = q.path.clone().or_else(|| head.as_ref().map(|s| s.file_path.clone()))
        .ok_or((StatusCode::NOT_FOUND, "symbol path unknown".into()))?;
    let sym_line = q.line.or_else(|| head.as_ref().map(|s| s.start_line))
        .ok_or((StatusCode::NOT_FOUND, "symbol line unknown".into()))?;

    let pool = ctx.lsp.clone();
    let references = collect_references(&pool, &ctx.worktree, &sym_path, sym_line).await;
    let mb = worktree::merge_base(&ctx.repo.root, &pr.base_sha, &pr.head_sha).map_err(err)?;
    let dobj = diff::compute(&ctx.worktree, &mb, &pr.head_sha).ok();
    let in_pr_paths: std::collections::HashSet<String> = dobj
        .map(|d| d.files.into_iter().map(|f| f.path).collect())
        .unwrap_or_default();

    let refs_out: Vec<serde_json::Value> = references.into_iter().map(|(p, l, diag, msg)| {
        json!({
            "path": p, "line": l,
            "in_pr_diff": in_pr_paths.contains(&p),
            "diagnostic": diag, "diag_msg": msg,
        })
    }).collect();

    let head_sig = head.as_ref().and_then(|s| s.signature.clone()).unwrap_or_default();
    let base_sig = ctx.indexer
        .symbol_by_qualified_name(&slug, &pr.base_sha, &sym).ok().flatten()
        .and_then(|s| s.signature).unwrap_or_default();

    Ok(Json(json!({
        "symbol": {
            "qualified_name": sym,
            "path": sym_path,
            "line": sym_line,
            "kind": head.as_ref().map(|s| s.kind.clone()).unwrap_or_default(),
        },
        "signature_before": base_sig,
        "signature_after": head_sig,
        "references": refs_out,
    })))
}

async fn collect_references(
    pool: &std::sync::Arc<lsp::LspPool>,
    workspace: &std::path::Path,
    rel_path: &str,
    line: u32,
) -> Vec<(String, u32, &'static str, String)> {
    let ext = std::path::Path::new(rel_path).extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = match lsp::ext_to_lang(ext) { Some(l) => l, None => return vec![] };
    let handle = match pool.get_or_spawn(lang).await { Some(h) => h, None => return vec![] };
    let abs = workspace.join(rel_path);
    let text = std::fs::read_to_string(&abs).unwrap_or_default();
    let _ = handle.did_open(&abs, &text).await;
    let uri = match url::Url::from_file_path(&abs) { Ok(u) => u.to_string(), Err(_) => return vec![] };
    let params = serde_json::json!({
        "textDocument": {"uri": uri},
        "position": {"line": line.saturating_sub(1), "character": 0},
        "context": {"includeDeclaration": false},
    });
    let resp: serde_json::Value = match handle.request("textDocument/references", params).await {
        Ok(v) => v, Err(_) => return vec![],
    };
    let mut out = Vec::new();
    if let Some(arr) = resp.as_array() {
        for r in arr {
            let u = r.get("uri").and_then(|x| x.as_str()).unwrap_or("");
            let ln = r.get("range").and_then(|x| x.get("start")).and_then(|x| x.get("line")).and_then(|x| x.as_u64()).unwrap_or(0) as u32 + 1;
            let path_rel = url::Url::parse(u).ok()
                .and_then(|u| u.to_file_path().ok())
                .and_then(|p| p.strip_prefix(workspace).ok().map(|p| p.to_string_lossy().to_string()))
                .unwrap_or_else(|| u.to_string());
            let diag_params = serde_json::json!({ "textDocument": {"uri": u} });
            let (status, msg) = match handle.request::<_, serde_json::Value>("textDocument/diagnostic", diag_params).await {
                Ok(v) => classify_diag(&v, ln),
                Err(_) => ("ok", String::new()),
            };
            out.push((path_rel, ln, status, msg));
        }
    }
    out
}

fn classify_diag(v: &serde_json::Value, line: u32) -> (&'static str, String) {
    let items = v.get("items").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    for d in &items {
        let dl = d.get("range").and_then(|r| r.get("start")).and_then(|s| s.get("line")).and_then(|l| l.as_u64()).unwrap_or(0) as u32 + 1;
        if dl != line { continue; }
        let sev = d.get("severity").and_then(|s| s.as_u64()).unwrap_or(3);
        let msg = d.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
        if sev == 1 { return ("error", msg); }
        if sev == 2 { return ("warning", msg); }
    }
    ("ok", String::new())
}
