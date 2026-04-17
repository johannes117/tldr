use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{auth, logging, state};

pub struct CheckResult {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

impl CheckResult {
    fn ok(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { name: name.into(), ok: true, detail: detail.into() }
    }
    fn fail(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { name: name.into(), ok: false, detail: detail.into() }
    }
}

pub fn check_git() -> CheckResult {
    let out = Command::new("git").arg("--version").output();
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            let ver = parse_git_version(&s);
            match ver {
                Some((maj, min)) if (maj, min) >= (2, 35) => {
                    CheckResult::ok("git", format!("{}.{} ({})", maj, min, s.trim()))
                }
                Some((maj, min)) => CheckResult::fail("git", format!("{}.{} < 2.35", maj, min)),
                None => CheckResult::fail("git", format!("unparseable: {}", s.trim())),
            }
        }
        _ => CheckResult::fail("git", "not found".to_string()),
    }
}

fn parse_git_version(s: &str) -> Option<(u32, u32)> {
    let rest = s.split_whitespace().nth(2)?;
    let mut parts = rest.split('.');
    let maj: u32 = parts.next()?.parse().ok()?;
    let min: u32 = parts.next()?.parse().ok()?;
    Some((maj, min))
}

pub fn check_disk_space() -> CheckResult {
    let Ok(dir) = state::state_root() else {
        return CheckResult::fail("disk", "state dir unavailable".to_string());
    };
    match free_bytes(&dir) {
        Some(b) => {
            let mb = b / (1024 * 1024);
            if b >= 500 * 1024 * 1024 {
                CheckResult::ok("disk", format!("{} MB free at {}", mb, dir.display()))
            } else {
                CheckResult::fail("disk", format!("only {} MB free at {}", mb, dir.display()))
            }
        }
        None => CheckResult::fail("disk", "unable to stat".to_string()),
    }
}

#[cfg(unix)]
fn free_bytes(p: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(p.as_os_str().as_bytes()).ok()?;
    unsafe {
        let mut sfs: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut sfs) != 0 { return None; }
        Some(sfs.f_bavail as u64 * sfs.f_frsize as u64)
    }
}

#[cfg(not(unix))]
fn free_bytes(_p: &Path) -> Option<u64> { None }

pub async fn check_token() -> CheckResult {
    let token = match auth::token().await {
        Ok(t) => t,
        Err(e) => return CheckResult::fail("github token", format!("{e}")),
    };
    let client = reqwest::Client::builder()
        .user_agent("tldr-cli")
        .build()
        .ok();
    let Some(c) = client else {
        return CheckResult::fail("github token", "http client".to_string());
    };
    let resp = c.get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .send().await;
    match resp {
        Ok(r) if r.status().is_success() => CheckResult::ok("github token", "valid".to_string()),
        Ok(r) => CheckResult::fail("github token", format!("status {}", r.status())),
        Err(e) => CheckResult::fail("github token", format!("{e}")),
    }
}

pub fn check_lsp_binaries() -> Vec<CheckResult> {
    ["tsserver", "pyright", "gopls", "rust-analyzer"]
        .iter()
        .map(|b| match which::which(b) {
            Ok(p) => CheckResult::ok(format!("lsp:{b}"), p.display().to_string()),
            Err(_) => CheckResult::fail(format!("lsp:{b}"), "not on PATH".to_string()),
        })
        .collect()
}

pub async fn run(bundle: bool) -> Result<()> {
    let mut results = vec![check_git(), check_disk_space(), check_token().await];
    results.extend(check_lsp_binaries());

    for r in &results {
        let mark = if r.ok { "\u{2713}" } else { "\u{2717}" };
        println!("{mark} {}: {}", r.name, r.detail);
    }

    if bundle {
        let path = create_bundle()?;
        println!("\nbundle: {}", path.display());
    }
    Ok(())
}

pub fn create_bundle() -> Result<PathBuf> {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let name = format!("tldr-doctor-{}.tar.gz", ts);
    let out_path = std::env::current_dir()?.join(&name);
    let file = std::fs::File::create(&out_path)?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut tar = tar::Builder::new(enc);

    // Logs: last 3 days, redacted.
    if let Ok(dir) = logging::logs_dir() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let now = std::time::SystemTime::now();
            let cutoff = std::time::Duration::from_secs(3 * 24 * 60 * 60);
            for e in rd.flatten() {
                let Ok(md) = e.metadata() else { continue };
                if !md.is_file() { continue; }
                let Ok(mt) = md.modified() else { continue };
                if now.duration_since(mt).unwrap_or_default() > cutoff { continue; }
                let contents = std::fs::read_to_string(e.path()).unwrap_or_default();
                let redacted = logging::redact(&contents);
                append_str(&mut tar, &format!("logs/{}", e.file_name().to_string_lossy()), &redacted)?;
            }
        }
    }

    // Config (redacted: only emit token_source key, never values).
    let cfg_summary = redacted_config_summary()?;
    append_str(&mut tar, "config.toml", &cfg_summary)?;

    // System info.
    let sys = system_info();
    append_str(&mut tar, "system.txt", &sys)?;

    tar.finish()?;
    Ok(out_path)
}

fn append_str<W: std::io::Write>(
    tar: &mut tar::Builder<W>,
    path: &str,
    contents: &str,
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(chrono::Utc::now().timestamp() as u64);
    header.set_cksum();
    tar.append_data(&mut header, path, contents.as_bytes())
        .map_err(|e| anyhow!("tar append: {e}"))?;
    Ok(())
}

fn redacted_config_summary() -> Result<String> {
    let source = if std::env::var("GITHUB_TOKEN").is_ok() {
        "env:GITHUB_TOKEN"
    } else if which::which("gh").is_ok() {
        "gh-cli"
    } else {
        "none"
    };
    Ok(format!("# redacted tldr config summary\ntoken_source = \"{}\"\n", source))
}

fn system_info() -> String {
    format!(
        "os = {}\narch = {}\ntldr_version = {}\nrustc = {}\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        env!("CARGO_PKG_VERSION"),
        option_env!("RUSTC_VERSION").unwrap_or("unknown"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_never_contains_raw_tokens() {
        // Ensure the redaction regex catches what we promise.
        let s = "header Authorization: Bearer ghp_ABC123XYZ trailing";
        let r = logging::redact(s);
        assert!(!r.contains("ghp_ABC123XYZ"));
        assert!(!r.contains("Bearer ghp"));
    }

    #[test]
    fn config_summary_has_no_token() {
        std::env::set_var("GITHUB_TOKEN", "ghp_secretvalue");
        let s = redacted_config_summary().unwrap();
        assert!(!s.contains("ghp_secretvalue"));
        assert!(s.contains("token_source"));
        std::env::remove_var("GITHUB_TOKEN");
    }
}
