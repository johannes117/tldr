use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

use crate::github;
use crate::repo::Slug;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedIssue {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalLink {
    pub text: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorPr {
    pub number: u64,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameLine {
    pub line: u32,
    pub author: String,
    pub commit_sha: String,
    pub commit_msg: String,
    pub date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhyTrace {
    pub pr_description_section: Option<String>,
    pub linked_issues: Vec<LinkedIssue>,
    pub external_links: Vec<ExternalLink>,
    pub prior_prs: Vec<PriorPr>,
    pub blame: Vec<BlameLine>,
    pub release_notes: Option<String>,
}

pub struct WhyCtx<'a> {
    pub client: &'a github::Client,
    pub slug: &'a Slug,
    pub repo_root: &'a Path,
    pub worktree: &'a Path,
    pub pr_body: Option<&'a str>,
}

pub async fn trace_hunk(
    ctx: &WhyCtx<'_>,
    pr_number: u64,
    path: &str,
    new_start_line: u32,
    new_line_count: u32,
) -> WhyTrace {
    let body = ctx.pr_body.unwrap_or("");
    let pr_description_section = extract_section_for_path(body, path);
    let external_links = scan_external_links(body);
    let mut linked_issues = scan_body_issue_refs(body, ctx.slug);
    if let Ok(more) = fetch_closing_issues(ctx.client, ctx.slug, pr_number).await {
        for m in more {
            if !linked_issues.iter().any(|x| x.number == m.number) {
                linked_issues.push(m);
            }
        }
    }
    let blame = git_blame(ctx.worktree, path, new_start_line, new_line_count);
    let shas: Vec<String> = blame.iter().map(|b| b.commit_sha.clone()).collect();
    let prior_prs = fetch_prior_prs(ctx.client, ctx.slug, &shas, pr_number).await.unwrap_or_default();
    let release_notes = find_release_notes(ctx.repo_root, &shas);

    WhyTrace {
        pr_description_section,
        linked_issues,
        external_links,
        prior_prs,
        blame,
        release_notes,
    }
}

fn extract_section_for_path(body: &str, path: &str) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    let mut needles: Vec<String> = Vec::new();
    needles.push(path.to_string());
    if let Some(fname) = Path::new(path).file_name().and_then(|s| s.to_str()) {
        needles.push(fname.to_string());
    }
    let mut parent = Path::new(path).parent();
    while let Some(p) = parent {
        if let Some(s) = p.to_str() {
            if !s.is_empty() {
                needles.push(s.to_string());
                if let Some(seg) = Path::new(s).file_name().and_then(|x| x.to_str()) {
                    needles.push(seg.to_string());
                }
            }
        }
        parent = p.parent();
    }
    let lines: Vec<&str> = body.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let heading = trimmed.trim_start_matches('#').trim().to_lowercase();
            if needles.iter().any(|n| !n.is_empty() && heading.contains(&n.to_lowercase())) {
                let level = trimmed.chars().take_while(|c| *c == '#').count();
                let mut section = String::new();
                section.push_str(line);
                section.push('\n');
                let mut j = i + 1;
                while j < lines.len() {
                    let t = lines[j].trim_start();
                    if t.starts_with('#') {
                        let l2 = t.chars().take_while(|c| *c == '#').count();
                        if l2 <= level {
                            break;
                        }
                    }
                    section.push_str(lines[j]);
                    section.push('\n');
                    j += 1;
                }
                return Some(section.trim_end().to_string());
            }
        }
        i += 1;
    }
    None
}

fn scan_external_links(body: &str) -> Vec<ExternalLink> {
    let re = regex::Regex::new(r"(?i)https?://[^\s)\]]+").unwrap();
    let md = regex::Regex::new(r"\[([^\]]+)\]\((https?://[^)]+)\)").unwrap();
    let domains = [
        "linear.app",
        "notion.so",
        "slack.com",
        "figma.com",
        "docs.google.com",
    ];
    let mut out: Vec<ExternalLink> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for cap in md.captures_iter(body) {
        let text = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
        let url = cap.get(2).map(|m| m.as_str()).unwrap_or("").to_string();
        if domains.iter().any(|d| url.contains(d)) && seen.insert(url.clone()) {
            out.push(ExternalLink { text, url });
        }
    }
    for m in re.find_iter(body) {
        let url = m.as_str().trim_end_matches(|c: char| matches!(c, '.' | ',' | ';')).to_string();
        if domains.iter().any(|d| url.contains(d)) && seen.insert(url.clone()) {
            out.push(ExternalLink { text: url.clone(), url });
        }
    }
    out
}

fn scan_body_issue_refs(body: &str, slug: &Slug) -> Vec<LinkedIssue> {
    let mut out: Vec<LinkedIssue> = Vec::new();
    let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let re_xrepo = regex::Regex::new(r"([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)#(\d+)").unwrap();
    for c in re_xrepo.captures_iter(body) {
        let owner = c.get(1).unwrap().as_str();
        let name = c.get(2).unwrap().as_str();
        let n: u64 = c.get(3).unwrap().as_str().parse().unwrap_or(0);
        if n == 0 {
            continue;
        }
        if !seen.insert(n) {
            continue;
        }
        let url = format!("https://github.com/{}/{}/issues/{}", owner, name, n);
        out.push(LinkedIssue {
            number: n,
            title: format!("{}/{}#{}", owner, name, n),
            url,
            state: String::new(),
        });
    }
    let re_local = regex::Regex::new(r"(?:^|[^A-Za-z0-9_/])#(\d+)").unwrap();
    for c in re_local.captures_iter(body) {
        let n: u64 = c.get(1).unwrap().as_str().parse().unwrap_or(0);
        if n == 0 {
            continue;
        }
        if !seen.insert(n) {
            continue;
        }
        out.push(LinkedIssue {
            number: n,
            title: format!("#{}", n),
            url: format!("https://github.com/{}/{}/issues/{}", slug.owner, slug.name, n),
            state: String::new(),
        });
    }
    out
}

async fn fetch_closing_issues(
    client: &github::Client,
    slug: &Slug,
    pr_number: u64,
) -> Result<Vec<LinkedIssue>> {
    let query = r#"
        query($owner:String!, $name:String!, $number:Int!) {
            repository(owner:$owner, name:$name) {
                pullRequest(number:$number) {
                    closingIssuesReferences(first: 20) {
                        nodes { number title url state }
                    }
                }
            }
        }
    "#;
    let vars = serde_json::json!({
        "owner": slug.owner,
        "name": slug.name,
        "number": pr_number,
    });
    let v = github::graphql(client, query, vars).await?;
    let nodes = v
        .pointer("/data/repository/pullRequest/closingIssuesReferences/nodes")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(nodes
        .into_iter()
        .filter_map(|n| {
            Some(LinkedIssue {
                number: n.get("number")?.as_u64()?,
                title: n.get("title")?.as_str()?.to_string(),
                url: n.get("url")?.as_str()?.to_string(),
                state: n.get("state").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            })
        })
        .collect())
}

fn git_blame(worktree: &Path, path: &str, start: u32, count: u32) -> Vec<BlameLine> {
    if count == 0 {
        return vec![];
    }
    let end = start + count.saturating_sub(1);
    let out = Command::new("git")
        .current_dir(worktree)
        .args([
            "blame",
            "-L",
            &format!("{},{}", start, end),
            "--porcelain",
            "--",
            path,
        ])
        .output();
    let out = match out {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let mut results: Vec<BlameLine> = Vec::new();
    let mut commit_cache: std::collections::HashMap<String, (String, String, String)> =
        std::collections::HashMap::new();
    let mut cur_sha = String::new();
    let mut cur_author = String::new();
    let mut cur_summary = String::new();
    let mut cur_time: i64 = 0;
    let mut cur_line: u32 = 0;
    for line in stdout.lines() {
        if line.starts_with('\t') {
            let (author, msg, date) = if let Some(c) = commit_cache.get(&cur_sha) {
                c.clone()
            } else {
                let entry = (
                    cur_author.clone(),
                    cur_summary.clone(),
                    format_ts(cur_time),
                );
                commit_cache.insert(cur_sha.clone(), entry.clone());
                entry
            };
            results.push(BlameLine {
                line: cur_line,
                author,
                commit_sha: cur_sha.clone(),
                commit_msg: msg,
                date,
            });
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        let key = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");
        if key.len() == 40 && key.chars().all(|c| c.is_ascii_hexdigit()) {
            cur_sha = key.to_string();
            let mut hdr = rest.split_whitespace();
            let _orig = hdr.next();
            if let Some(fl) = hdr.next() {
                cur_line = fl.parse().unwrap_or(0);
            }
            if let Some(cached) = commit_cache.get(&cur_sha) {
                cur_author = cached.0.clone();
                cur_summary = cached.1.clone();
            } else {
                cur_author.clear();
                cur_summary.clear();
                cur_time = 0;
            }
        } else if key == "author" {
            cur_author = rest.to_string();
        } else if key == "author-time" {
            cur_time = rest.parse().unwrap_or(0);
        } else if key == "summary" {
            cur_summary = rest.to_string();
        }
    }
    results
}

fn format_ts(ts: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
        .map(|d| d.to_rfc3339())
        .unwrap_or_default()
}

async fn fetch_prior_prs(
    client: &github::Client,
    slug: &Slug,
    shas: &[String],
    exclude: u64,
) -> Result<Vec<PriorPr>> {
    let mut unique: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for s in shas {
        if seen.insert(s.clone()) {
            unique.push(s.clone());
        }
        if unique.len() >= 10 {
            break;
        }
    }
    let mut out: Vec<PriorPr> = Vec::new();
    let mut seen_pr: std::collections::HashSet<u64> = std::collections::HashSet::new();
    for sha in unique {
        let url = format!(
            "https://api.github.com/repos/{}/{}/commits/{}/pulls",
            slug.owner, slug.name, sha
        );
        let v: serde_json::Value = match github::rest_json(client, reqwest::Method::GET, &url, None).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(arr) = v.as_array() {
            for p in arr {
                let n = p.get("number").and_then(|x| x.as_u64()).unwrap_or(0);
                if n == 0 || n == exclude {
                    continue;
                }
                if !seen_pr.insert(n) {
                    continue;
                }
                let title = p.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let url = p.get("html_url").and_then(|x| x.as_str()).unwrap_or("").to_string();
                out.push(PriorPr { number: n, title, url });
                if out.len() >= 5 {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

fn find_release_notes(repo_root: &Path, shas: &[String]) -> Option<String> {
    let candidates = ["CHANGELOG.md", "CHANGES.md", "HISTORY.md"];
    for c in candidates.iter() {
        let p = repo_root.join(c);
        if let Ok(text) = std::fs::read_to_string(&p) {
            for sha in shas {
                let short = &sha[..7.min(sha.len())];
                if text.contains(sha.as_str()) || text.contains(short) {
                    if let Some(section) = extract_changelog_section(&text, sha, short) {
                        return Some(section);
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::Slug;

    #[test]
    fn extracts_heading_section_for_path() {
        let body = "# Summary\nblah\n\n## src/auth.rs\ndetails about auth\nmore\n\n## Other\nnope";
        let s = extract_section_for_path(body, "src/auth.rs").unwrap();
        assert!(s.contains("## src/auth.rs"));
        assert!(s.contains("details about auth"));
        assert!(!s.contains("## Other"));
    }

    #[test]
    fn extracts_section_by_filename() {
        let body = "## auth.rs\nthe reason\n";
        let s = extract_section_for_path(body, "crates/tldr/src/auth.rs").unwrap();
        assert!(s.contains("the reason"));
    }

    #[test]
    fn no_section_returns_none() {
        assert!(extract_section_for_path("", "x").is_none());
        assert!(extract_section_for_path("no headings here", "x").is_none());
    }

    #[test]
    fn external_link_extraction() {
        let body = "See [linear task](https://linear.app/foo/TASK-1) and https://notion.so/page and https://example.com/ignored";
        let links = scan_external_links(body);
        assert!(links.iter().any(|l| l.url.contains("linear.app")));
        assert!(links.iter().any(|l| l.url.contains("notion.so")));
        assert!(!links.iter().any(|l| l.url.contains("example.com")));
    }

    #[test]
    fn issue_refs_local_and_xrepo() {
        let slug = Slug { owner: "o".into(), name: "r".into() };
        let body = "Fixes #123 and also foo/bar#456 but not xyz123";
        let issues = scan_body_issue_refs(body, &slug);
        assert!(issues.iter().any(|i| i.number == 123 && i.url.contains("o/r/issues/123")));
        assert!(issues.iter().any(|i| i.number == 456 && i.url.contains("foo/bar/issues/456")));
    }

    #[test]
    fn issue_ref_dedup() {
        let slug = Slug { owner: "o".into(), name: "r".into() };
        let body = "#5 #5 #5";
        let issues = scan_body_issue_refs(body, &slug);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn format_ts_zero() {
        assert_eq!(format_ts(0), "1970-01-01T00:00:00+00:00");
    }
}

fn extract_changelog_section(text: &str, sha: &str, short: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        if l.contains(sha) || l.contains(short) {
            let start = lines[..i]
                .iter()
                .rposition(|x| x.trim_start().starts_with('#'))
                .unwrap_or(i.saturating_sub(2));
            let end = lines[i + 1..]
                .iter()
                .position(|x| x.trim_start().starts_with('#'))
                .map(|j| i + 1 + j)
                .unwrap_or(lines.len().min(i + 20));
            return Some(lines[start..end].join("\n").trim_end().to_string());
        }
    }
    None
}
