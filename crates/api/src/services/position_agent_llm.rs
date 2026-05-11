//! LLM provider abstraction for position-agent answers.

use crate::error::{ApiError, ApiResult};
use crate::models::{AgentLlmContext, AgentLlmReplyMeta};
use async_trait::async_trait;
use serde_json::json;
use std::sync::LazyLock;

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .expect("reqwest client for position_agent_llm")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderMode {
    Disabled,
    OpenAiCompatible,
}

impl ProviderMode {
    fn from_env() -> Self {
        let raw = std::env::var("CLMM_AGENT_LLM_MODE")
            .ok()
            .unwrap_or_else(|| "disabled".to_string());
        match raw.trim().to_ascii_lowercase().as_str() {
            "openai_compatible" | "openai-compatible" | "openai" => Self::OpenAiCompatible,
            _ => Self::Disabled,
        }
    }
}

#[async_trait]
trait LlmProvider {
    async fn complete(
        &self,
        position_address: &str,
        prompt: &str,
        context: Option<&AgentLlmContext>,
    ) -> ApiResult<(String, AgentLlmReplyMeta)>;
}

struct DisabledFallbackProvider;

fn fallback_reply(position_address: &str, prompt: &str) -> String {
    format!(
        "Analiza fallback dla {position_address}: przyjmuje '{prompt}'. Rekomenduje porownac zakres z oknami 7d/30d i warianty +/-1.5%, +/-2.5%, +/-4.0%."
    )
}

#[async_trait]
impl LlmProvider for DisabledFallbackProvider {
    async fn complete(
        &self,
        position_address: &str,
        prompt: &str,
        _context: Option<&AgentLlmContext>,
    ) -> ApiResult<(String, AgentLlmReplyMeta)> {
        Ok((
            fallback_reply(position_address, prompt),
            AgentLlmReplyMeta {
                provider: "disabled_fallback".to_string(),
                used_fallback: true,
                model: None,
            },
        ))
    }
}

struct OpenAiCompatibleProvider {
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAiCompatibleProvider {
    fn from_env() -> Option<Self> {
        let base_url = std::env::var("CLMM_AGENT_LLM_URL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())?;
        let api_key = std::env::var("CLMM_AGENT_LLM_API_KEY")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())?;
        let model = std::env::var("CLMM_AGENT_LLM_MODEL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "gpt-4o-mini".to_string());
        Some(Self {
            base_url,
            api_key,
            model,
        })
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn complete(
        &self,
        position_address: &str,
        prompt: &str,
        context: Option<&AgentLlmContext>,
    ) -> ApiResult<(String, AgentLlmReplyMeta)> {
        let ctx = context
            .map(|c| serde_json::to_string(c).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|| "{}".to_string());
        let body = json!({
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": "You are a concise LP strategy assistant. Return practical suggestions with tradeoffs."
                },
                {
                    "role": "user",
                    "content": format!(
                        "position_address={position_address}\ncontext={ctx}\nquestion={prompt}\nRespond in Polish, max 5 sentences."
                    )
                }
            ],
            "temperature": 0.2
        });
        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );
        let resp = HTTP
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ApiError::service_unavailable(format!("LLM request failed: {e}")))?;
        let status = resp.status();
        let payload: serde_json::Value = resp.json().await.map_err(|e| {
            ApiError::service_unavailable(format!("LLM invalid JSON response: {e}"))
        })?;
        if !status.is_success() {
            return Err(ApiError::service_unavailable(format!(
                "LLM HTTP {}: {}",
                status, payload
            )));
        }
        let text = payload
            .get("choices")
            .and_then(|x| x.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApiError::service_unavailable("LLM empty answer".to_string()))?
            .to_string();
        Ok((
            text,
            AgentLlmReplyMeta {
                provider: "openai_compatible".to_string(),
                used_fallback: false,
                model: Some(self.model.clone()),
            },
        ))
    }
}

pub async fn generate_agent_reply(
    position_address: &str,
    prompt: &str,
    context: Option<&AgentLlmContext>,
) -> ApiResult<(String, AgentLlmReplyMeta)> {
    match ProviderMode::from_env() {
        ProviderMode::Disabled => {
            DisabledFallbackProvider
                .complete(position_address, prompt, context)
                .await
        }
        ProviderMode::OpenAiCompatible => {
            if let Some(provider) = OpenAiCompatibleProvider::from_env() {
                match provider.complete(position_address, prompt, context).await {
                    Ok(v) => Ok(v),
                    Err(_) => {
                        DisabledFallbackProvider
                            .complete(position_address, prompt, context)
                            .await
                    }
                }
            } else {
                DisabledFallbackProvider
                    .complete(position_address, prompt, context)
                    .await
            }
        }
    }
}
