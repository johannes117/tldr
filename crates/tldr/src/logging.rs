use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

pub static REDACT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"gh[ps]_[A-Za-z0-9]+|github_pat_\S+|Bearer \S+").unwrap()
});

pub fn redact(s: &str) -> String {
    REDACT_RE.replace_all(s, "<REDACTED>").into_owned()
}

pub fn logs_dir() -> Result<PathBuf> {
    let base = if let Ok(x) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(x)
    } else if let Ok(h) = std::env::var("HOME") {
        PathBuf::from(h).join(".local/state")
    } else {
        return Err(anyhow!("no HOME"));
    };
    let p = base.join("tldr/logs");
    std::fs::create_dir_all(&p).ok();
    Ok(p)
}

const MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;
const MAX_TOTAL_BYTES: u64 = 100 * 1024 * 1024;

pub fn cleanup_old_logs(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<(PathBuf, SystemTime, u64)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let md = e.metadata().ok()?;
            if !md.is_file() { return None; }
            let mtime = md.modified().ok()?;
            Some((e.path(), mtime, md.len()))
        })
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    let now = SystemTime::now();
    let mut cum: u64 = 0;
    for (path, mtime, size) in entries {
        let age = now.duration_since(mtime).unwrap_or(Duration::ZERO).as_secs();
        cum += size;
        if age > MAX_AGE_SECS || cum > MAX_TOTAL_BYTES {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// A writer wrapper that applies redaction to every line before forwarding.
struct RedactingWriter<W: std::io::Write> {
    inner: W,
}

impl<W: std::io::Write> std::io::Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let s = String::from_utf8_lossy(buf);
        let red = redact(&s);
        self.inner.write_all(red.as_bytes())?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { self.inner.flush() }
}

struct RedactingMakeWriter<M> {
    inner: M,
}

impl<'a, M> tracing_subscriber::fmt::MakeWriter<'a> for RedactingMakeWriter<M>
where
    M: tracing_subscriber::fmt::MakeWriter<'a>,
{
    type Writer = RedactingWriter<M::Writer>;
    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter { inner: self.inner.make_writer() }
    }
}

pub struct LogGuard {
    _file: WorkerGuard,
}

pub fn init(foreground: bool) -> Result<LogGuard> {
    let dir = logs_dir()?;
    cleanup_old_logs(&dir);

    let appender = tracing_appender::rolling::daily(&dir, "tldr.log");
    let (nb, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_env("TLDR_LOG_FILTER")
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(false)
        .with_target(true)
        .with_writer(RedactingMakeWriter { inner: nb });

    let console = foreground || std::env::var("TLDR_LOG").ok().as_deref() == Some("1");

    let registry = tracing_subscriber::registry().with(filter).with(file_layer);
    if console {
        let c = tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_writer(RedactingMakeWriter { inner: std::io::stderr });
        registry.with(c).try_init().ok();
    } else {
        registry.try_init().ok();
    }

    Ok(LogGuard { _file: guard })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_tokens() {
        assert_eq!(redact("ghp_abcDEF123"), "<REDACTED>");
        assert_eq!(redact("ghs_xyz987"), "<REDACTED>");
        assert_eq!(redact("github_pat_11ABC"), "<REDACTED>");
        assert_eq!(redact("Bearer abc.def.ghi"), "<REDACTED>");
        assert_eq!(redact("hello ghp_TOKENx world"), "hello <REDACTED> world");
        assert_eq!(redact("normal text"), "normal text");
    }
}
