use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::tool_execution_handler::ToolExecutionHandler;
use crate::model_client::{ModelCallError, SharedModelClient, ModelConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub output: String,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub struct ToolCallContext {
    pub agent_role: String,
    pub node_id: String,
}


#[derive(Debug, Clone)]
pub enum RunOutcome {
    Completed,
    Failed(FailureReason),
}

// The conversation comes back on every exit path, failures included, so the
// caller can persist it.
pub struct RunResult {
    pub outcome: RunOutcome,
    pub(crate) conversation: Vec<Turn>,
}

#[derive(Debug, Clone)]
pub enum FailureReason {
    ModelError(String),
    // Sole backstop for both an unproductive loop and a genuinely stuck
    // agent — no separate stuck-detection exists.
    IterationCapExceeded,
    // Reasoning cut off mid-generation, no text or tool call produced.
    EmptyTurn,
    Crashed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Turn{
    User { content: Arc<str>},
    // raw is the provider response verbatim, reasoning included — never strip it.
    // tool_results are in call order, one per tool call inside raw.
    Assistant { raw: Value, tool_results: Vec<ToolResult> },
}

pub struct AgentRuntime;

impl AgentRuntime {
    pub async fn run<'a>(
        system_prompt: &str,
        tool_schema: Vec<ToolDefinition>,
        task_prompt: Arc<str>,
        agent_name: String,
        model_config: ModelConfig,
        model_client: SharedModelClient,
        tool_handler: Arc<ToolExecutionHandler>,
        node_id: String,
        iteration_cap: u32,
    ) -> RunResult {
        let mut conversation: Vec<Turn> = vec![Turn::User {
            content: task_prompt,
        }];

        let ctx = ToolCallContext {
            agent_role: agent_name.clone(),
            node_id: node_id.clone(),
        };

        let mut iterations: u32 = 0;

        loop {
            iterations += 1;
            if iterations > iteration_cap {
                return RunResult {
                    outcome: RunOutcome::Failed(FailureReason::IterationCapExceeded),
                    conversation,
                };
            }

            let response = match model_client
                .call(system_prompt, &conversation, &tool_schema, &model_config)
                .await
            {
                Ok(r) => r,
                Err(ModelCallError::RateLimited) => {
                    return RunResult {
                        outcome: RunOutcome::Failed(FailureReason::ModelError(
                            "rate limited".to_string(),
                        )),
                        conversation,
                    };
                }
                Err(e) => {
                    return RunResult {
                        outcome: RunOutcome::Failed(FailureReason::ModelError(e.to_string())),
                        conversation,
                    };
                }
            };

            let has_text = response.text.as_ref().is_some_and(|t| !t.trim().is_empty());

            if !has_text && response.tool_calls.is_empty() {
                // Not a valid stopping point — route to the existing
                // retry-with-reason path rather than looping on empty turns.
                return RunResult {
                    outcome: RunOutcome::Failed(FailureReason::EmptyTurn),
                    conversation,
                };
            }

            let mut tool_results = Vec::with_capacity(response.tool_calls.len());
            let mut saw_completion = false;

            for call in &response.tool_calls {
                let result = tool_handler.execute(call, &ctx).await;

                // create_exchange_node triggers complete_task internally per
                // its own tool semantics, so both end the loop. A failed call
                // does not: the model sees the error and retries.
                if !result.is_error && (call.name == "complete_task" || call.name == "create_exchange_node") {
                    saw_completion = true;
                }
                tool_results.push(result);
            }

            conversation.push(Turn::Assistant {
                raw: response.raw,
                tool_results,
            });

            if saw_completion {
                return RunResult {
                    outcome: RunOutcome::Completed,
                    conversation,
                };
            }
        }
    }
}
