use anyhow::{anyhow, Result};
use tokio::process::Command;

pub async fn token() -> Result<String> {
    if let Ok(t) = std::env::var("GITHUB_TOKEN") {
        if !t.is_empty() { return Ok(t); }
    }
    if let Ok(t) = std::env::var("GH_TOKEN") {
        if !t.is_empty() { return Ok(t); }
    }
    let out = Command::new("gh").args(["auth", "token"]).output().await
        .map_err(|e| anyhow!("gh not available: {e}"))?;
    if !out.status.success() {
        return Err(anyhow!("gh auth token failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let t = String::from_utf8(out.stdout)?.trim().to_string();
    if t.is_empty() { return Err(anyhow!("empty token")); }
    Ok(t)
}

// TODO(future): keychain storage via `keyring` crate behind feature flag.
