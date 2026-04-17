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

// TODO(future): call graph traversal
pub struct CallGraph;
// TODO(future): blast radius computation
pub struct BlastRadius;
// TODO(future): coverage overlay ingest
pub struct Coverage;
// TODO(future): AI walkthrough generation
pub struct Walkthrough;
