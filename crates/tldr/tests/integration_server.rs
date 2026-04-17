// Integration tests that operate against real subsystems (git, draft store) without spinning
// the full axum server, which requires GitHub + LSP infrastructure that would need real network.

use std::path::Path;
use std::process::Command;
use tldr::{diff, draft};

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git").current_dir(repo).args(args).output().unwrap();
    assert!(out.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
}

fn init_fixture_repo() -> tempfile::TempDir {
    let td = tempfile::tempdir().unwrap();
    let p = td.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.email", "t@t.test"]);
    git(p, &["config", "user.name", "t"]);
    std::fs::write(p.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(p.join("b.txt"), "alpha\nbeta\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-qm", "base"]);
    git(p, &["checkout", "-qb", "feature"]);
    std::fs::write(p.join("a.txt"), "one\ntwo-changed\nthree\nfour\n").unwrap();
    std::fs::write(p.join("c.txt"), "new file\n").unwrap();
    std::fs::remove_file(p.join("b.txt")).unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-qm", "feature change"]);
    td
}

#[test]
fn diff_compute_against_fixture_repo() {
    let td = init_fixture_repo();
    let d = diff::compute(td.path(), "main", "feature").expect("diff compute");
    let paths: Vec<&str> = d.files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"a.txt"), "missing a.txt: {paths:?}");
    assert!(paths.contains(&"c.txt"));
    let a = d.files.iter().find(|f| f.path == "a.txt").unwrap();
    assert_eq!(a.status, "modified");
    assert!(a.stats.added >= 1);
    let c = d.files.iter().find(|f| f.path == "c.txt").unwrap();
    assert_eq!(c.status, "added");
    let b = d.files.iter().find(|f| f.path == "b.txt");
    assert!(b.is_some());
    assert_eq!(b.unwrap().status, "deleted");
}

#[test]
fn diff_compute_file_returns_single() {
    let td = init_fixture_repo();
    let f = diff::compute_file(td.path(), "main", "feature", "a.txt").unwrap().unwrap();
    assert_eq!(f.path, "a.txt");
    assert!(!f.hunks.is_empty());
}

#[test]
fn draft_roundtrip() {
    let td = tempfile::tempdir().unwrap();
    let path = td.path().join("draft.json");
    let mut d = draft::load(&path, 42).unwrap();
    assert_eq!(d.pr, 42);
    d.body = "LGTM with nits".into();
    d.verdict = Some("comment".into());
    d.comments.push(draft::DraftComment {
        id: draft::new_comment_id(),
        path: "a.txt".into(),
        line: 2,
        side: "RIGHT".into(),
        body: "nit: rename".into(),
        created_at: "2025-01-01T00:00:00Z".into(),
    });
    draft::save(&path, &d).unwrap();
    let loaded = draft::load(&path, 42).unwrap();
    assert_eq!(loaded.body, "LGTM with nits");
    assert_eq!(loaded.comments.len(), 1);
    assert_eq!(loaded.comments[0].path, "a.txt");
}

#[test]
fn draft_default_when_missing() {
    let td = tempfile::tempdir().unwrap();
    let path = td.path().join("nope.json");
    let d = draft::load(&path, 7).unwrap();
    assert_eq!(d.pr, 7);
    assert!(d.comments.is_empty());
    assert!(d.body.is_empty());
}
