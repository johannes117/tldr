pub mod api;
pub mod assets;

use anyhow::Result;
use axum::Router;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::github::PrMeta;
use crate::repo::{Repo, Slug};

#[derive(Clone)]
pub struct ServerCtx {
    pub repo: Repo,
    pub slug: Slug,
    pub worktree: PathBuf,
    pub pr: Arc<Mutex<PrMeta>>,
    pub token: String,
}

impl ServerCtx {
    pub fn new(repo: Repo, slug: Slug, worktree: PathBuf, pr: PrMeta, token: String) -> Self {
        Self { repo, slug, worktree, pr: Arc::new(Mutex::new(pr)), token }
    }
}

pub fn pick_port() -> u16 {
    use std::net::TcpListener;
    for p in 47800..47900u16 {
        if TcpListener::bind(("127.0.0.1", p)).is_ok() { return p; }
    }
    47800
}

pub async fn serve(addr: String, ctx: ServerCtx) -> Result<()> {
    let app = Router::new()
        .nest("/api", api::router())
        .fallback(assets::handler)
        .with_state(ctx);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
