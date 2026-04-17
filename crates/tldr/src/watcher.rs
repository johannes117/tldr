use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::draft;
use crate::github::{self, Client};
use crate::indexer::Indexer;
use crate::repo::Slug;
use crate::state;

const DEFAULT_INTERVAL_SECS: u64 = 60;
const RATE_LIMITED_INTERVAL_SECS: u64 = 300;
const RATE_LIMIT_THRESHOLD: u64 = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WatchEvent {
    NewCommits { old_head: String, new_head: String, count: u32 },
    NewComments { count: u32 },
    CiStatusChange { state: String },
    StateChange { state: String },
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct WatchStatus {
    pub head_sha: String,
    pub comment_count: u32,
    pub ci_state: String,
    pub pr_state: String,
}

pub struct PrWatcher {
    pub handle: JoinHandle<()>,
}

pub struct WatchRegistry {
    pub tx: broadcast::Sender<WatchEvent>,
    pub status: Arc<RwLock<WatchStatus>>,
    pub watcher: Mutex<Option<PrWatcher>>,
}

impl WatchRegistry {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(128);
        Self {
            tx,
            status: Arc::new(RwLock::new(WatchStatus::default())),
            watcher: Mutex::new(None),
        }
    }
}

pub struct WatcherDeps {
    pub token: String,
    pub slug: Slug,
    pub pr_number: u32,
    pub repo_root: std::path::PathBuf,
    pub worktree: std::path::PathBuf,
    pub indexer: Arc<Indexer>,
    pub tx: broadcast::Sender<WatchEvent>,
    pub status: Arc<RwLock<WatchStatus>>,
}

impl PrWatcher {
    pub fn spawn(deps: WatcherDeps) -> Self {
        let handle = tokio::spawn(async move { run(deps).await });
        Self { handle }
    }
}

async fn run(deps: WatcherDeps) {
    let WatcherDeps {
        token, slug, pr_number, repo_root, worktree, indexer, tx, status,
    } = deps;
    let client = Client::new(token.clone());
    let mut interval = Duration::from_secs(DEFAULT_INTERVAL_SECS);
    let mut last_head: Option<String> = None;
    let mut last_comment_total: Option<u32> = None;
    let mut last_ci: Option<String> = None;
    let mut last_state: Option<String> = None;

    loop {
        match poll_once(&client, &slug, pr_number as u64).await {
            Ok(PollResult { head_sha, pr_state, merged, rl_remaining, review_comments, issue_comments, ci_state }) => {
                let effective_state = if merged { "merged".to_string() } else { pr_state.clone() };
                let total_comments = review_comments + issue_comments;
                {
                    let mut s = status.write().await;
                    s.head_sha = head_sha.clone();
                    s.comment_count = total_comments;
                    s.ci_state = ci_state.clone();
                    s.pr_state = effective_state.clone();
                }

                if let Some(prev) = &last_head {
                    if prev != &head_sha {
                        let count = count_new_commits(&repo_root, prev, &head_sha, pr_number as u64).unwrap_or(0);
                        let _ = tx.send(WatchEvent::NewCommits { old_head: prev.clone(), new_head: head_sha.clone(), count });
                        if let Err(e) = update_worktree(&repo_root, &worktree, pr_number as u64) {
                            tracing::warn!(error = %e, "watcher.update_worktree failed");
                        }
                        remap_orphans(&slug, pr_number as u64, &worktree, prev, &head_sha);
                        let slug_str = format!("{}", slug);
                        let indexer = indexer.clone();
                        let wt = worktree.clone();
                        let head = head_sha.clone();
                        tokio::task::spawn_blocking(move || {
                            if let Err(e) = indexer.index_tree(&slug_str, &head, &wt) {
                                tracing::warn!(error = %e, "watcher.index_tree failed");
                            }
                        });
                    }
                }
                last_head = Some(head_sha.clone());

                if let Some(prev) = last_comment_total {
                    if total_comments > prev {
                        let _ = tx.send(WatchEvent::NewComments { count: total_comments - prev });
                    }
                }
                last_comment_total = Some(total_comments);

                if last_ci.as_deref() != Some(ci_state.as_str()) && !ci_state.is_empty() {
                    let _ = tx.send(WatchEvent::CiStatusChange { state: ci_state.clone() });
                }
                last_ci = Some(ci_state);

                if last_state.as_deref() != Some(effective_state.as_str()) {
                    let _ = tx.send(WatchEvent::StateChange { state: effective_state.clone() });
                }
                last_state = Some(effective_state.clone());

                if effective_state == "closed" || effective_state == "merged" {
                    tracing::info!(pr = pr_number, state = %effective_state, "watcher.exit");
                    return;
                }

                interval = if rl_remaining < RATE_LIMIT_THRESHOLD {
                    Duration::from_secs(RATE_LIMITED_INTERVAL_SECS)
                } else {
                    Duration::from_secs(DEFAULT_INTERVAL_SECS)
                };
            }
            Err(e) => {
                tracing::warn!(error = %e, pr = pr_number, "watcher.poll_failed");
            }
        }
        tokio::time::sleep(interval).await;
    }
}

struct PollResult {
    head_sha: String,
    pr_state: String,
    merged: bool,
    rl_remaining: u64,
    review_comments: u32,
    issue_comments: u32,
    ci_state: String,
}

async fn poll_once(c: &Client, slug: &Slug, n: u64) -> anyhow::Result<PollResult> {
    let url = format!("https://api.github.com/repos/{}/{}/pulls/{}", slug.owner, slug.name, n);
    let v: Value = github::rest_json(c, reqwest::Method::GET, &url, None).await?;
    let head_sha = v["head"]["sha"].as_str().unwrap_or("").to_string();
    let pr_state = v["state"].as_str().unwrap_or("open").to_string();
    let merged = v["merged"].as_bool().unwrap_or(false);
    let review_comments = v["review_comments"].as_u64().unwrap_or(0) as u32;
    let issue_comments = v["comments"].as_u64().unwrap_or(0) as u32;

    let rl_url = "https://api.github.com/rate_limit";
    let rl: Value = github::rest_json(c, reqwest::Method::GET, rl_url, None).await.unwrap_or(Value::Null);
    let rl_remaining = rl["rate"]["remaining"].as_u64().unwrap_or(5000);

    let ci_state = if !head_sha.is_empty() {
        let cs_url = format!("https://api.github.com/repos/{}/{}/commits/{}/status", slug.owner, slug.name, head_sha);
        let cs: Value = github::rest_json(c, reqwest::Method::GET, &cs_url, None).await.unwrap_or(Value::Null);
        cs["state"].as_str().unwrap_or("").to_string()
    } else {
        String::new()
    };

    Ok(PollResult {
        head_sha, pr_state, merged, rl_remaining,
        review_comments, issue_comments, ci_state,
    })
}

fn count_new_commits(repo_root: &Path, old: &str, new: &str, pr: u64) -> anyhow::Result<u32> {
    // fetch first so revs exist locally
    let _ = Command::new("git")
        .args(["-C", repo_root.to_str().unwrap(), "fetch", "origin",
               &format!("pull/{pr}/head:refs/tldr/pr-{pr}"), "--force"])
        .output();
    let out = Command::new("git")
        .args(["-C", repo_root.to_str().unwrap(), "rev-list", "--count",
               &format!("{old}..{new}")])
        .output()?;
    if !out.status.success() { return Ok(0); }
    Ok(String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0))
}

fn update_worktree(repo_root: &Path, worktree: &Path, pr: u64) -> anyhow::Result<()> {
    let refspec = format!("pull/{pr}/head:refs/tldr/pr-{pr}");
    let out = Command::new("git")
        .args(["-C", worktree.to_str().unwrap(), "fetch", "origin", &refspec, "--force"])
        .output()?;
    if !out.status.success() {
        // Fall back to fetching in repo_root
        let _ = Command::new("git")
            .args(["-C", repo_root.to_str().unwrap(), "fetch", "origin", &refspec, "--force"])
            .output();
    }
    let refname = format!("refs/tldr/pr-{pr}");
    let out = Command::new("git")
        .args(["-C", worktree.to_str().unwrap(), "reset", "--hard", &refname])
        .output()?;
    if !out.status.success() {
        return Err(anyhow::anyhow!("reset failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

fn remap_orphans(slug: &Slug, pr: u64, worktree: &Path, old_head: &str, new_head: &str) {
    let path = match state::draft_path(slug, pr) { Ok(p) => p, Err(_) => return };
    let mut d = match draft::load(&path, pr) { Ok(d) => d, Err(_) => return };
    let mut cache: HashMap<String, Option<HashMap<u32, Option<u32>>>> = HashMap::new();
    let mut changed = false;
    let comments_len = d.comments.len();
    for i in 0..comments_len {
        let cpath = d.comments[i].path.clone();
        let mapping = cache.entry(cpath.clone())
            .or_insert_with(|| line_map(worktree, old_head, new_head, &cpath));
        let orphan = match mapping {
            Some(m) => m.get(&d.comments[i].line).map(|v| v.is_none()).unwrap_or(true),
            None => false,
        };
        let body = &d.comments[i].body;
        let marker = "\n\n[orphaned]";
        let already = body.contains(marker);
        if orphan && !already {
            d.comments[i].body = format!("{}{}", body, marker);
            changed = true;
        }
    }
    if changed {
        let _ = draft::save(&path, &d);
    }
}

fn line_map(worktree: &Path, old: &str, new: &str, path: &str) -> Option<HashMap<u32, Option<u32>>> {
    let out = Command::new("git")
        .args(["-C", worktree.to_str().unwrap(), "diff", "--unified=0",
               old, new, "--", path])
        .output().ok()?;
    if !out.status.success() { return None; }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    // Build a map of old_line -> Option<new_line>.
    // Default assumption: unchanged lines outside hunks map 1:1 with running offsets.
    let mut map: HashMap<u32, Option<u32>> = HashMap::new();
    let mut old_cursor: i64 = 1;
    let mut new_cursor: i64 = 1;
    for line in text.lines() {
        if let Some(hdr) = line.strip_prefix("@@") {
            // @@ -old_start,old_count +new_start,new_count @@
            let parts: Vec<&str> = hdr.split_whitespace().collect();
            if parts.len() < 2 { continue; }
            let (ol, oc) = parse_range(parts[0]);
            let (nl, nc) = parse_range(parts[1]);
            // Map unchanged range before hunk.
            while old_cursor < ol as i64 {
                map.insert(old_cursor as u32, Some(new_cursor as u32));
                old_cursor += 1;
                new_cursor += 1;
            }
            // Lines inside the hunk: all old lines removed -> None.
            for k in 0..oc {
                map.insert((ol + k) as u32, None);
            }
            old_cursor = (ol + oc) as i64;
            new_cursor = (nl + nc) as i64;
        }
    }
    // After last hunk, remaining unchanged lines: we don't know file length, so any
    // line >= old_cursor not in map is assumed to map with offset.
    // Record the offset as a special key via closure returned through map (not possible),
    // so we encode: iterate expected range up to a cap.
    // Cap at 100k lines.
    let cap: i64 = 200_000;
    while old_cursor < cap {
        map.insert(old_cursor as u32, Some(new_cursor as u32));
        old_cursor += 1;
        new_cursor += 1;
    }
    Some(map)
}

fn parse_range(s: &str) -> (u64, u64) {
    // e.g. "-12,4" or "+13"
    let s = &s[1..];
    let mut it = s.split(',');
    let start: u64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let count: u64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(1);
    (start, count)
}
