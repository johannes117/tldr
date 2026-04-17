use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

use crate::codeowners::CodeOwners;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileFrame {
    pub path: String,
    pub owned: bool,
    pub touched_before: bool,
    pub expertise_score: f32,
}

pub async fn frame_files(
    repo_path: &Path,
    reviewer_login: Option<&str>,
    reviewer_email: Option<&str>,
    files: &[String],
) -> Vec<FileFrame> {
    let co = load_codeowners(repo_path);
    let login_tag = reviewer_login.map(|l| format!("@{}", l.trim_start_matches('@')));

    let mut out = Vec::with_capacity(files.len());
    for path in files {
        let owned = match (&co, &login_tag) {
            (Some(c), Some(tag)) => c.owners_for(path).iter().any(|o| o.eq_ignore_ascii_case(tag)),
            _ => false,
        };
        let (touched_before, expertise_score) = match reviewer_email {
            Some(email) if !email.is_empty() => {
                let all = count_commits(repo_path, email, path, None);
                let recent = count_commits(repo_path, email, path, Some("90 days ago"));
                let score = (recent as f32 / 10.0).min(1.0);
                (all > 0, score)
            }
            _ => (false, 0.0),
        };
        out.push(FileFrame { path: path.clone(), owned, touched_before, expertise_score });
    }
    out
}

fn load_codeowners(repo_path: &Path) -> Option<CodeOwners> {
    // Override from .tldr/config.toml
    let cfg = repo_path.join(".tldr/config.toml");
    if let Ok(s) = std::fs::read_to_string(&cfg) {
        if let Ok(v) = toml::from_str::<toml::Value>(&s) {
            if let Some(p) = v.get("codeowners").and_then(|c| c.get("path")).and_then(|p| p.as_str()) {
                let fp = repo_path.join(p);
                if let Ok(txt) = std::fs::read_to_string(&fp) {
                    return Some(CodeOwners::parse(&txt));
                }
            }
        }
    }
    CodeOwners::load_from_repo(repo_path)
}

fn count_commits(repo_path: &Path, email: &str, path: &str, since: Option<&str>) -> usize {
    let mut args: Vec<String> = vec![
        "-C".into(), repo_path.to_string_lossy().into_owned(),
        "log".into(),
        format!("--author={}", email),
        "--pretty=format:%H".into(),
    ];
    if let Some(s) = since { args.push(format!("--since={}", s)); }
    args.push("--".into());
    args.push(path.to_string());
    let out = match Command::new("git").args(&args).output() {
        Ok(o) => o,
        Err(_) => return 0,
    };
    if !out.status.success() { return 0; }
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines().filter(|l| !l.trim().is_empty()).count()
}
