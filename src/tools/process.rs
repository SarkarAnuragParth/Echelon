use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{json, Value};

use super::{definition, git, parse_args, ToolGroup};
use crate::agent_runtime::{ToolCallContext, ToolDefinition};
use crate::sandbox::NoSandbox;

// Implemented by the sandbox. bash is the one tool with unbounded capability,
// which is why it goes through the sandbox and commit does not.
#[async_trait::async_trait]
pub trait CommandExecutor: Send + Sync {
    async fn run(&self, command: &str, ctx: &ToolCallContext) -> String;
}

#[derive(Deserialize)]
struct BashArgs {
    command: String,
}

#[derive(Deserialize)]
struct CommitArgs {
    message: String,
}

// Tools that run a process: the sandboxed shell, and git on the host.
pub struct ProcessTools {
    project_root: PathBuf,
    commands: NoSandbox,
}

impl ProcessTools {
    pub fn new(project_root: PathBuf) -> Self {
        let commands = NoSandbox::new(project_root.clone());
        Self { project_root, commands }
    }

    async fn bash(&self, args: &Value, ctx: &ToolCallContext) -> Result<String, String> {
        let args: BashArgs = parse_args("bash", args)?;
        Ok(self.commands.run(&args.command, ctx).await)
    }

    async fn commit(&self, args: &Value, ctx: &ToolCallContext) -> Result<String, String> {
        let args: CommitArgs = parse_args("commit", args)?;

        let root = &self.project_root;

        // Setting the identity makes author and committer the agent role, so
        // blame attributes lines to roles and commit works with no git identity set.
        let name = format!("user.name={}", ctx.agent_role);
        let email = format!("user.email={}@echelon.local", ctx.agent_role);

        git::run(root, "add", &["add", "-A"]).await?;
        git::run(root, "commit", &["-c", &name, "-c", &email, "commit", "-m", &args.message]).await?;
        let summary = git::run(root, "show", &["show", "--stat", "HEAD"]).await?;

        Ok(format!("COMMITTED: {}\n{summary}", args.message))
    }
}

#[async_trait::async_trait]
impl ToolGroup for ProcessTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![
            definition(
                "bash",
                concat!(
                    "Execute a shell command in the project sandbox. ",
                    "Use for running tests, installing dependencies, linting, build commands, security scanners, and any other shell operation your task requires. ",
                    "Returns stdout, stderr, and exit code. A non-zero exit code indicates failure — read the error output and fix before proceeding. ",
                    "Commands are executed from the project root. Timeout: 120 seconds.",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The shell command to execute. Examples: 'pytest tests/auth_test.py -v', 'pip install bcrypt --break-system-packages', 'python -m mypy src/auth.py'"
                        }
                    },
                    "required": ["command"]
                }),
            ),
            definition(
                "commit",
                concat!(
                    "Stage all current changes and commit them to version control with the given message. ",
                    "Call this after every focused unit of work — one commit per task or functionality. ",
                    "Do not accumulate multiple unrelated changes before committing. ",
                    "Returns the commit diff summary. Fails with an error if there is nothing to commit or if not inside a git repository.",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "message": {
                            "type": "string",
                            "description": "A descriptive commit message explaining what changed and why. Follow conventional commit format where possible: 'feat(auth): add empty-string guard to validate_token' or 'fix(config): correct Redis TTL key name'. Do not use generic messages like 'updates' or 'changes'."
                        }
                    },
                    "required": ["message"]
                }),
            ),
        ]
    }

    async fn call(&self, name: &str, args: &Value, ctx: &ToolCallContext) -> Result<String, String> {
        match name {
            "bash" => self.bash(args, ctx).await,
            "commit" => self.commit(args, ctx).await,
            other => Err(format!("ERROR: unknown tool '{other}'")),
        }
    }
}
