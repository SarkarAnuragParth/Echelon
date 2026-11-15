use std::sync::Arc;

use serde_json::Value;
use thiserror::Error;

use crate::agent_runtime::{ToolCall, ToolDefinition, Turn};

#[derive(Debug, Clone)]
pub struct ModelConfig {
    pub provider: String,
    pub model: String,
    pub temperature: Option<f32>,
    // Some providers (Qwen3-family and others) expose a dual thinking /
    // non-thinking mode. None means "use the provider's default."
    pub enable_thinking: Option<bool>,
}


#[derive(Debug, Error)]
pub enum ModelCallError {
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("provider returned an unparseable response: {0}")]
    MalformedResponse(String),
    #[error("rate limited by provider")]
    RateLimited,
}

pub struct ModelResponse {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub raw: Value,
}

// Provider-agnostic — swapping providers/models means implementing this
// trait, not touching AgentRuntime.
#[async_trait::async_trait]
pub trait ModelClient: Send + Sync {
    async fn call(
        &self,
        system_prompt: &str,
        conversation: &[Turn],
        tools: &[ToolDefinition],
        config: &ModelConfig,
    ) -> Result<ModelResponse, ModelCallError>;
}

pub type SharedModelClient = Arc<dyn ModelClient>;
