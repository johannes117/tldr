//! Minimal async JSON-RPC LSP client pool.
//!
//! Spawns one language-server per language per repo. Implements the subset
//! of LSP used by the indexer / blast-radius / call-graph features.

use anyhow::{anyhow, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{oneshot, Mutex};

pub trait LspServer: Send + Sync {
    fn command(&self) -> &str;
    fn args(&self) -> Vec<String>;
    fn lang(&self) -> &str;
}

pub struct Tsserver;
impl LspServer for Tsserver {
    fn command(&self) -> &str { "typescript-language-server" }
    fn args(&self) -> Vec<String> { vec!["--stdio".into()] }
    fn lang(&self) -> &str { "typescript" }
}
pub struct Pyright;
impl LspServer for Pyright {
    fn command(&self) -> &str { "pyright-langserver" }
    fn args(&self) -> Vec<String> { vec!["--stdio".into()] }
    fn lang(&self) -> &str { "python" }
}
pub struct Gopls;
impl LspServer for Gopls {
    fn command(&self) -> &str { "gopls" }
    fn args(&self) -> Vec<String> { vec![] }
    fn lang(&self) -> &str { "go" }
}
pub struct RustAnalyzer;
impl LspServer for RustAnalyzer {
    fn command(&self) -> &str { "rust-analyzer" }
    fn args(&self) -> Vec<String> { vec![] }
    fn lang(&self) -> &str { "rust" }
}

pub fn all_servers() -> Vec<Box<dyn LspServer>> {
    vec![
        Box::new(Tsserver),
        Box::new(Pyright),
        Box::new(Gopls),
        Box::new(RustAnalyzer),
    ]
}

pub fn ext_to_lang(ext: &str) -> Option<&'static str> {
    match ext {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some("typescript"),
        "py" => Some("python"),
        "go" => Some("go"),
        "rs" => Some("rust"),
        _ => None,
    }
}

pub struct LspHandle {
    lang: String,
    #[allow(dead_code)]
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: AtomicI64,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
    restarts: u32,
    degraded: bool,
}

impl LspHandle {
    async fn spawn(server: &dyn LspServer, workspace: &Path) -> Result<Self> {
        let mut child = Command::new(server.command())
            .args(server.args())
            .current_dir(workspace)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("spawn {}", server.command()))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        crate::indexer::log_lsp_spawn(server.command(), child.id());

        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_r = pending.clone();
        tokio::spawn(async move {
            let _ = read_loop(stdout, pending_r).await;
        });

        let handle = Self {
            lang: server.lang().to_string(),
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            next_id: AtomicI64::new(1),
            pending,
            restarts: 0,
            degraded: false,
        };
        handle.initialize(workspace).await?;
        Ok(handle)
    }

    async fn initialize(&self, workspace: &Path) -> Result<()> {
        let uri = url::Url::from_file_path(workspace)
            .map_err(|_| anyhow!("workspace not absolute"))?;
        let params = json!({
            "processId": std::process::id(),
            "rootUri": uri.to_string(),
            "capabilities": {
                "textDocument": {
                    "callHierarchy": { "dynamicRegistration": false },
                    "references": {},
                    "definition": {},
                    "diagnostic": {},
                }
            },
            "workspaceFolders": [{ "uri": uri.to_string(), "name": "tldr" }],
        });
        let _: Value = self.request("initialize", params).await?;
        self.notify("initialized", json!({})).await?;
        Ok(())
    }

    pub async fn request<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        write_message(&self.stdin, &msg).await?;
        let resp = tokio::time::timeout(std::time::Duration::from_secs(15), rx)
            .await
            .map_err(|_| anyhow!("lsp timeout: {method}"))?
            .map_err(|_| anyhow!("lsp channel dropped"))?;
        if let Some(err) = resp.get("error") {
            return Err(anyhow!("lsp error: {err}"));
        }
        let result = resp.get("result").cloned().unwrap_or(Value::Null);
        Ok(serde_json::from_value(result)?)
    }

    pub async fn notify<P: Serialize>(&self, method: &str, params: P) -> Result<()> {
        let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        write_message(&self.stdin, &msg).await
    }

    pub async fn did_open(&self, path: &Path, text: &str) -> Result<()> {
        let uri = url::Url::from_file_path(path).map_err(|_| anyhow!("bad path"))?;
        let lang_id = match self.lang.as_str() {
            "typescript" => "typescript",
            "python" => "python",
            "go" => "go",
            "rust" => "rust",
            _ => "plaintext",
        };
        self.notify("textDocument/didOpen", json!({
            "textDocument": {
                "uri": uri.to_string(),
                "languageId": lang_id,
                "version": 1,
                "text": text,
            }
        })).await
    }
}

async fn write_message(stdin: &Arc<Mutex<ChildStdin>>, msg: &Value) -> Result<()> {
    let body = serde_json::to_vec(msg)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut guard = stdin.lock().await;
    guard.write_all(header.as_bytes()).await?;
    guard.write_all(&body).await?;
    guard.flush().await?;
    Ok(())
}

async fn read_loop(
    stdout: ChildStdout,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
) -> Result<()> {
    let mut reader = BufReader::new(stdout);
    loop {
        // Read headers
        let mut content_length: Option<usize> = None;
        let mut header_buf = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            if reader.read_exact(&mut byte).await.is_err() {
                return Ok(());
            }
            header_buf.push(byte[0]);
            if header_buf.ends_with(b"\r\n\r\n") {
                break;
            }
            if header_buf.len() > 8192 {
                return Err(anyhow!("header too large"));
            }
        }
        let header_str = String::from_utf8_lossy(&header_buf);
        for line in header_str.split("\r\n") {
            if let Some(v) = line.strip_prefix("Content-Length:") {
                content_length = v.trim().parse().ok();
            }
        }
        let len = content_length.ok_or_else(|| anyhow!("no content-length"))?;
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).await?;
        let v: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(id) = v.get("id").and_then(|i| i.as_i64()) {
            let mut p = pending.lock().await;
            if let Some(tx) = p.remove(&id) {
                let _ = tx.send(v);
            }
        }
        // Notifications (no id) are ignored.
    }
}

pub struct LspPool {
    workspace: PathBuf,
    servers: Mutex<HashMap<String, Arc<LspHandle>>>,
    degraded: Mutex<HashMap<String, bool>>,
}

impl LspPool {
    pub fn new(workspace: PathBuf) -> Self {
        Self {
            workspace,
            servers: Mutex::new(HashMap::new()),
            degraded: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get_or_spawn(&self, lang: &str) -> Option<Arc<LspHandle>> {
        if *self.degraded.lock().await.get(lang).unwrap_or(&false) {
            return None;
        }
        {
            let s = self.servers.lock().await;
            if let Some(h) = s.get(lang) {
                return Some(h.clone());
            }
        }
        let server: Box<dyn LspServer> = match lang {
            "typescript" => Box::new(Tsserver),
            "python" => Box::new(Pyright),
            "go" => Box::new(Gopls),
            "rust" => Box::new(RustAnalyzer),
            _ => return None,
        };
        if which::which(server.command()).is_err() {
            self.degraded.lock().await.insert(lang.into(), true);
            tracing::info!(lang = lang, cmd = server.command(), "lsp server binary not found; degraded");
            return None;
        }
        let mut attempt = 0u32;
        loop {
            match LspHandle::spawn(server.as_ref(), &self.workspace).await {
                Ok(h) => {
                    let arc = Arc::new(h);
                    self.servers.lock().await.insert(lang.into(), arc.clone());
                    return Some(arc);
                }
                Err(e) => {
                    attempt += 1;
                    tracing::warn!(lang = lang, attempt = attempt, error = %e, "lsp spawn failed");
                    if attempt >= 3 {
                        self.degraded.lock().await.insert(lang.into(), true);
                        return None;
                    }
                }
            }
        }
    }

    /// Run a best-effort call-hierarchy pass over `files` under workspace, inserting
    /// `refs` rows (kind="call") into the indexer. No-ops if server is unavailable.
    pub async fn enrich_refs(
        &self,
        _indexer: &crate::indexer::Indexer,
        _repo_slug: &str,
        _commit_sha: &str,
        _files: &[PathBuf],
    ) -> Result<()> {
        // Wiring stub: actual enrichment queries prepareCallHierarchy per symbol
        // which is expensive; v1 invokes on-demand from the call-graph API instead
        // of blanket indexing. See server::api::call_graph.
        Ok(())
    }
}

// Re-export lsp-types for callers.
pub use lsp_types;
