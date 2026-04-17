use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diff {
    pub files: Vec<FileDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String, // modified|added|deleted|renamed
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hunk {
    pub header: String,
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Line {
    pub kind: String, // "add" | "del" | "ctx"
    pub content: String,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

pub fn compute(repo: &Path, base: &str, head: &str) -> Result<Diff> {
    let out = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "diff", "--no-color",
               &format!("{base}..{head}")])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("git diff: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    Ok(parse_unified(&text))
}

pub fn parse_unified(text: &str) -> Diff {
    let mut files: Vec<FileDiff> = vec![];
    let mut cur_file: Option<FileDiff> = None;
    let mut cur_hunk: Option<Hunk> = None;
    let mut old_ln: u32 = 0;
    let mut new_ln: u32 = 0;

    for line in text.lines() {
        if line.starts_with("diff --git ") {
            if let Some(h) = cur_hunk.take() { if let Some(f) = cur_file.as_mut() { f.hunks.push(h); } }
            if let Some(f) = cur_file.take() { files.push(f); }
            let parts: Vec<&str> = line.splitn(4, ' ').collect();
            let path = parts.get(3).unwrap_or(&"").trim_start_matches("b/").to_string();
            cur_file = Some(FileDiff { path, old_path: None, status: "modified".into(), hunks: vec![] });
        } else if line.starts_with("new file mode") {
            if let Some(f) = cur_file.as_mut() { f.status = "added".into(); }
        } else if line.starts_with("deleted file mode") {
            if let Some(f) = cur_file.as_mut() { f.status = "deleted".into(); }
        } else if line.starts_with("rename from ") {
            if let Some(f) = cur_file.as_mut() {
                f.old_path = Some(line.trim_start_matches("rename from ").to_string());
                f.status = "renamed".into();
            }
        } else if line.starts_with("+++ b/") {
            if let Some(f) = cur_file.as_mut() { f.path = line.trim_start_matches("+++ b/").to_string(); }
        } else if line.starts_with("--- a/") {
            if let Some(f) = cur_file.as_mut() {
                if f.old_path.is_none() { f.old_path = Some(line.trim_start_matches("--- a/").to_string()); }
            }
        } else if line.starts_with("@@") {
            if let Some(h) = cur_hunk.take() { if let Some(f) = cur_file.as_mut() { f.hunks.push(h); } }
            let (os, ns) = parse_hunk_header(line);
            old_ln = os; new_ln = ns;
            cur_hunk = Some(Hunk { header: line.to_string(), old_start: os, new_start: ns, lines: vec![] });
        } else if let Some(h) = cur_hunk.as_mut() {
            if let Some(rest) = line.strip_prefix('+') {
                h.lines.push(Line { kind: "add".into(), content: rest.to_string(), old_line: None, new_line: Some(new_ln) });
                new_ln += 1;
            } else if let Some(rest) = line.strip_prefix('-') {
                h.lines.push(Line { kind: "del".into(), content: rest.to_string(), old_line: Some(old_ln), new_line: None });
                old_ln += 1;
            } else if let Some(rest) = line.strip_prefix(' ') {
                h.lines.push(Line { kind: "ctx".into(), content: rest.to_string(), old_line: Some(old_ln), new_line: Some(new_ln) });
                old_ln += 1; new_ln += 1;
            }
        }
    }
    if let Some(h) = cur_hunk.take() { if let Some(f) = cur_file.as_mut() { f.hunks.push(h); } }
    if let Some(f) = cur_file.take() { files.push(f); }
    Diff { files }
}

fn parse_hunk_header(s: &str) -> (u32, u32) {
    // @@ -a,b +c,d @@
    let mut old_s = 0u32; let mut new_s = 0u32;
    let parts: Vec<&str> = s.split(' ').collect();
    for p in parts {
        if let Some(rest) = p.strip_prefix('-') {
            old_s = rest.split(',').next().and_then(|x| x.parse().ok()).unwrap_or(0);
        } else if let Some(rest) = p.strip_prefix('+') {
            new_s = rest.split(',').next().and_then(|x| x.parse().ok()).unwrap_or(0);
        }
    }
    (old_s, new_s)
}
