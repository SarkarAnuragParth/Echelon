use std::collections::HashMap;
use std::path::{Path};
use std::sync::Arc;
use std::sync::Mutex;
use crate::agent_runtime::{ToolCall, ToolCallContext, ToolDefinition, ToolResult};
use crate::tools::external::{ExternalTools};
use crate::tools::files::FileTools;
use crate::tools::graph::{Graph, GraphTools};
use crate::tools::process::{ProcessTools};
use crate::tools::skills::SkillTools;
use crate::tools::ToolGroup;
pub struct ToolExecutionHandler {
    groups: Vec<Box<dyn ToolGroup>>,
    // Tool name to the index of the group that owns it.
    routes: HashMap<String, usize>,
}

impl ToolExecutionHandler {
    pub fn new(
        project_root: &Path,
        graph: Arc<Mutex<Graph>>
    ) -> Self {
        let groups: Vec<Box<dyn ToolGroup>> = vec![
            Box::new(FileTools::load(project_root.to_path_buf())),
            Box::new(SkillTools::new(project_root.join(".echelon/skills"))),
            Box::new(GraphTools::new(graph)),
            Box::new(ProcessTools::new(project_root.to_path_buf())),
            Box::new(ExternalTools::new()),
        ];

        let mut routes = HashMap::new();
        for (index, group) in groups.iter().enumerate() {
            for definition in group.definitions() {
                let name = definition.name;
                assert!(routes.insert(name.clone(), index).is_none(), "duplicate tool name: {name}");
            }
        }

        Self { groups, routes }
    }

    // Every tool this handler can run. The orchestrator picks each role's
    // subset from this by name.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.groups.iter().flat_map(|group| group.definitions()).collect()
    }

    pub async fn execute(&self, call: &ToolCall, ctx: &ToolCallContext) -> ToolResult {
        let result = match self.routes.get(&call.name) {
            Some(&index) => self.groups[index].call(&call.name, &call.arguments, ctx).await,
            None => Err(format!("ERROR: unknown tool '{}'", call.name)),
        };

        let (output, is_error) = match result {
            Ok(output) => (output, false),
            Err(output) => (output, true),
        };

        ToolResult {
            tool_call_id: call.id.clone(),
            output,
            is_error,
        }
    }
}


