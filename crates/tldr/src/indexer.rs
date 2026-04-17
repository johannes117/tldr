use anyhow::{anyhow, Context, Result};
use rayon::prelude::*;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: Option<i64>,
    pub repo_id: i64,
    pub commit_sha: String,
    pub file_path: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub start_line: u32,
    pub end_line: u32,
    pub start_col: u32,
    pub end_col: u32,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexStats {
    pub files: usize,
    pub symbols: usize,
    pub elapsed_ms: u128,
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexStatus {
    pub phase: String,
    pub files_done: usize,
    pub files_total: usize,
    pub symbols_count: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PhaseEvent {
    pub phase: String,
    pub files_done: usize,
    pub files_total: usize,
    pub symbols_count: usize,
    pub errors: usize,
    pub detail: String,
}

pub trait Language: Send + Sync {
    fn name(&self) -> &str;
    fn extensions(&self) -> &[&str];
    fn ts_language(&self) -> tree_sitter::Language;
    fn query(&self) -> &tree_sitter::Query;
}

macro_rules! lang_impl {
    ($ty:ident, $name:expr, $exts:expr, $ts_fn:expr, $query_src:expr) => {
        pub struct $ty {
            lang: tree_sitter::Language,
            query: tree_sitter::Query,
        }
        impl $ty {
            pub fn try_new() -> Option<Self> {
                let lang: tree_sitter::Language = $ts_fn;
                let query = match tree_sitter::Query::new(&lang, $query_src) {
                    Ok(q) => q,
                    Err(e) => {
                        tracing::warn!(lang = $name, error = %e, "tree-sitter query compile failed");
                        return None;
                    }
                };
                Some(Self { lang, query })
            }
        }
        impl Language for $ty {
            fn name(&self) -> &str { $name }
            fn extensions(&self) -> &[&str] { $exts }
            fn ts_language(&self) -> tree_sitter::Language { self.lang.clone() }
            fn query(&self) -> &tree_sitter::Query { &self.query }
        }
    };
}

const TS_QUERY: &str = r#"
(function_declaration name: (identifier) @name) @func
(class_declaration name: (type_identifier) @name) @class
(method_definition name: (property_identifier) @name) @method
(interface_declaration name: (type_identifier) @name) @interface
(type_alias_declaration name: (type_identifier) @name) @type
(enum_declaration name: (identifier) @name) @enum
(lexical_declaration (variable_declarator name: (identifier) @name)) @const
"#;

const JS_QUERY: &str = r#"
(function_declaration name: (identifier) @name) @func
(class_declaration name: (identifier) @name) @class
(method_definition name: (property_identifier) @name) @method
(lexical_declaration (variable_declarator name: (identifier) @name)) @const
"#;

const PY_QUERY: &str = r#"
(function_definition name: (identifier) @name) @func
(class_definition name: (identifier) @name) @class
(assignment left: (identifier) @name) @const
"#;

const GO_QUERY: &str = r#"
(function_declaration name: (identifier) @name) @func
(method_declaration name: (field_identifier) @name) @method
(type_declaration (type_spec name: (type_identifier) @name)) @type
(const_declaration (const_spec name: (identifier) @name)) @const
(var_declaration (var_spec name: (identifier) @name)) @var
"#;

const RUST_QUERY: &str = r#"
(function_item name: (identifier) @name) @func
(struct_item name: (type_identifier) @name) @struct
(enum_item name: (type_identifier) @name) @enum
(trait_item name: (type_identifier) @name) @trait
(impl_item type: (type_identifier) @name) @impl
(type_item name: (type_identifier) @name) @type
(const_item name: (identifier) @name) @const
(static_item name: (identifier) @name) @static
"#;

lang_impl!(TsLang, "typescript", &["ts", "tsx"], tree_sitter_typescript::language_tsx(), TS_QUERY);
lang_impl!(JsLang, "javascript", &["js", "jsx", "mjs", "cjs"], tree_sitter_javascript::language(), JS_QUERY);
lang_impl!(PyLang, "python", &["py"], tree_sitter_python::language(), PY_QUERY);
lang_impl!(GoLang, "go", &["go"], tree_sitter_go::language(), GO_QUERY);
lang_impl!(RustLang, "rust", &["rs"], tree_sitter_rust::language(), RUST_QUERY);

fn load_languages() -> Vec<Box<dyn Language>> {
    let mut v: Vec<Box<dyn Language>> = Vec::new();
    if let Some(l) = TsLang::try_new() { v.push(Box::new(l)); } else { tracing::warn!("ts grammar unavailable"); }
    if let Some(l) = JsLang::try_new() { v.push(Box::new(l)); } else { tracing::warn!("js grammar unavailable"); }
    if let Some(l) = PyLang::try_new() { v.push(Box::new(l)); } else { tracing::warn!("py grammar unavailable"); }
    if let Some(l) = GoLang::try_new() { v.push(Box::new(l)); } else { tracing::warn!("go grammar unavailable"); }
    if let Some(l) = RustLang::try_new() { v.push(Box::new(l)); } else { tracing::warn!("rust grammar unavailable"); }
    v
}

fn pick_language<'a>(langs: &'a [Box<dyn Language>], path: &Path) -> Option<&'a dyn Language> {
    let ext = path.extension()?.to_str()?;
    for l in langs {
        if l.extensions().iter().any(|e| *e == ext) {
            return Some(l.as_ref());
        }
    }
    None
}

pub struct Indexer {
    db: Mutex<Connection>,
    #[allow(dead_code)]
    state_dir: PathBuf,
    tx: broadcast::Sender<PhaseEvent>,
    status: Arc<Mutex<IndexStatus>>,
}

const MIGRATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS repos (
    id INTEGER PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS symbols (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL,
    commit_sha TEXT NOT NULL,
    file_path TEXT NOT NULL,
    name TEXT NOT NULL,
    qualified_name TEXT NOT NULL,
    kind TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_col INTEGER NOT NULL,
    end_col INTEGER NOT NULL,
    signature TEXT,
    UNIQUE(repo_id, commit_sha, qualified_name, file_path)
);
CREATE INDEX IF NOT EXISTS idx_symbols_repo_commit_path
    ON symbols(repo_id, commit_sha, file_path);
CREATE TABLE IF NOT EXISTS refs (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL,
    commit_sha TEXT NOT NULL,
    from_symbol_id INTEGER,
    to_symbol_id INTEGER,
    kind TEXT NOT NULL,
    site_file TEXT,
    site_line INTEGER,
    site_text TEXT
);
CREATE INDEX IF NOT EXISTS idx_refs_from ON refs(from_symbol_id);
CREATE INDEX IF NOT EXISTS idx_refs_to ON refs(to_symbol_id);
CREATE TABLE IF NOT EXISTS blame (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL,
    commit_sha TEXT NOT NULL,
    file_path TEXT NOT NULL,
    line INTEGER NOT NULL,
    author TEXT,
    author_commit TEXT,
    author_time TEXT
);
CREATE TABLE IF NOT EXISTS prs (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL,
    number INTEGER NOT NULL,
    head_sha TEXT NOT NULL,
    base_sha TEXT NOT NULL,
    merge_base_sha TEXT,
    title TEXT,
    body TEXT,
    author TEXT,
    UNIQUE(repo_id, number, head_sha)
);
"#;

impl Indexer {
    pub fn open(repo_slug: &str) -> Result<Self> {
        let state = crate::state::state_root()?;
        let dir = state.join("repos").join(repo_slug.replace('/', "__"));
        std::fs::create_dir_all(&dir).ok();
        let db_path = dir.join("index.db");
        let conn = Connection::open(&db_path).with_context(|| format!("open {}", db_path.display()))?;
        conn.execute_batch(MIGRATIONS)?;
        conn.execute(
            "INSERT OR IGNORE INTO repos(slug, created_at) VALUES(?1, ?2)",
            params![repo_slug, chrono::Utc::now().to_rfc3339()],
        )?;
        let (tx, _rx) = broadcast::channel(256);
        Ok(Self {
            db: Mutex::new(conn),
            state_dir: dir,
            tx,
            status: Arc::new(Mutex::new(IndexStatus {
                phase: "idle".into(),
                files_done: 0,
                files_total: 0,
                symbols_count: 0,
                errors: 0,
            })),
        })
    }

    pub fn phase_events(&self) -> broadcast::Receiver<PhaseEvent> { self.tx.subscribe() }

    pub fn status(&self) -> IndexStatus { self.status.lock().unwrap().clone() }

    pub fn record_pr(&self, repo_slug: &str, number: u64, head_sha: &str, base_sha: &str, merge_base: Option<&str>, title: Option<&str>, body: Option<&str>, author: Option<&str>) -> Result<()> {
        let conn = self.db.lock().unwrap();
        let repo_id = repo_id(&conn, repo_slug)?;
        conn.execute(
            "INSERT OR REPLACE INTO prs(repo_id, number, head_sha, base_sha, merge_base_sha, title, body, author) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![repo_id, number as i64, head_sha, base_sha, merge_base, title, body, author],
        )?;
        Ok(())
    }

    pub fn index_tree(&self, repo_slug: &str, commit_sha: &str, worktree_path: &Path) -> Result<IndexStats> {
        let start = Instant::now();
        let langs = Arc::new(load_languages());

        self.emit_phase("discovering", 0, 0, 0, 0, "walking tree");

        let mut files: Vec<PathBuf> = Vec::new();
        let walker = ignore::WalkBuilder::new(worktree_path).hidden(false).build();
        for entry in walker.flatten() {
            let p = entry.path();
            if !p.is_file() { continue; }
            if pick_language(&langs, p).is_some() {
                files.push(p.to_path_buf());
            }
        }
        let total = files.len();
        self.emit_phase("parsing", 0, total, 0, 0, "starting parse");

        let symbols_count = Arc::new(AtomicUsize::new(0));
        let files_done = Arc::new(AtomicUsize::new(0));
        let errors = Arc::new(AtomicUsize::new(0));

        let repo_id_val = {
            let conn = self.db.lock().unwrap();
            repo_id(&conn, repo_slug)?
        };

        let worktree_path = worktree_path.to_path_buf();
        let results: Vec<Vec<Symbol>> = files.par_iter().map(|path| {
            let rel = path.strip_prefix(&worktree_path).unwrap_or(path).to_string_lossy().to_string();
            let lang = match pick_language(&langs, path) { Some(l) => l, None => return Vec::new() };
            let src = match std::fs::read(path) { Ok(b) => b, Err(_) => { errors.fetch_add(1, Ordering::Relaxed); return Vec::new(); } };
            let mut parser = tree_sitter::Parser::new();
            if parser.set_language(&lang.ts_language()).is_err() {
                errors.fetch_add(1, Ordering::Relaxed);
                return Vec::new();
            }
            let tree = match parser.parse(&src, None) {
                Some(t) => t,
                None => { errors.fetch_add(1, Ordering::Relaxed); return Vec::new(); }
            };
            let syms = extract_symbols(&tree, &src, lang, repo_id_val, commit_sha, &rel);
            let done = files_done.fetch_add(1, Ordering::Relaxed) + 1;
            symbols_count.fetch_add(syms.len(), Ordering::Relaxed);
            if done % 64 == 0 {
                tracing::info!(target: "index.phase", phase = "parsing", files_done = done, files_total = total, "progress");
            }
            syms
        }).collect();

        self.emit_phase("writing", files_done.load(Ordering::Relaxed), total, symbols_count.load(Ordering::Relaxed), errors.load(Ordering::Relaxed), "persisting");

        {
            let mut conn = self.db.lock().unwrap();
            let tx = conn.transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT OR IGNORE INTO symbols(repo_id, commit_sha, file_path, name, qualified_name, kind, start_line, end_line, start_col, end_col, signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)"
                )?;
                for batch in &results {
                    for s in batch {
                        stmt.execute(params![
                            s.repo_id, s.commit_sha, s.file_path, s.name, s.qualified_name, s.kind,
                            s.start_line, s.end_line, s.start_col, s.end_col, s.signature,
                        ])?;
                    }
                }
            }
            tx.commit()?;
        }

        let stats = IndexStats {
            files: total,
            symbols: symbols_count.load(Ordering::Relaxed),
            elapsed_ms: start.elapsed().as_millis(),
            errors: errors.load(Ordering::Relaxed),
        };
        self.emit_phase("done", total, total, stats.symbols, stats.errors, "complete");
        tracing::info!(target: "index.phase", phase = "done", files = stats.files, symbols = stats.symbols, elapsed_ms = stats.elapsed_ms as u64, errors = stats.errors, "index complete");
        Ok(stats)
    }

    fn emit_phase(&self, phase: &str, files_done: usize, files_total: usize, symbols_count: usize, errors: usize, detail: &str) {
        let ev = PhaseEvent {
            phase: phase.into(), files_done, files_total, symbols_count, errors,
            detail: detail.into(),
        };
        {
            let mut s = self.status.lock().unwrap();
            s.phase = ev.phase.clone();
            s.files_done = files_done;
            s.files_total = files_total;
            s.symbols_count = symbols_count;
            s.errors = errors;
        }
        let _ = self.tx.send(ev);
        tracing::info!(target: "index.phase", phase = phase, files_done = files_done, files_total = files_total, detail = detail, "phase");
    }

    pub fn symbols_for_file(&self, repo_slug: &str, commit_sha: &str, path: &str) -> Result<Vec<Symbol>> {
        let conn = self.db.lock().unwrap();
        let repo_id_val = repo_id(&conn, repo_slug)?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_id, commit_sha, file_path, name, qualified_name, kind, start_line, end_line, start_col, end_col, signature FROM symbols WHERE repo_id=?1 AND commit_sha=?2 AND file_path=?3 ORDER BY start_line"
        )?;
        let rows = stmt.query_map(params![repo_id_val, commit_sha, path], |r| {
            Ok(Symbol {
                id: r.get(0)?,
                repo_id: r.get(1)?,
                commit_sha: r.get(2)?,
                file_path: r.get(3)?,
                name: r.get(4)?,
                qualified_name: r.get(5)?,
                kind: r.get(6)?,
                start_line: r.get(7)?,
                end_line: r.get(8)?,
                start_col: r.get(9)?,
                end_col: r.get(10)?,
                signature: r.get(11)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn symbol_by_qualified_name(&self, repo_slug: &str, commit_sha: &str, qname: &str) -> Result<Option<Symbol>> {
        let conn = self.db.lock().unwrap();
        let repo_id_val = repo_id(&conn, repo_slug)?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_id, commit_sha, file_path, name, qualified_name, kind, start_line, end_line, start_col, end_col, signature FROM symbols WHERE repo_id=?1 AND commit_sha=?2 AND qualified_name=?3 LIMIT 1"
        )?;
        let mut rows = stmt.query_map(params![repo_id_val, commit_sha, qname], |r| {
            Ok(Symbol {
                id: r.get(0)?, repo_id: r.get(1)?, commit_sha: r.get(2)?, file_path: r.get(3)?,
                name: r.get(4)?, qualified_name: r.get(5)?, kind: r.get(6)?,
                start_line: r.get(7)?, end_line: r.get(8)?, start_col: r.get(9)?, end_col: r.get(10)?,
                signature: r.get(11)?,
            })
        })?;
        Ok(rows.next().and_then(|r| r.ok()))
    }

    /// Return outgoing call edges (kind='call') joined to symbol qualified_names.
    pub fn call_edges(&self, repo_slug: &str, commit_sha: &str) -> Result<Vec<CallEdgeRow>> {
        let conn = self.db.lock().unwrap();
        let repo_id_val = repo_id(&conn, repo_slug)?;
        let mut stmt = conn.prepare(
            "SELECT sf.qualified_name, st.qualified_name, r.site_file, r.site_line, COALESCE(r.site_text,'')
             FROM refs r
             JOIN symbols sf ON sf.id = r.from_symbol_id
             JOIN symbols st ON st.id = r.to_symbol_id
             WHERE r.repo_id=?1 AND r.commit_sha=?2 AND r.kind='call'"
        )?;
        let rows = stmt.query_map(params![repo_id_val, commit_sha], |r| {
            Ok(CallEdgeRow {
                from: r.get(0)?, to: r.get(1)?,
                path: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                line: r.get::<_, Option<u32>>(3)?.unwrap_or(0),
                site_text: r.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

#[derive(Debug, Clone)]
pub struct CallEdgeRow {
    pub from: String,
    pub to: String,
    pub path: String,
    pub line: u32,
    pub site_text: String,
}

fn repo_id(conn: &Connection, slug: &str) -> Result<i64> {
    conn.query_row("SELECT id FROM repos WHERE slug=?1", params![slug], |r| r.get(0))
        .map_err(|e| anyhow!("repo not found in db: {e}"))
}

fn extract_symbols(
    tree: &tree_sitter::Tree,
    src: &[u8],
    lang: &dyn Language,
    repo_id: i64,
    commit_sha: &str,
    file_path: &str,
) -> Vec<Symbol> {
    let mut out = Vec::new();
    let mut cursor = tree_sitter::QueryCursor::new();
    let query = lang.query();
    let name_idx: Vec<u32> = query.capture_names().iter().enumerate()
        .filter(|(_, n)| *n == &"name")
        .map(|(i, _)| i as u32)
        .collect();

    let matches = cursor.matches(query, tree.root_node(), src);
    for m in matches {
        let pattern = m.pattern_index;
        let kind_name = query.capture_names().iter().enumerate()
            .filter(|(_, n)| *n != &"name")
            .map(|(_, n)| *n)
            .next()
            .unwrap_or("symbol");

        let mut name: Option<String> = None;
        let mut outer: Option<tree_sitter::Node> = None;
        for cap in m.captures {
            if name_idx.contains(&cap.index) {
                if let Ok(s) = cap.node.utf8_text(src) { name = Some(s.to_string()); }
            } else {
                outer = Some(cap.node);
            }
        }
        let name = match name { Some(n) => n, None => continue };
        let node = outer.unwrap_or(m.captures[0].node);
        let start = node.start_position();
        let end = node.end_position();

        let kind = query.capture_names().get(
            m.captures.iter().find(|c| !name_idx.contains(&c.index)).map(|c| c.index).unwrap_or(0) as usize
        ).copied().unwrap_or(kind_name).to_string();

        let qualified_name = format!("{}:{}:{}", file_path, name, start.row + 1);
        out.push(Symbol {
            id: None,
            repo_id,
            commit_sha: commit_sha.to_string(),
            file_path: file_path.to_string(),
            name,
            qualified_name,
            kind,
            start_line: (start.row + 1) as u32,
            end_line: (end.row + 1) as u32,
            start_col: start.column as u32,
            end_col: end.column as u32,
            signature: None,
        });
        let _ = pattern;
    }
    out
}

/// Structured phase log (kept for callers).
pub fn log_phase(phase: &str, detail: &str) {
    tracing::info!(target: "index.phase", phase = phase, detail = detail, "index.phase");
}

pub fn log_lsp_spawn(binary: &str, pid: Option<u32>) {
    tracing::info!(binary = binary, pid = pid, "lsp.spawn");
}
pub fn log_lsp_exit(binary: &str, code: Option<i32>) {
    tracing::info!(binary = binary, code = code, "lsp.exit");
}

pub struct CallGraph;
pub struct BlastRadius;
pub struct Coverage;
pub struct Walkthrough;
