// Integration test for session file read/write/alive.
use tldr::session::{self, SessionFile};

fn sample(pid: u32, port: u16) -> SessionFile {
    SessionFile {
        pid,
        port,
        started_at: "2025-01-01T00:00:00Z".into(),
        active_prs: vec![1, 2, 3],
        csrf_token_fingerprint: "abcd1234".into(),
        csrf_token: "full-csrf-token-value".into(),
        slug: "owner/repo".into(),
    }
}

#[test]
fn write_load_round_trip() {
    let td = tempfile::tempdir().unwrap();
    let path = td.path().join("sub/session.json");
    let s = sample(12345, 47800);
    session::write_atomic(&path, &s).unwrap();
    assert!(path.exists());
    let loaded = session::load(&path).unwrap();
    assert_eq!(loaded.pid, 12345);
    assert_eq!(loaded.active_prs, vec![1, 2, 3]);
    assert_eq!(loaded.slug, "owner/repo");
}

#[test]
fn remove_deletes_file() {
    let td = tempfile::tempdir().unwrap();
    let path = td.path().join("session.json");
    session::write_atomic(&path, &sample(1, 1)).unwrap();
    assert!(path.exists());
    session::remove(&path);
    assert!(!path.exists());
}

#[test]
fn pid_alive_current_process() {
    assert!(session::pid_alive(std::process::id()));
}

#[test]
fn pid_alive_unlikely_pid() {
    // PID 0 is not a real user pid on unix; should be reported not-alive from user context.
    // Use a very high pid that is extremely unlikely to exist.
    assert!(!session::pid_alive(9_999_999));
}

#[test]
fn port_alive_closed_port_false() {
    // Pick a port that is extremely unlikely to be listening.
    assert!(!session::port_alive(1));
}

#[test]
fn is_alive_false_when_dead_pid() {
    let s = sample(9_999_999, 1);
    assert!(!session::is_alive(&s));
}
