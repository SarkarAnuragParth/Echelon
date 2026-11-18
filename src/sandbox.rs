//! Placeholder: runs bash() commands directly on the host with no isolation.
//! To be replaced by the microVM sandbox; the CommandExecutor trait stays the same.

use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;

use crate::agent_runtime::ToolCallContext;
use crate::tools::process::CommandExecutor;

const TIMEOUT: Duration = Duration::from_secs(120);

pub struct NoSandbox {
    project_root: PathBuf,
}

impl NoSandbox {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }
}

#[cfg(windows)]
fn shell(command: &str) -> Command {
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", command]);
    cmd
}

#[cfg(not(windows))]
fn shell(command: &str) -> Command {
    let mut cmd = Command::new("bash");
    cmd.args(["-c", command]);
    cmd
}

#[async_trait::async_trait]
impl CommandExecutor for NoSandbox {
    async fn run(&self, command: &str, _ctx: &ToolCallContext) -> String {
        let mut cmd = shell(command);
        // kill_on_drop makes the timeout below actually kill the shell.
        cmd.current_dir(&self.project_root).kill_on_drop(true);

        let output = match tokio::time::timeout(TIMEOUT, cmd.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => return format!("ERROR: could not run command: {e}"),
            Err(_) => return format!("ERROR: command timed out after {}s and was killed", TIMEOUT.as_secs()),
        };

        let exit_code = output
            .status
            .code()
            .map_or_else(|| "terminated by signal".to_string(), |c| c.to_string());

        format!(
            "exit code: {exit_code}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout).trim_end(),
            String::from_utf8_lossy(&output.stderr).trim_end(),
        )
    }
}