use anyhow::Result;
use serde_json::json;

use super::{graphql, rest_json, Client};
use crate::draft::Draft;
use crate::repo::Slug;

pub async fn submit(c: &Client, slug: &Slug, pr_node_id: &str, head_sha: &str, draft: &Draft) -> Result<serde_json::Value> {
    let event = match draft.verdict.as_deref() {
        Some("approve") => "APPROVE",
        Some("request_changes") => "REQUEST_CHANGES",
        _ => "COMMENT",
    };
    let threads: Vec<_> = draft.comments.iter().map(|c| json!({
        "path": c.path,
        "line": c.line,
        "side": c.side,
        "body": c.body,
    })).collect();

    let q = r#"mutation($input: AddPullRequestReviewInput!) {
      addPullRequestReview(input: $input) { pullRequestReview { id url state } }
    }"#;
    let vars = json!({
        "input": {
            "pullRequestId": pr_node_id,
            "event": event,
            "body": draft.body,
            "commitOID": head_sha,
            "threads": threads,
        }
    });
    let v = graphql(c, q, vars).await?;

    // Push viewed state per file (REST).
    for (path, st) in &draft.file_state {
        if st.viewed {
            let url = format!(
                "https://api.github.com/repos/{}/{}/pulls/{}/files/{}/viewed",
                slug.owner, slug.name, draft.pr, urlencoding(path)
            );
            // fire and forget; ignore failures
            let _: Result<serde_json::Value> = rest_json(c, reqwest::Method::PUT, &url, Some(json!({}))).await;
        }
    }
    Ok(v)
}

fn urlencoding(s: &str) -> String {
    s.chars().map(|ch| match ch {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => ch.to_string(),
        c => format!("%{:02X}", c as u32),
    }).collect()
}
