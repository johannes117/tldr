use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::Command;

pub fn fetch_pr_ref(repo_root: &Path, pr: u64) -> Result<()> {
    let refspec = format!("pull/{pr}/head:refs/tldr/pr-{pr}");
    let out = Command::new("git")
        .args(["-C", repo_root.to_str().unwrap(), "fetch", "origin", &refspec, "--force"])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("git fetch failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

pub fn ensure_worktree(repo_root: &Path, wt_path: &Path, pr: u64) -> Result<()> {
    if wt_path.exists() {
        return Ok(());
    }
    if let Some(parent) = wt_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let refname = format!("refs/tldr/pr-{pr}");
    let out = Command::new("git")
        .args(["-C", repo_root.to_str().unwrap(), "worktree", "add", "--detach",
               wt_path.to_str().unwrap(), &refname])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("git worktree add failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

pub fn remove_worktree(repo_root: &Path, wt_path: &Path) -> Result<()> {
    let _ = Command::new("git")
        .args(["-C", repo_root.to_str().unwrap(), "worktree", "remove", "--force",
               wt_path.to_str().unwrap()])
        .output()?;
    Ok(())
}

pub fn merge_base(repo_root: &Path, a: &str, b: &str) -> Result<String> {
    let out = Command::new("git")
        .args(["-C", repo_root.to_str().unwrap(), "merge-base", a, b])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("merge-base: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}
