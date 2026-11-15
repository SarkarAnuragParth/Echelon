use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::agent_runtime::{ToolCall, ToolDefinition, Turn};
use crate::model_client::{ModelClient, ModelConfig, ModelResponse, ModelCallError};
// come from the module you pasted.

pub enum MockStep {
    /// Plain text, no tool calls.
    Text(String),
    /// One or more tool calls in a single model turn: (tool name, arguments).
    ToolCalls(Vec<(String, Value)>),
    /// Simulate a provider failure.
    Error(ModelCallError),
}

pub struct MockModelClient {
    script: Mutex<VecDeque<MockStep>>,
    next_id: AtomicU64,
    /// Returned once the script runs out.
    fallback_text: String,
}

impl MockModelClient {
    pub fn new(steps: Vec<MockStep>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(steps.into()),
            next_id: AtomicU64::new(0),
            fallback_text: "mock: script exhausted".to_string(),
        })
    }

    pub fn all_tools_script() -> Arc<Self> {
        Self::new(vec![
            Self::tool("view_project_structure", json!({})),
            Self::tool(
                "create_file",
                json!({
                    "file_path": "mock/notes.md",
                    "file_purpose": "Scratch file created by the mock model to exercise file tools",
                    "initial_content": "# Notes\n"
                }),
            ),
            Self::tool(
                "read",
                json!({ "file_path": "mock/notes.md", "include_git_info": false }),
            ),
            // create_file returns version 1, so this write is valid without a fresh read.
            Self::tool(
                "write",
                json!({
                    "file_path": "mock/notes.md",
                    "content": "# Notes\n\nUpdated by the mock model.\n",
                    "version": 1
                }),
            ),
            Self::tool(
                "update_terminology",
                json!({
                    "term": "Mock Run",
                    "definition": "A test execution where a scripted client replaces the real model."
                }),
            ),
            Self::tool("bash", json!({ "command": "echo hello from mock" })),
            Self::tool("commit", json!({ "message": "test(mock): add notes.md via mock run" })),
            Self::tool("load_skill", json!({ "name": "example-skill" })),
            Self::tool(
                "request_skill",
                json!({ "need": "Mock request: need a skill covering X for tasks A and B." }),
            ),
            Self::tool("web_search", json!({ "query": "rust async trait objects", "topic": "general" })),
            Self::tool(
                "ask_user",
                json!({
                    "question": "Should the mock run continue?",
                    "recommendation": "Yes, continue; this is a test."
                }),
            ),
            Self::tool(
                "complete_task",
                json!({ "summary": "Mock run exercised every tool and completed." }),
            ),
        ])
    }

    /// Convenience: single tool call step.
    pub fn tool(name: &str, args: Value) -> MockStep {
        MockStep::ToolCalls(vec![(name.to_string(), args)])
    }
}

#[async_trait::async_trait]
impl ModelClient for MockModelClient {
    async fn call(
        &self,
        _system_prompt: &str,
        _conversation: &[Turn],
        _tools: &[ToolDefinition],
        _config: &ModelConfig,
    ) -> Result<ModelResponse, ModelCallError> {
        // Guard is dropped at the end of this statement, before any await.
        let step = self.script.lock().unwrap().pop_front();

        match step {
            Some(MockStep::Text(t)) => Ok(ModelResponse {
                raw: json!({ "mock": true, "text": t }),
                text: Some(t),
                tool_calls: vec![],
            }),
            Some(MockStep::ToolCalls(calls)) => {
                let tool_calls: Vec<ToolCall> = calls
                    .into_iter()
                    .map(|(name, arguments)| ToolCall {
                        id: format!("mock_call_{}", self.next_id.fetch_add(1, Ordering::Relaxed)),
                        name,
                        arguments,
                    })
                    .collect();
                Ok(ModelResponse {
                    raw: json!({ "mock": true, "tool_calls": tool_calls.len() }),
                    text: None,
                    tool_calls,
                })
            }
            Some(MockStep::Error(e)) => Err(e),
            None => Ok(ModelResponse {
                raw: json!({ "mock": true, "exhausted": true }),
                text: Some(self.fallback_text.clone()),
                tool_calls: vec![],
            }),
        }
    }
}