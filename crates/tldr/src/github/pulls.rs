use anyhow::Result;
use serde_json::Value;

use super::{rest_json, Client, PrMeta};
use crate::repo::Slug;

pub async fn fetch(c: &Client, slug: &Slug, n: u64) -> Result<PrMeta> {
    let url = format!("https://api.github.com/repos/{}/{}/pulls/{n}", slug.owner, slug.name);
    let v: Value = rest_json(c, reqwest::Method::GET, &url, None).await?;
    Ok(PrMeta {
        number: v["number"].as_u64().unwrap_or(n),
        title: v["title"].as_str().unwrap_or("").into(),
        body: v["body"].as_str().map(str::to_string),
        state: v["state"].as_str().unwrap_or("open").into(),
        head_sha: v["head"]["sha"].as_str().unwrap_or("").into(),
        base_sha: v["base"]["sha"].as_str().unwrap_or("").into(),
        head_ref: v["head"]["ref"].as_str().unwrap_or("").into(),
        base_ref: v["base"]["ref"].as_str().unwrap_or("").into(),
        node_id: v["node_id"].as_str().unwrap_or("").into(),
        author: v["user"]["login"].as_str().map(str::to_string),
        html_url: v["html_url"].as_str().unwrap_or("").into(),
    })
}
