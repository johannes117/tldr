use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::process::Command;

// NOTE: This is GitHub CLI's public OAuth client ID, reused here for development.
// A production build of `tldr` should register its own OAuth app and substitute
// the client_id below.
const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const SCOPES: &str = "repo read:user read:org";
const KEYRING_SERVICE: &str = "tldr";
const KEYRING_USER: &str = "github";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    Env,
    Keychain,
    Gh,
}

impl TokenSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenSource::Env => "env",
            TokenSource::Keychain => "keychain",
            TokenSource::Gh => "gh",
        }
    }
}

/// Legacy alias kept for existing callers.
pub async fn token() -> Result<String> {
    get_token().await
}

pub async fn get_token() -> Result<String> {
    get_token_with_source().await.map(|(t, _)| t)
}

pub async fn get_token_with_source() -> Result<(String, TokenSource)> {
    if let Ok(t) = std::env::var("GITHUB_TOKEN") {
        if !t.is_empty() {
            return Ok((t, TokenSource::Env));
        }
    }
    if let Ok(t) = std::env::var("GH_TOKEN") {
        if !t.is_empty() {
            return Ok((t, TokenSource::Env));
        }
    }
    if let Some(t) = read_keyring().ok().flatten() {
        if !t.is_empty() {
            return Ok((t, TokenSource::Keychain));
        }
    }
    if let Some(t) = read_fallback_file().ok().flatten() {
        if !t.is_empty() {
            return Ok((t, TokenSource::Keychain));
        }
    }
    if let Ok(out) = Command::new("gh").args(["auth", "token"]).output().await {
        if out.status.success() {
            let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !t.is_empty() {
                return Ok((t, TokenSource::Gh));
            }
        }
    }
    Err(anyhow!(
        "no GitHub token found. Run `tldr auth login` to sign in, or set GITHUB_TOKEN."
    ))
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResp {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct AccessTokenResp {
    access_token: Option<String>,
    error: Option<String>,
    #[allow(dead_code)]
    error_description: Option<String>,
    interval: Option<u64>,
}

pub async fn device_login() -> Result<String> {
    let http = reqwest::Client::builder()
        .user_agent("tldr-cli")
        .build()?;

    let dc: DeviceCodeResp = http
        .post("https://github.com/login/device/code")
        .header("Accept", "application/json")
        .form(&[("client_id", CLIENT_ID), ("scope", SCOPES)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    println!();
    println!("To authenticate, visit: {}", dc.verification_uri);
    println!("And enter the code: {}", dc.user_code);
    println!();
    open::that(&dc.verification_uri).ok();

    let deadline = Instant::now() + Duration::from_secs(dc.expires_in);
    let mut interval = Duration::from_secs(dc.interval.max(1));

    loop {
        if Instant::now() >= deadline {
            return Err(anyhow!("device code expired; please retry `tldr auth login`"));
        }
        tokio::time::sleep(interval).await;

        let resp: AccessTokenResp = http
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .form(&[
                ("client_id", CLIENT_ID),
                ("device_code", dc.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await?
            .json()
            .await?;

        if let Some(tok) = resp.access_token {
            return Ok(tok);
        }
        match resp.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                let bump = resp.interval.unwrap_or(dc.interval + 5);
                interval = Duration::from_secs(bump.max(interval.as_secs() + 5));
            }
            Some("expired_token") => {
                return Err(anyhow!("device code expired; please retry `tldr auth login`"));
            }
            Some("access_denied") => {
                return Err(anyhow!("authorization denied"));
            }
            Some(other) => return Err(anyhow!("oauth error: {other}")),
            None => return Err(anyhow!("unexpected empty response from GitHub")),
        }
    }
}

pub fn store_token(token: &str) -> Result<()> {
    match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .and_then(|e| e.set_password(token))
    {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::warn!("keyring store failed: {e}; falling back to file");
            write_fallback_file(token)
        }
    }
}

pub fn clear_token() -> Result<()> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.delete_password();
    }
    let p = fallback_path()?;
    if p.exists() {
        std::fs::remove_file(&p).ok();
    }
    Ok(())
}

fn read_keyring() -> Result<Option<String>> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    match entry.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow!("{e}")),
    }
}

fn fallback_path() -> Result<PathBuf> {
    let base = if let Ok(x) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(x)
    } else {
        let home = std::env::var("HOME").context("HOME not set")?;
        PathBuf::from(home).join(".local/state")
    };
    Ok(base.join("tldr").join("auth.json"))
}

fn write_fallback_file(token: &str) -> Result<()> {
    let p = fallback_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::json!({ "github_token": token }).to_string();
    std::fs::write(&p, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&p, perm)?;
    }
    Ok(())
}

fn read_fallback_file() -> Result<Option<String>> {
    let p = fallback_path()?;
    if !p.exists() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&p)?;
    let v: serde_json::Value = serde_json::from_str(&body)?;
    Ok(v.get("github_token")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_source_as_str() {
        assert_eq!(TokenSource::Env.as_str(), "env");
        assert_eq!(TokenSource::Keychain.as_str(), "keychain");
        assert_eq!(TokenSource::Gh.as_str(), "gh");
    }

    #[tokio::test]
    async fn env_github_token_takes_precedence() {
        // Snapshot & clear both
        let prev_gh = std::env::var("GITHUB_TOKEN").ok();
        let prev_gh2 = std::env::var("GH_TOKEN").ok();
        std::env::set_var("GITHUB_TOKEN", "ghp_test_env_token");
        std::env::remove_var("GH_TOKEN");
        let (t, src) = get_token_with_source().await.unwrap();
        assert_eq!(t, "ghp_test_env_token");
        assert_eq!(src, TokenSource::Env);
        // restore
        match prev_gh { Some(v) => std::env::set_var("GITHUB_TOKEN", v), None => std::env::remove_var("GITHUB_TOKEN") }
        if let Some(v) = prev_gh2 { std::env::set_var("GH_TOKEN", v); }
    }

    #[tokio::test]
    async fn gh_token_fallback_when_no_github_token() {
        let prev_gh = std::env::var("GITHUB_TOKEN").ok();
        let prev_gh2 = std::env::var("GH_TOKEN").ok();
        std::env::remove_var("GITHUB_TOKEN");
        std::env::set_var("GH_TOKEN", "ghp_from_gh_token");
        let (t, src) = get_token_with_source().await.unwrap();
        assert_eq!(t, "ghp_from_gh_token");
        assert_eq!(src, TokenSource::Env);
        if let Some(v) = prev_gh { std::env::set_var("GITHUB_TOKEN", v); }
        match prev_gh2 { Some(v) => std::env::set_var("GH_TOKEN", v), None => std::env::remove_var("GH_TOKEN") }
    }
}

pub async fn fetch_login(token: &str) -> Result<String> {
    let http = reqwest::Client::builder().user_agent("tldr-cli").build()?;
    let v: serde_json::Value = http
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(v.get("login")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string())
}
