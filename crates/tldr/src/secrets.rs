use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;

const SERVICE: &str = "tldr";

pub fn store(key: &str, value: &str) -> Result<()> {
    match keyring::Entry::new(SERVICE, key).and_then(|e| e.set_password(value)) {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::warn!("keyring store failed for {key}: {e}; using file fallback");
            write_fallback(key, value)
        }
    }
}

pub fn read(key: &str) -> Result<Option<String>> {
    if let Ok(entry) = keyring::Entry::new(SERVICE, key) {
        match entry.get_password() {
            Ok(s) if !s.is_empty() => return Ok(Some(s)),
            Ok(_) => {}
            Err(keyring::Error::NoEntry) => {}
            Err(e) => tracing::warn!("keyring read failed for {key}: {e}"),
        }
    }
    read_fallback(key)
}

pub fn clear(key: &str) -> Result<()> {
    if let Ok(entry) = keyring::Entry::new(SERVICE, key) {
        let _ = entry.delete_password();
    }
    let p = fallback_path()?;
    if p.exists() {
        let body = std::fs::read_to_string(&p)?;
        let mut v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::json!({}));
        if let Some(obj) = v.as_object_mut() {
            obj.remove(key);
            std::fs::write(&p, v.to_string())?;
        }
    }
    Ok(())
}

fn fallback_path() -> Result<PathBuf> {
    let base = if let Ok(x) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(x)
    } else {
        let home = std::env::var("HOME").context("HOME not set")?;
        PathBuf::from(home).join(".local/state")
    };
    Ok(base.join("tldr").join("secrets.json"))
}

fn write_fallback(key: &str, value: &str) -> Result<()> {
    let p = fallback_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut v: serde_json::Value = if p.exists() {
        let s = std::fs::read_to_string(&p)?;
        serde_json::from_str(&s).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    v.as_object_mut()
        .ok_or_else(|| anyhow!("secrets file is not an object"))?
        .insert(key.to_string(), serde_json::Value::String(value.to_string()));
    std::fs::write(&p, v.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn read_fallback(key: &str) -> Result<Option<String>> {
    let p = fallback_path()?;
    if !p.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(&p)?;
    let v: serde_json::Value = serde_json::from_str(&s)?;
    Ok(v.get(key).and_then(|x| x.as_str()).map(str::to_string))
}
