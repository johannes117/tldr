use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{sse::{Event, KeepAlive, Sse}, IntoResponse},
    Json,
};
use futures::stream::StreamExt;
use serde_json::json;
use std::convert::Infallible;

use super::ServerCtx;
use crate::{ai, config, diff, state, worktree};

pub async fn get_walkthrough(
    State(ctx): State<ServerCtx>,
    Path(n): Path<u64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if let Some(disabled) = disabled_response(&ctx) {
        return Ok(Json(disabled));
    }
    let pr = ctx.pr.lock().await.clone();
    let _ = n;
    let repo_state_dir = state::repo_dir(&ctx.slug).map_err(err)?;
    let cp = ai::cache_path(&repo_state_dir, pr.number, &pr.head_sha);
    if let Some(cached) = ai::load_cache(&cp) {
        return Ok(Json(json!({"status": "ready", "walkthrough": cached})));
    }
    Ok(Json(json!({"status": "none"})))
}

pub async fn get_usage(
    State(ctx): State<ServerCtx>,
    Path(_n): Path<u64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if let Some(disabled) = disabled_response(&ctx) {
        return Ok(Json(disabled));
    }
    let pr = ctx.pr.lock().await.clone();
    let repo_state_dir = state::repo_dir(&ctx.slug).map_err(err)?;
    let cp = ai::cache_path(&repo_state_dir, pr.number, &pr.head_sha);
    match ai::load_cache(&cp) {
        Some(c) => Ok(Json(json!({"status": "ready", "usage": c.usage}))),
        None => Ok(Json(json!({"status": "none"}))),
    }
}

pub async fn post_generate(
    State(ctx): State<ServerCtx>,
    Path(_n): Path<u64>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if let Some(disabled) = disabled_response(&ctx) {
        return Ok(Json(disabled).into_response());
    }
    let cfg = config::Config::load().map_err(err)?;
    if !cfg.ai.enabled {
        return Ok(Json(json!({"enabled": false, "reason": "disabled by global config"})).into_response());
    }
    let slug_str = format!("{}", ctx.slug);
    if !cfg.ai.confirmed.repos.iter().any(|r| r == &slug_str) {
        return Ok(Json(json!({"status": "unconfirmed", "reason": "privacy notice not accepted"})).into_response());
    }

    let pr = ctx.pr.lock().await.clone();
    let base = worktree::merge_base(&ctx.repo.root, &pr.base_sha, &pr.head_sha).map_err(err)?;
    let d = diff::compute(&ctx.worktree, &base, &pr.head_sha).map_err(err)?;
    let diff_text = render_diff_text(&d, cfg.ai.context_kb * 1024);
    let pr_desc = pr.body.clone().unwrap_or_default();
    let context = String::new();

    let provider = ai::build_provider(&cfg.ai).map_err(err)?;
    let mut event_stream = provider.generate_walkthrough(&diff_text, &pr_desc, &context).await.map_err(err)?;

    let repo_state_dir = state::repo_dir(&ctx.slug).map_err(err)?;
    let cache_path = ai::cache_path(&repo_state_dir, pr.number, &pr.head_sha);
    let pr_number = pr.number;
    let head_sha = pr.head_sha.clone();

    let sse = async_stream::stream! {
        let mut acc = String::new();
        let mut usage = ai::Usage::default();
        let mut emitted_ids: std::collections::HashSet<String> = Default::default();
        while let Some(ev) = event_stream.next().await {
            match ev {
                Ok(ai::ProviderEvent::Text(t)) => {
                    acc.push_str(&t);
                    if let Some(steps) = try_parse_steps(&acc) {
                        for step in steps {
                            if !emitted_ids.contains(&step.id) {
                                emitted_ids.insert(step.id.clone());
                                let payload = serde_json::to_string(&step).unwrap_or_default();
                                yield Ok::<_, Infallible>(Event::default().event("step").data(payload));
                            }
                        }
                    }
                }
                Ok(ai::ProviderEvent::Usage(u)) => { usage = u; }
                Err(e) => {
                    yield Ok(Event::default().event("error").data(e.to_string()));
                }
            }
        }
        let steps = try_parse_steps(&acc).unwrap_or_default();
        let cached = ai::CachedWalkthrough {
            pr_number,
            head_sha: head_sha.clone(),
            steps: steps.clone(),
            usage: usage.clone(),
            generated_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(e) = ai::save_cache(&cache_path, &cached) {
            tracing::warn!(error = %e, "walkthrough cache save failed");
        }
        tracing::info!(
            provider = %usage.provider, model = %usage.model,
            input_tokens = usage.input_tokens, output_tokens = usage.output_tokens,
            "walkthrough.usage"
        );
        yield Ok(Event::default().event("done").data(serde_json::to_string(&cached).unwrap_or_default()));
    };

    Ok(Sse::new(sse).keep_alive(KeepAlive::default()).into_response())
}

pub async fn post_confirm(
    State(ctx): State<ServerCtx>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut cfg = config::Config::load().map_err(err)?;
    let slug_str = format!("{}", ctx.slug);
    if !cfg.ai.confirmed.repos.iter().any(|r| r == &slug_str) {
        cfg.ai.confirmed.repos.push(slug_str);
        cfg.save().map_err(err)?;
    }
    Ok(Json(json!({"ok": true})))
}

pub async fn get_status(
    State(ctx): State<ServerCtx>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let cfg = config::Config::load().map_err(err)?;
    let repo_ai = ai::load_repo_ai_config(&ctx.repo.root);
    let slug_str = format!("{}", ctx.slug);
    let confirmed = cfg.ai.confirmed.repos.iter().any(|r| r == &slug_str);
    if let Some(disabled) = disabled_response(&ctx) {
        return Ok(Json(disabled));
    }
    Ok(Json(json!({
        "enabled": cfg.ai.enabled,
        "repo_enabled": repo_ai.enabled,
        "provider": cfg.ai.provider,
        "model": cfg.ai.model,
        "context_kb": cfg.ai.context_kb,
        "confirmed": confirmed,
    })))
}

fn disabled_response(ctx: &ServerCtx) -> Option<serde_json::Value> {
    let repo_ai = ai::load_repo_ai_config(&ctx.repo.root);
    if repo_ai.enabled == Some(false) {
        return Some(json!({"enabled": false, "reason": "disabled by repo config"}));
    }
    None
}

fn err<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn render_diff_text(d: &diff::Diff, max_bytes: usize) -> String {
    let mut out = String::new();
    for f in &d.files {
        out.push_str(&format!("--- {}\n+++ {}\n", f.old_path.clone().unwrap_or_else(|| f.path.clone()), f.path));
        for h in &f.hunks {
            out.push_str(&format!("@@ {} @@\n", h.header));
            for l in &h.lines {
                let prefix = match l.kind.as_str() { "add" => "+", "del" => "-", _ => " " };
                out.push_str(prefix);
                out.push_str(&l.content);
                out.push('\n');
                if out.len() > max_bytes { return out; }
            }
        }
    }
    out
}

fn try_parse_steps(raw: &str) -> Option<Vec<ai::WalkthroughStep>> {
    let s = raw.trim();
    let s = s.trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    if !s.starts_with('[') { return None; }
    serde_json::from_str::<Vec<ai::WalkthroughStep>>(s).ok()
}
