// TODO(future): tree-sitter + LSP-backed symbol index, call graph, blast radius.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SymbolTable {
    pub symbols: Vec<Symbol>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub path: String,
    pub line: u32,
}

pub trait Language: Send + Sync {
    fn name(&self) -> &'static str;
    fn extensions(&self) -> &'static [&'static str];
    fn index(&self, _root: &std::path::Path) -> SymbolTable { SymbolTable::default() }
}

pub struct NoopLang;
impl Language for NoopLang {
    fn name(&self) -> &'static str { "noop" }
    fn extensions(&self) -> &'static [&'static str] { &[] }
}

/// Emit a structured log event for index phase transitions.
pub fn log_phase(phase: &str, detail: &str) {
    tracing::info!(phase = phase, detail = detail, "index.phase");
}

/// Stub LSP lifecycle logging hooks (real LSP integration lands later).
pub fn log_lsp_spawn(binary: &str, pid: Option<u32>) {
    tracing::info!(binary = binary, pid = pid, "lsp.spawn");
}
pub fn log_lsp_exit(binary: &str, code: Option<i32>) {
    tracing::info!(binary = binary, code = code, "lsp.exit");
}

// TODO(future): call graph traversal
pub struct CallGraph;
// TODO(future): blast radius computation
pub struct BlastRadius;
// TODO(future): coverage overlay ingest
pub struct Coverage;
// TODO(future): AI walkthrough generation
pub struct Walkthrough;
