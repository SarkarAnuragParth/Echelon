use std::path::Path;

use tokio::process::Command;

// Runs git in the project root. Shared by commit and read's git info, so where
// git runs can be changed in one place.
pub async fn run(project_root: &Path, label: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(project_root)
        .output()
        .await
        .map_err(|e| format!("ERROR: could not run git: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status.success() {
        return Ok(stdout.into_owned());
    }

    // git reports "nothing to commit" on stdout, so both streams matter.
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("ERROR: git {label} failed:\n{}\n{}", stdout.trim(), stderr.trim()))
}
