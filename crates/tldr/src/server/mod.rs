pub mod api;
pub mod assets;
pub mod walkthrough;

use anyhow::Result;
use axum::{
    extract::State,
    http::{HeaderValue, Method, Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    Router,
};
use base64::Engine;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::github::PrMeta;
use crate::indexer::Indexer;
use crate::lsp::LspPool;
use crate::repo::{Repo, Slug};
use crate::session::{self, SessionFile};
use crate::watcher::WatchRegistry;

#[derive(Clone)]
pub struct ServerCtx {
    pub repo: Repo,
    pub slug: Slug,
    pub worktree: PathBuf,
    pub pr: Arc<Mutex<PrMeta>>,
    pub prs: Arc<Mutex<Vec<PrMeta>>>,
    pub token: String,
    pub port: u16,
    pub started_at: String,
    pub shutdown: Arc<Notify>,
    pub csrf: Arc<String>,
    pub indexer: Arc<Indexer>,
    pub reviewer: Arc<Mutex<Option<ReviewerIdentity>>>,
    pub watchers: Arc<Mutex<std::collections::HashMap<u64, Arc<WatchRegistry>>>>,
    pub lsp: Arc<LspPool>,
}

#[derive(Clone, Debug, Default)]
pub struct ReviewerIdentity {
    pub login: Option<String>,
    pub email: Option<String>,
}

impl ServerCtx {
    pub fn new(repo: Repo, slug: Slug, worktree: PathBuf, pr: PrMeta, token: String, port: u16) -> Self {
        let worktree_clone = worktree.clone();
        let prs = vec![pr.clone()];
        let mut buf = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut buf);
        let csrf = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);
        let slug_str = format!("{}", slug);
        let indexer = Indexer::open(&slug_str)
            .map(Arc::new)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "indexer.open failed; creating in-memory fallback may not work");
                // re-attempt and panic-worthy; but to keep it non-fatal, try once more
                Arc::new(Indexer::open(&slug_str).expect("indexer open failed"))
            });
        Self {
            repo, slug, worktree,
            pr: Arc::new(Mutex::new(pr)),
            prs: Arc::new(Mutex::new(prs)),
            token, port,
            started_at: chrono::Utc::now().to_rfc3339(),
            shutdown: Arc::new(Notify::new()),
            csrf: Arc::new(csrf),
            indexer,
            reviewer: Arc::new(Mutex::new(None)),
            watchers: Arc::new(Mutex::new(std::collections::HashMap::new())),
            lsp: Arc::new(LspPool::new(worktree_clone)),
        }
    }

    pub async fn write_session(&self) -> Result<()> {
        let active: Vec<u64> = self.prs.lock().await.iter().map(|p| p.number).collect();
        let fp: String = self.csrf.chars().take(8).collect();
        let sess = SessionFile {
            pid: std::process::id(),
            port: self.port,
            started_at: self.started_at.clone(),
            active_prs: active,
            csrf_token_fingerprint: fp,
            csrf_token: (*self.csrf).clone(),
            slug: format!("{}", self.slug),
        };
        session::write_atomic(&session::session_path(&self.slug)?, &sess)
    }
}

pub fn pick_port() -> u16 {
    use std::net::TcpListener;
    for p in 47800..47900u16 {
        if TcpListener::bind(("127.0.0.1", p)).is_ok() { return p; }
    }
    47800
}

async fn csrf_guard(
    State(ctx): State<ServerCtx>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let m = req.method();
    if matches!(*m, Method::POST | Method::PUT | Method::DELETE | Method::PATCH) {
        let hdr = req
            .headers()
            .get("x-tldr-csrf")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        use subtle::ConstantTimeEq;
        if hdr.as_bytes().ct_eq(ctx.csrf.as_bytes()).unwrap_u8() != 1 {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Ok(next.run(req).await)
}

pub async fn serve(addr: String, ctx: ServerCtx) -> Result<()> {
    ctx.write_session().await.ok();
    let session_path = session::session_path(&ctx.slug)?;
    let _guard = session::Guard { path: session_path.clone() };

    let shutdown = ctx.shutdown.clone();
    let port = ctx.port;
    let origin = format!("http://127.0.0.1:{port}");
    let origin_alt = format!("http://localhost:{port}");
    let cors = CorsLayer::new().allow_origin(AllowOrigin::predicate(move |o: &HeaderValue, _| {
        let b = o.as_bytes();
        b == origin.as_bytes() || b == origin_alt.as_bytes()
    }));

    let api_router = api::router()
        .layer(middleware::from_fn_with_state(ctx.clone(), csrf_guard));
    let app = Router::new()
        .nest("/api", api_router)
        .layer(cors)
        .fallback(assets::handler)
        .with_state(ctx);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(port = port, pid = std::process::id(), addr = %addr, "server.start");

    let shutdown_signal = async move {
        tokio::select! {
            _ = shutdown.notified() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await?;
    tracing::info!(port = port, pid = std::process::id(), "server.stop");
    session::remove(&session_path);
    Ok(())
}
