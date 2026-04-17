pub mod pulls;
pub mod review;

use anyhow::{anyhow, Result};
use reqwest::header;
use serde::{Deserialize, Serialize};

use crate::repo::Slug;

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrMeta {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub head_sha: String,
    pub base_sha: String,
    pub head_ref: String,
    pub base_ref: String,
    pub node_id: String,
    pub author: Option<String>,
    pub html_url: String,
}

impl Client {
    pub fn new(token: String) -> Self {
        let mut h = header::HeaderMap::new();
        h.insert(header::ACCEPT, "application/vnd.github+json".parse().unwrap());
        h.insert(header::USER_AGENT, "tldr-cli".parse().unwrap());
        let http = reqwest::Client::builder().default_headers(h).build().unwrap();
        Self { http, token }
    }

    pub async fn fetch_pr(&self, slug: &Slug, n: u64) -> Result<PrMeta> {
        pulls::fetch(self, slug, n).await
    }

    pub(crate) fn bearer(&self) -> String { format!("Bearer {}", self.token) }
    pub(crate) fn http(&self) -> &reqwest::Client { &self.http }
}

pub(crate) async fn rest_json<T: serde::de::DeserializeOwned>(
    c: &Client,
    method: reqwest::Method,
    url: &str,
    body: Option<serde_json::Value>,
) -> Result<T> {
    let mut rb = c.http().request(method, url).header("Authorization", c.bearer());
    if let Some(b) = body { rb = rb.json(&b); }
    let resp = rb.send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("github {status}: {text}"));
    }
    Ok(resp.json().await?)
}

pub(crate) async fn graphql(
    c: &Client,
    query: &str,
    variables: serde_json::Value,
) -> Result<serde_json::Value> {
    let resp = c.http()
        .post("https://api.github.com/graphql")
        .header("Authorization", c.bearer())
        .json(&serde_json::json!({ "query": query, "variables": variables }))
        .send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("graphql {status}: {text}"));
    }
    let v: serde_json::Value = resp.json().await?;
    if let Some(errs) = v.get("errors") {
        return Err(anyhow!("graphql errors: {errs}"));
    }
    Ok(v)
}
