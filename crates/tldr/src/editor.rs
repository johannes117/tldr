use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Editor {
    VsCode,
    Cursor,
    Zed,
    JetBrains,
    Neovim,
    Custom { command: String, args_template: String },
}

impl Editor {
    pub fn command(&self) -> &str {
        match self {
            Editor::VsCode => "code",
            Editor::Cursor => "cursor",
            Editor::Zed => "zed",
            Editor::JetBrains => "idea",
            Editor::Neovim => "nvim",
            Editor::Custom { command, .. } => command.as_str(),
        }
    }

    pub fn args(&self, path: &Path, line: Option<u32>, col: Option<u32>) -> Vec<String> {
        let p = path.to_string_lossy().to_string();
        match self {
            Editor::VsCode | Editor::Cursor => {
                let target = match (line, col) {
                    (Some(l), Some(c)) => format!("{p}:{l}:{c}"),
                    (Some(l), None) => format!("{p}:{l}"),
                    _ => p,
                };
                vec!["--goto".into(), target]
            }
            Editor::Zed => {
                let target = match (line, col) {
                    (Some(l), Some(c)) => format!("{p}:{l}:{c}"),
                    (Some(l), None) => format!("{p}:{l}"),
                    _ => p,
                };
                vec![target]
            }
            Editor::JetBrains => {
                let mut v = Vec::new();
                if let Some(l) = line {
                    v.push("--line".into());
                    v.push(l.to_string());
                }
                if let Some(c) = col {
                    v.push("--column".into());
                    v.push(c.to_string());
                }
                v.push(p);
                v
            }
            Editor::Neovim => {
                let mut v = Vec::new();
                if let Some(l) = line {
                    v.push(format!("+{l}"));
                }
                v.push(p);
                v
            }
            Editor::Custom { args_template, .. } => {
                let rendered = args_template
                    .replace("{path}", &p)
                    .replace("{line}", &line.map(|l| l.to_string()).unwrap_or_default())
                    .replace("{col}", &col.map(|c| c.to_string()).unwrap_or_default());
                rendered.split_whitespace().map(|s| s.to_string()).collect()
            }
        }
    }
}

pub fn launch(
    editor: &Editor,
    worktree: &Path,
    file: Option<&str>,
    line: Option<u32>,
    col: Option<u32>,
) -> Result<()> {
    let target: PathBuf = match file {
        Some(rel) => resolve_within(worktree, rel)?,
        None => worktree.to_path_buf(),
    };
    let args = editor.args(&target, line, col);
    Command::new(editor.command())
        .args(&args)
        .spawn()
        .with_context(|| format!("failed to launch {}", editor.command()))?;
    Ok(())
}

/// Resolve `rel` under `worktree`, rejecting escapes via `..` or symlinks.
pub fn resolve_within(worktree: &Path, rel: &str) -> Result<PathBuf> {
    if Path::new(rel).is_absolute() {
        return Err(anyhow!("path must be relative"));
    }
    let joined = worktree.join(rel);
    let root = worktree
        .canonicalize()
        .with_context(|| format!("canonicalize worktree {}", worktree.display()))?;
    // Canonicalize parent if file doesn't yet exist, otherwise the full path.
    let canon = match joined.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            let parent = joined
                .parent()
                .ok_or_else(|| anyhow!("no parent"))?
                .canonicalize()
                .with_context(|| "canonicalize parent")?;
            let name = joined
                .file_name()
                .ok_or_else(|| anyhow!("no file name"))?;
            parent.join(name)
        }
    };
    if !canon.starts_with(&root) {
        return Err(anyhow!("path escapes worktree"));
    }
    Ok(canon)
}

pub fn parse_name(name: &str) -> Result<Editor> {
    Ok(match name.to_lowercase().as_str() {
        "code" | "vscode" | "vs-code" => Editor::VsCode,
        "cursor" => Editor::Cursor,
        "zed" => Editor::Zed,
        "idea" | "jetbrains" | "intellij" => Editor::JetBrains,
        "nvim" | "neovim" | "vim" => Editor::Neovim,
        _ => return Err(anyhow!("unknown editor `{name}` (use vscode|cursor|zed|jetbrains|neovim or configure custom)")),
    })
}
