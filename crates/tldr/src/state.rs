use anyhow::{anyhow, Result};
use std::path::PathBuf;

use crate::repo::Slug;

pub fn state_root() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "tldr", "tldr")
        .ok_or_else(|| anyhow!("no state dir"))?;
    let p = dirs.data_local_dir().to_path_buf();
    std::fs::create_dir_all(&p).ok();
    Ok(p)
}

pub fn repo_dir(slug: &Slug) -> Result<PathBuf> {
    let p = state_root()?.join("repos").join(format!("{}__{}", slug.owner, slug.name));
    std::fs::create_dir_all(&p).ok();
    Ok(p)
}

pub fn worktree_path(slug: &Slug, pr: u64) -> Result<PathBuf> {
    let p = repo_dir(slug)?.join("worktrees").join(format!("pr-{pr}"));
    Ok(p)
}

pub fn draft_path(slug: &Slug, pr: u64) -> Result<PathBuf> {
    let dir = repo_dir(slug)?.join("drafts");
    std::fs::create_dir_all(&dir).ok();
    Ok(dir.join(format!("pr-{pr}.json")))
}
