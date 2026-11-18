use std::collections::HashMap;
use std::sync::{OnceLock};

use include_dir::{include_dir, Dir};
use serde::Deserialize;


#[derive(Debug, Deserialize)]
pub struct Module {
    pub id: String,
    pub lld: String,
    #[serde(default)]
    pub depends_on_modules: Vec<String>,
    pub tasks: HashMap<String, Task>,
}

#[derive(Debug, Deserialize,)]
pub struct Task {
    pub agent: String,
    pub story_points: u32,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub task_prompt: std::sync::Arc<str>,
    #[serde(default)]
    pub temperature: f32
}

pub fn load_module(path: std::path::PathBuf) -> Result<Module, Box<dyn std::error::Error>> {
    let yaml = std::fs::read_to_string(path)?;
    let module: Module = serde_yaml::from_str(&yaml)?;
    Ok(module)
}


static AGENTS: Dir = include_dir!("$CARGO_MANIFEST_DIR/prompts/agents");

#[derive(Debug, Deserialize)]
pub struct AgentFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub model: String,
}

pub struct AgentPrompt {
    pub frontmatter: AgentFrontmatter,
    pub system_prompt: &'static str,
}

type AgentPromptMap = HashMap<String, AgentPrompt>;

static AGENT_PROMPTS: OnceLock<AgentPromptMap> = OnceLock::new();

fn agent_prompts() -> &'static AgentPromptMap {
    AGENT_PROMPTS.get_or_init(|| {
        AGENTS
            .files()
            .map(|file| {
                let role = file
                    .path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.strip_suffix(".agent"))
                    .unwrap_or_else(|| panic!("unexpected file in prompts/agents: {}", file.path().display()))
                    .to_string();

                let content = std::str::from_utf8(file.contents())
                    .unwrap_or_else(|_| panic!("{} is not valid UTF-8", file.path().display()));

                (role, parse_agent_file(content, file.path()))
            })
            .collect()
    })
}

fn parse_agent_file(content: &'static str, path: &std::path::Path) -> AgentPrompt {
    let rest = content
        .strip_prefix("---\n")
        .unwrap_or_else(|| panic!("{}: must start with '---' frontmatter", path.display()));
    let (fm_text, body) = rest
        .split_once("\n---\n")
        .unwrap_or_else(|| panic!("{}: missing closing '---' for frontmatter", path.display()));

    let frontmatter: AgentFrontmatter = serde_yaml::from_str(fm_text)
        .unwrap_or_else(|e| panic!("{}: malformed frontmatter: {e}", path.display()));

    AgentPrompt { frontmatter, system_prompt: body.trim_start_matches('\n') }
}

pub fn agent_system_prompt(role: &str) -> Option<&'static AgentPrompt> {
    agent_prompts().get(role)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pm_agent_file() {
        let pm = agent_system_prompt("pm").expect("pm agent file should exist");
        assert_eq!(pm.frontmatter.name, "Product Manager");
        assert!(pm.frontmatter.model.is_empty());
        assert!(pm.system_prompt.contains("custodian of functional truth"));
        // frontmatter must not leak into the returned system prompt
        assert!(!pm.system_prompt.contains("description:"));
    }

    #[test]
    fn unknown_role_is_none() {
        assert!(agent_system_prompt("nonexistent").is_none());
    }
}
