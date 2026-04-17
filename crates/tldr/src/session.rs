use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::repo::Slug;
use crate::state;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFile {
    pub pid: u32,
    pub port: u16,
    pub started_at: String,
    pub active_prs: Vec<u64>,
    pub csrf_token_fingerprint: String,
    #[serde(default)]
    pub csrf_token: String,
    pub slug: String,
}

pub fn session_path(slug: &Slug) -> Result<PathBuf> {
    Ok(state::repo_dir(slug)?.join("session.json"))
}

pub fn all_session_paths() -> Result<Vec<PathBuf>> {
    let root = state::state_root()?.join("repos");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let p = e.path().join("session.json");
            if p.exists() { out.push(p); }
        }
    }
    Ok(out)
}

pub fn load(path: &Path) -> Result<SessionFile> {
    let s = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&s)?)
}

pub fn write_atomic(path: &Path, sess: &SessionFile) -> Result<()> {
    if let Some(p) = path.parent() { std::fs::create_dir_all(p).ok(); }
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(serde_json::to_string_pretty(sess)?.as_bytes())?;
        f.sync_all().ok();
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn remove(path: &Path) { let _ = std::fs::remove_file(path); }

pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    unsafe {
        if libc::kill(pid as i32, 0) == 0 { return true; }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    { let _ = pid; false }
}

pub fn port_alive(port: u16) -> bool {
    use std::net::TcpStream;
    use std::time::Duration;
    TcpStream::connect_timeout(&(([127, 0, 0, 1], port).into()), Duration::from_millis(200)).is_ok()
}

pub fn is_alive(s: &SessionFile) -> bool {
    #[cfg(unix)]
    { pid_alive(s.pid) && port_alive(s.port) }
    #[cfg(not(unix))]
    { port_alive(s.port) }
}

pub struct Guard { pub path: PathBuf }
impl Drop for Guard {
    fn drop(&mut self) { remove(&self.path); }
}
