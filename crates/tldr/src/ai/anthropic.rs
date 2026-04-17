use super::{AiConfig, Provider, ProviderEvent, Usage, SYSTEM_PROMPT, build_user_prompt};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};

pub struct AnthropicProvider {
    api_key: String,
    model: String,
    endpoint: String,
}

impl AnthropicProvider {
    pub fn from_config(cfg: &AiConfig) -> Result<Self> {
        let api_key = std::env::var(&cfg.api_key_env)
            .map_err(|_| anyhow!("env var {} not set", cfg.api_key_env))?;
        let endpoint = if cfg.endpoint.is_empty() {
            "https://api.anthropic.com/v1/messages".to_string()
        } else {
            cfg.endpoint.clone()
        };
        Ok(Self { api_key, model: cfg.model.clone(), endpoint })
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn generate_walkthrough(
        &self,
        diff: &str,
        pr_desc: &str,
        context: &str,
    ) -> Result<BoxStream<'static, Result<ProviderEvent>>> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "stream": true,
            "system": SYSTEM_PROMPT,
            "messages": [{"role": "user", "content": build_user_prompt(diff, pr_desc, context)}],
        });
        let client = reqwest::Client::new();
        let resp = client.post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("anthropic error: {} {}", resp.status(), resp.text().await.unwrap_or_default()));
        }
        let model = self.model.clone();
        let stream = resp.bytes_stream();
        let out = async_stream::try_stream! {
            let mut buf = String::new();
            let mut in_tokens: u64 = 0;
            let mut out_tokens: u64 = 0;
            futures::pin_mut!(stream);
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                buf.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(idx) = buf.find("\n\n") {
                    let event_block: String = buf.drain(..idx+2).collect();
                    for line in event_block.lines() {
                        let line = line.trim();
                        if let Some(data) = line.strip_prefix("data:") {
                            let data = data.trim();
                            let Ok(v): Result<serde_json::Value, _> = serde_json::from_str(data) else { continue };
                            let t = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
                            if t == "content_block_delta" {
                                if let Some(text) = v.pointer("/delta/text").and_then(|x| x.as_str()) {
                                    yield ProviderEvent::Text(text.to_string());
                                }
                            } else if t == "message_start" {
                                if let Some(n) = v.pointer("/message/usage/input_tokens").and_then(|x| x.as_u64()) { in_tokens = n; }
                            } else if t == "message_delta" {
                                if let Some(n) = v.pointer("/usage/output_tokens").and_then(|x| x.as_u64()) { out_tokens = n; }
                            }
                        }
                    }
                }
            }
            yield ProviderEvent::Usage(Usage {
                input_tokens: in_tokens,
                output_tokens: out_tokens,
                provider: "anthropic".into(),
                model: model.clone(),
            });
        };
        Ok(Box::pin(out))
    }
}
