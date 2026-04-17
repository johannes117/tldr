//! JSON-RPC 2.0 over WebSocket (SPEC §9.3).
//!
//! Second transport alongside REST. Delegates to the same primitives
//! (`draft`, `indexer`, `github`, `watcher`) used by the REST handlers in
//! `server::api`. Subscriptions broadcast server→client notifications.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::server::ServerCtx;
use crate::watcher::{PrWatcher, WatchRegistry, WatcherDeps};
use crate::{draft, github, state, worktree};

pub fn router() -> Router<ServerCtx> {
    Router::new().route("/ws", get(ws_upgrade))
}

#[derive(Deserialize)]
struct WsQuery {
    csrf: Option<String>,
}

async fn ws_upgrade(
    State(ctx): State<ServerCtx>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    use subtle::ConstantTimeEq;
    let supplied = q.csrf.unwrap_or_default();
    if supplied.as_bytes().ct_eq(ctx.csrf.as_bytes()).unwrap_u8() != 1 {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, ctx)))
}

/// Outbound message: either a response to a call, or a notification.
#[derive(Debug)]
enum Outbound {
    Frame(String),
}

async fn handle_socket(socket: WebSocket, ctx: ServerCtx) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Outbound>();

    // Writer task.
    let writer = tokio::spawn(async move {
        while let Some(Outbound::Frame(s)) = rx.recv().await {
            if sink.send(Message::Text(s)).await.is_err() {
                break;
            }
        }
    });

    // Subscription handles keyed by (kind, pr_number).
    let subs: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    while let Some(msg) = stream.next().await {
        let Ok(msg) = msg else { break };
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        let ctx = ctx.clone();
        let tx = tx.clone();
        let subs = subs.clone();
        tokio::spawn(async move {
            let parsed: Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.send(Outbound::Frame(err_frame(
                        Value::Null,
                        -32700,
                        &format!("parse error: {e}"),
                    )));
                    return;
                }
            };
            let id = parsed.get("id").cloned().unwrap_or(Value::Null);
            let method = parsed
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let params = parsed.get("params").cloned().unwrap_or(Value::Null);
            match dispatch(&ctx, &method, params, tx.clone(), subs.clone()).await {
                Ok(Some(result)) => {
                    let _ = tx.send(Outbound::Frame(resp_frame(id, result)));
                }
                Ok(None) => { /* subscription accepted or fire-and-forget */ }
                Err((code, msg)) => {
                    let _ = tx.send(Outbound::Frame(err_frame(id, code, &msg)));
                }
            }
        });
    }

    // Drop sender so writer exits; abort any subs.
    drop(tx);
    let mut guard = subs.lock().await;
    for h in guard.drain(..) {
        h.abort();
    }
    let _ = writer.await;
}

fn resp_frame(id: Value, result: Value) -> String {
    serde_json::to_string(&json!({"jsonrpc":"2.0","id":id,"result":result})).unwrap()
}
fn err_frame(id: Value, code: i64, msg: &str) -> String {
    serde_json::to_string(
        &json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":msg}}),
    )
    .unwrap()
}
fn notif_frame(method: &str, params: Value) -> String {
    serde_json::to_string(&json!({"jsonrpc":"2.0","method":method,"params":params})).unwrap()
}

type DispatchResult = Result<Option<Value>, (i64, String)>;

async fn dispatch(
    ctx: &ServerCtx,
    method: &str,
    params: Value,
    tx: mpsc::UnboundedSender<Outbound>,
    subs: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
) -> DispatchResult {
    let pr_num = || -> Result<u64, (i64, String)> {
        params
            .get("pr_number")
            .and_then(|v| v.as_u64())
            .ok_or((-32602, "pr_number required".into()))
    };
    match method {
        "review.open" => {
            let n = pr_num()?;
            open_pr(ctx, n).await.map(Some)
        }
        "review.get_draft" => {
            let n = pr_num()?;
            let path = state::draft_path(&ctx.slug, n).map_err(internal)?;
            let d = draft::load(&path, n).map_err(internal)?;
            Ok(Some(serde_json::to_value(d).unwrap()))
        }
        "review.add_comment" => {
            let n = pr_num()?;
            let c = params
                .get("comment")
                .cloned()
                .ok_or((-32602, "comment required".into()))?;
            let path = state::draft_path(&ctx.slug, n).map_err(internal)?;
            let mut d = draft::load(&path, n).map_err(internal)?;
            let id = draft::new_comment_id();
            d.comments.push(draft::DraftComment {
                id: id.clone(),
                path: c
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                line: c.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                side: c
                    .get("side")
                    .and_then(|v| v.as_str())
                    .unwrap_or("RIGHT")
                    .to_string(),
                body: c
                    .get("body")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            });
            draft::save(&path, &d).map_err(internal)?;
            Ok(Some(json!({ "comment_id": id })))
        }
        "review.update_comment" => {
            let n = pr_num()?;
            let cid = params
                .get("comment_id")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "comment_id required".into()))?
                .to_string();
            let patch = params.get("patch").cloned().unwrap_or(Value::Null);
            let path = state::draft_path(&ctx.slug, n).map_err(internal)?;
            let mut d = draft::load(&path, n).map_err(internal)?;
            let found = d.comments.iter_mut().find(|c| c.id == cid);
            let Some(c) = found else {
                return Err((-32004, "comment not found".into()));
            };
            if let Some(b) = patch.get("body").and_then(|v| v.as_str()) {
                c.body = b.to_string();
            }
            if let Some(line) = patch.get("line").and_then(|v| v.as_u64()) {
                c.line = line as u32;
            }
            if let Some(side) = patch.get("side").and_then(|v| v.as_str()) {
                c.side = side.to_string();
            }
            draft::save(&path, &d).map_err(internal)?;
            Ok(Some(json!({ "ok": true })))
        }
        "review.set_file_state" => {
            let n = pr_num()?;
            let p = params
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "path required".into()))?
                .to_string();
            let st = params.get("state").cloned().unwrap_or(Value::Null);
            let path = state::draft_path(&ctx.slug, n).map_err(internal)?;
            let mut d = draft::load(&path, n).map_err(internal)?;
            let entry = d.file_state.entry(p).or_default();
            if let Some(v) = st.get("viewed").and_then(|v| v.as_bool()) {
                entry.viewed = v;
            }
            if let Some(v) = st.get("collapsed").and_then(|v| v.as_bool()) {
                entry.collapsed = v;
            }
            draft::save(&path, &d).map_err(internal)?;
            Ok(Some(json!({ "ok": true })))
        }
        "review.submit" => {
            let n = pr_num()?;
            let verdict = params.get("verdict").and_then(|v| v.as_str()).map(String::from);
            let body = params.get("body").and_then(|v| v.as_str()).map(String::from);
            let path = state::draft_path(&ctx.slug, n).map_err(internal)?;
            let mut d = draft::load(&path, n).map_err(internal)?;
            if let Some(v) = verdict {
                d.verdict = Some(v);
            }
            if let Some(b) = body {
                d.body = b;
            }
            draft::save(&path, &d).map_err(internal)?;
            let pr = ctx.pr.lock().await.clone();
            let client = github::Client::new(ctx.token.clone());
            let result =
                github::review::submit(&client, &ctx.slug, &pr.node_id, &pr.head_sha, &d)
                    .await
                    .map_err(internal)?;
            let _ = std::fs::remove_file(&path);
            Ok(Some(json!({
                "github_review_id": result.get("id").cloned().unwrap_or(Value::Null),
                "errors": Value::Null,
                "result": result,
            })))
        }
        "index.status" => {
            let _ = pr_num()?;
            let s = ctx.indexer.status();
            Ok(Some(json!({
                "phase": s.phase,
                "progress": if s.files_total == 0 { 0.0 } else { s.files_done as f64 / s.files_total as f64 },
                "symbols_indexed": s.symbols_count,
                "errors": s.errors,
            })))
        }
        "index.call_graph" => {
            // TODO: refactor REST handler to a shared fn; for now return NotImplemented marker.
            Err((-32601, "index.call_graph not wired via RPC; use REST /api/pr/:n/call-graph".into()))
        }
        "index.blast_radius" => {
            Err((-32601, "index.blast_radius not wired via RPC; use REST /api/pr/:n/blast/:sym".into()))
        }
        "index.coverage" => {
            Err((-32601, "index.coverage not wired via RPC; use REST /api/pr/:n/coverage".into()))
        }
        "walkthrough.get" => {
            let n = pr_num()?;
            let pr = ctx.pr.lock().await.clone();
            let _ = n;
            let repo_state_dir = state::repo_dir(&ctx.slug).map_err(internal)?;
            let cp = crate::ai::cache_path(&repo_state_dir, pr.number, &pr.head_sha);
            match crate::ai::load_cache(&cp) {
                Some(c) => Ok(Some(serde_json::to_value(c).unwrap())),
                None => Ok(Some(Value::Null)),
            }
        }
        "walkthrough.generate" => {
            // TODO: streaming-over-RPC not wired; clients should hit REST SSE endpoint.
            Err((-32601, "walkthrough.generate not wired via RPC; use REST /api/pr/:n/walkthrough/generate".into()))
        }
        "review.subscribe" | "index.subscribe" | "github.subscribe" => {
            let n = pr_num()?;
            let reg = {
                let w = ctx.watchers.lock().await;
                w.get(&n).cloned()
            }
            .ok_or((-32003, "watcher not started".into()))?;
            let method = method.to_string();
            let mut rx = reg.tx.subscribe();
            let tx_clone = tx.clone();
            let h = tokio::spawn(async move {
                while let Ok(ev) = rx.recv().await {
                    let notif_method = match &method[..] {
                        "review.subscribe" => "review.event",
                        "index.subscribe" => "index.event",
                        _ => "github.event",
                    };
                    let v = serde_json::to_value(ev).unwrap_or(Value::Null);
                    let _ = tx_clone.send(Outbound::Frame(notif_frame(
                        notif_method,
                        json!({"pr_number": n, "event": v}),
                    )));
                }
            });
            subs.lock().await.push(h);
            Ok(Some(json!({ "subscribed": true, "pr_number": n })))
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

fn internal<E: std::fmt::Display>(e: E) -> (i64, String) {
    (-32000, e.to_string())
}

// Mirrors server::api::post_open_pr, since that function is private to that
// module. Keeps a single source of truth for the session-open primitives
// (github fetch, worktree ensure, indexer spawn, watcher spawn).
async fn open_pr(ctx: &ServerCtx, pr_number: u64) -> Result<Value, (i64, String)> {
    {
        let prs = ctx.prs.lock().await;
        if let Some(existing) = prs.iter().find(|p| p.number == pr_number) {
            return Ok(json!({
                "session_id": format!("{}-{}", ctx.slug, pr_number),
                "pr_data": existing,
                "already": true,
            }));
        }
    }
    let client = github::Client::new(ctx.token.clone());
    let meta = client.fetch_pr(&ctx.slug, pr_number).await.map_err(internal)?;
    worktree::fetch_pr_ref(&ctx.repo.root, pr_number).map_err(internal)?;
    let wt = state::worktree_path(&ctx.slug, pr_number).map_err(internal)?;
    worktree::ensure_worktree(&ctx.repo.root, &wt, pr_number).map_err(internal)?;
    let draft_path = state::draft_path(&ctx.slug, pr_number).map_err(internal)?;
    if !draft_path.exists() {
        let d = draft::load(&draft_path, pr_number).map_err(internal)?;
        draft::save(&draft_path, &d).map_err(internal)?;
    }
    ctx.prs.lock().await.push(meta.clone());
    ctx.write_session().await.ok();

    let indexer = ctx.indexer.clone();
    let slug_str = format!("{}", ctx.slug);
    let worktree_c = wt.clone();
    let repo_root = ctx.repo.root.clone();
    let meta_clone = meta.clone();
    tokio::task::spawn_blocking(move || {
        let mb = worktree::merge_base(&repo_root, &meta_clone.base_sha, &meta_clone.head_sha).ok();
        let _ = indexer.record_pr(
            &slug_str, meta_clone.number, &meta_clone.head_sha, &meta_clone.base_sha,
            mb.as_deref(), Some(&meta_clone.title), meta_clone.body.as_deref(), meta_clone.author.as_deref(),
        );
        let _ = indexer.index_tree(&slug_str, &meta_clone.base_sha, &worktree_c);
        let _ = indexer.index_tree(&slug_str, &meta_clone.head_sha, &worktree_c);
    });

    {
        let mut watchers = ctx.watchers.lock().await;
        if !watchers.contains_key(&pr_number) {
            let reg = Arc::new(WatchRegistry::new());
            {
                let mut s = reg.status.write().await;
                s.head_sha = meta.head_sha.clone();
                s.pr_state = meta.state.clone();
            }
            let deps = WatcherDeps {
                token: ctx.token.clone(),
                slug: ctx.slug.clone(),
                pr_number: pr_number as u32,
                repo_root: ctx.repo.root.clone(),
                worktree: wt.clone(),
                indexer: ctx.indexer.clone(),
                tx: reg.tx.clone(),
                status: reg.status.clone(),
            };
            let w = PrWatcher::spawn(deps);
            *reg.watcher.lock().await = Some(w);
            watchers.insert(pr_number, reg);
        }
    }

    Ok(json!({
        "session_id": format!("{}-{}", ctx.slug, pr_number),
        "pr_data": meta,
    }))
}
