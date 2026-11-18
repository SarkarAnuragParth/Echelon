use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::agent_runtime::{ToolCallContext, ToolDefinition};

pub mod external;
pub mod files;
pub mod git;
pub mod graph;
pub mod process;
pub mod skills;

// A group of tools that act on the same thing and share one collaborator. A
// tool's definition lives beside its implementation so the two cannot drift.
#[async_trait::async_trait]
pub trait ToolGroup: Send + Sync {
    fn definitions(&self) -> Vec<ToolDefinition>;

    // Ok is a successful call, Err a failed one. Both are model-facing text.
    async fn call(&self, name: &str, args: &Value, ctx: &ToolCallContext) -> Result<String, String>;
}

fn definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn parse_args<T: DeserializeOwned>(tool: &str, args: &Value) -> Result<T, String> {
    T::deserialize(args).map_err(|e| format!("ERROR: malformed arguments for {tool}: {e}"))
}

fn require_non_empty(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("ERROR: {field} cannot be empty"));
    }
    Ok(())
}
