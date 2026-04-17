use super::{AiConfig, Provider, ProviderEvent, Usage, SYSTEM_PROMPT, build_user_prompt};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};

pub struct OllamaProvider {
    model: String,
    endpoint: String,
}

impl OllamaProvider {
    pub fn from_config(cfg: &AiConfig) -> Self {
        let endpoint = if cfg.endpoint.is_empty() {
            "http://localhost:11434/api/chat".to_string()
        } else { cfg.endpoint.clone() };
        Self { model: cfg.model.clone(), endpoint }
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    async fn generate_walkthrough(
        &self,
        diff: &str,
        pr_desc: &str,
        context: &str,
    ) -> Result<BoxStream<'static, Result<ProviderEvent>>> {
        let body = serde_json::json!({
            "model": self.model,
            "stream": true,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": build_user_prompt(diff, pr_desc, context)},
            ],
        });
        let resp = reqwest::Client::new().post(&self.endpoint).json(&body).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("ollama error: {} {}", resp.status(), resp.text().await.unwrap_or_default()));
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
                while let Some(idx) = buf.find('\n') {
                    let line: String = buf.drain(..idx+1).collect();
                    let line = line.trim();
                    if line.is_empty() { continue; }
                    let Ok(v): Result<serde_json::Value, _> = serde_json::from_str(line) else { continue };
                    if let Some(text) = v.pointer("/message/content").and_then(|x| x.as_str()) {
                        if !text.is_empty() { yield ProviderEvent::Text(text.to_string()); }
                    }
                    if let Some(n) = v.get("prompt_eval_count").and_then(|x| x.as_u64()) { in_tokens = n; }
                    if let Some(n) = v.get("eval_count").and_then(|x| x.as_u64()) { out_tokens = n; }
                }
            }
            yield ProviderEvent::Usage(Usage {
                input_tokens: in_tokens,
                output_tokens: out_tokens,
                provider: "ollama".into(),
                model: model.clone(),
            });
        };
        Ok(Box::pin(out))
    }
}
