use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Repo {
    pub root: PathBuf,
    pub origin_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Slug { pub owner: String, pub name: String }

impl std::fmt::Display for Slug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl Repo {
    pub fn slug(&self) -> Result<Slug> {
        let url = self.origin_url.as_ref().ok_or_else(|| anyhow!("no origin remote"))?;
        parse_slug(url)
    }
}

pub fn find_from_cwd() -> Result<Repo> {
    let cwd = std::env::current_dir()?;
    find_from(&cwd)
}

pub fn find_from(start: &Path) -> Result<Repo> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .output()
        .context("git rev-parse")?;
    if !out.status.success() {
        return Err(anyhow!("not a git repo"));
    }
    let root = PathBuf::from(String::from_utf8(out.stdout)?.trim());
    let origin = Command::new("git")
        .args(["-C", root.to_str().unwrap(), "remote", "get-url", "origin"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() { String::from_utf8(o.stdout).ok() } else { None })
        .map(|s| s.trim().to_string());
    Ok(Repo { root, origin_url: origin })
}

pub fn parse_slug(url: &str) -> Result<Slug> {
    // git@github.com:owner/name.git  OR  https://github.com/owner/name(.git)
    let trimmed = url.trim().trim_end_matches(".git");
    let after = if let Some(i) = trimmed.find("github.com") {
        let rest = &trimmed[i + "github.com".len()..];
        rest.trim_start_matches(&[':', '/'][..])
    } else {
        trimmed
    };
    let mut parts = after.splitn(2, '/');
    let owner = parts.next().ok_or_else(|| anyhow!("bad url"))?.to_string();
    let name = parts.next().ok_or_else(|| anyhow!("bad url"))?.to_string();
    Ok(Slug { owner, name })
}
