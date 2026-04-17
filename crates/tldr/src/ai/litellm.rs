use super::{AiConfig, Provider, ProviderEvent, SYSTEM_PROMPT, build_user_prompt};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::stream::BoxStream;

pub struct LiteLlmProvider {
    api_key: String,
    model: String,
    endpoint: String,
}

impl LiteLlmProvider {
    pub fn from_config(cfg: &AiConfig) -> Result<Self> {
        if cfg.endpoint.is_empty() { return Err(anyhow!("litellm requires endpoint")); }
        let api_key = std::env::var(&cfg.api_key_env).unwrap_or_default();
        Ok(Self { api_key, model: cfg.model.clone(), endpoint: cfg.endpoint.clone() })
    }
}

#[async_trait]
impl Provider for LiteLlmProvider {
    async fn generate_walkthrough(
        &self,
        diff: &str,
        pr_desc: &str,
        context: &str,
    ) -> Result<BoxStream<'static, Result<ProviderEvent>>> {
        let body = serde_json::json!({
            "model": self.model,
            "stream": true,
            "stream_options": {"include_usage": true},
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": build_user_prompt(diff, pr_desc, context)},
            ],
        });
        let mut req = reqwest::Client::new().post(&self.endpoint).json(&body);
        if !self.api_key.is_empty() { req = req.bearer_auth(&self.api_key); }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("litellm error: {} {}", resp.status(), resp.text().await.unwrap_or_default()));
        }
        Ok(super::openai::sse_openai_stream(resp, self.model.clone(), "litellm".into()))
    }
}
