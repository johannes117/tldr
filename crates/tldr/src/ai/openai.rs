use super::{AiConfig, Provider, ProviderEvent, Usage, SYSTEM_PROMPT, build_user_prompt};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};

pub struct OpenAiProvider {
    api_key: String,
    model: String,
    endpoint: String,
    provider_name: String,
}

impl OpenAiProvider {
    pub fn from_config(cfg: &AiConfig) -> Result<Self> {
        let api_key = std::env::var(&cfg.api_key_env)
            .map_err(|_| anyhow!("env var {} not set", cfg.api_key_env))?;
        let endpoint = if cfg.endpoint.is_empty() {
            "https://api.openai.com/v1/chat/completions".to_string()
        } else { cfg.endpoint.clone() };
        Ok(Self { api_key, model: cfg.model.clone(), endpoint, provider_name: "openai".into() })
    }

    pub fn with_name(api_key: String, model: String, endpoint: String, name: &str) -> Self {
        Self { api_key, model, endpoint, provider_name: name.into() }
    }
}

pub(crate) fn sse_openai_stream(
    resp: reqwest::Response,
    model: String,
    provider_name: String,
) -> BoxStream<'static, Result<ProviderEvent>> {
    let stream = resp.bytes_stream();
    let out = async_stream::try_stream! {
        let mut buf = String::new();
        let mut in_tokens: u64 = 0;
        let mut out_tokens: u64 = 0;
        futures::pin_mut!(stream);
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(idx) = buf.find('\n') {
                let line: String = buf.drain(..idx+1).collect();
                let line = line.trim();
                let Some(data) = line.strip_prefix("data:") else { continue };
                let data = data.trim();
                if data == "[DONE]" { continue; }
                let Ok(v): Result<serde_json::Value, _> = serde_json::from_str(data) else { continue };
                if let Some(text) = v.pointer("/choices/0/delta/content").and_then(|x| x.as_str()) {
                    yield ProviderEvent::Text(text.to_string());
                }
                if let Some(n) = v.pointer("/usage/prompt_tokens").and_then(|x| x.as_u64()) { in_tokens = n; }
                if let Some(n) = v.pointer("/usage/completion_tokens").and_then(|x| x.as_u64()) { out_tokens = n; }
            }
        }
        yield ProviderEvent::Usage(Usage {
            input_tokens: in_tokens,
            output_tokens: out_tokens,
            provider: provider_name.clone(),
            model: model.clone(),
        });
    };
    Box::pin(out)
}

#[async_trait]
impl Provider for OpenAiProvider {
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
        let resp = reqwest::Client::new().post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("openai error: {} {}", resp.status(), resp.text().await.unwrap_or_default()));
        }
        Ok(sse_openai_stream(resp, self.model.clone(), self.provider_name.clone()))
    }
}
