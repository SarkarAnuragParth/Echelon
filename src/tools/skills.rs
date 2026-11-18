use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{json, Value};

use super::{definition, parse_args, ToolGroup};
use crate::agent_runtime::{ToolCallContext, ToolDefinition};

#[derive(Deserialize)]
struct LoadSkillArgs {
    name: String,
}

// The skill library on disk. request_skill lives with the graph tools, since
// it enqueues a node rather than reading anything.
pub struct SkillTools {
    skills_dir: PathBuf,
}

impl SkillTools {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self { skills_dir }
    }

    async fn load_skill(&self, args: &Value) -> Result<String, String> {
        let args: LoadSkillArgs = parse_args("load_skill", args)?;

        // The name becomes a path segment, so it must be a plain identifier.
        if args.name.is_empty() || args.name.contains(['/', '\\']) || args.name.contains("..") {
            return Err(format!("ERROR: invalid skill name '{}'", args.name));
        }

        tokio::fs::read_to_string(self.skills_dir.join(&args.name).join("SKILL.md"))
            .await
            .map_err(|e| format!("ERROR: could not load skill '{}': {e}", args.name))
    }
}

#[async_trait::async_trait]
impl ToolGroup for SkillTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![definition(
            "load_skill",
            concat!(
                "Load the full content of a skill into your context. ",
                "At task start you receive an <available_skills> block listing every skill you can access by name and description only. ",
                "When you identify a skill relevant to your current task, call load_skill() with its name to retrieve the full skill content. ",
                "If the skill body references supporting files (ref/ files or scripts), use read() or bash() to access those — they are not loaded automatically. ",
                "Pre-loaded skills are already in your context and do not need to be loaded.",
            ),
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The skill name exactly as it appears in your <available_skills> block. Use the kebab-case identifier, not the display title."
                    }
                },
                "required": ["name"]
            }),
        )]
    }

    async fn call(&self, name: &str, args: &Value, _ctx: &ToolCallContext) -> Result<String, String> {
        match name {
            "load_skill" => self.load_skill(args).await,
            other => Err(format!("ERROR: unknown tool '{other}'")),
        }
    }
}
