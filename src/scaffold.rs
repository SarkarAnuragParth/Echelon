use std::path::Path;

use include_dir::{include_dir, Dir};

// Baked into the binary at compile time — no runtime path to the package
// install location is ever needed to find these.
static TEMPLATE: Dir = include_dir!("$CARGO_MANIFEST_DIR/scaffold-template");

// Created even if the template has no files in them (git and zip drop empty dirs).
const DIRS: [&str; 6] = [
    ".echelon",
    ".echelon/conversations",
    ".echelon/dumps",
    ".echelon/skills",
    ".echelon/state",
    "src",
];

// Writes the scaffold template into the project folder, overwriting every
// template file that already exists. No comparison, no merging, no exceptions.
// Only call this on a fresh project: running it on a project that has already
// been worked on replaces whatever the agents wrote into those files.
pub async fn ensure(project_root: &Path) -> Result<(), String> {
    let project_root = project_root.to_path_buf();
    tokio::task::spawn_blocking(move || ensure_blocking(&project_root))
        .await
        .map_err(|e| format!("ERROR: scaffold task panicked: {e}"))?
}

fn ensure_blocking(project_root: &Path) -> Result<(), String> {
    for dir in DIRS {
        std::fs::create_dir_all(project_root.join(dir))
            .map_err(|e| format!("ERROR: could not create '{dir}': {e}"))?;
    }

    write_template(project_root, &TEMPLATE)?;
    ensure_git_repo(project_root)
}

fn write_template(project_root: &Path, dir: &Dir) -> Result<(), String> {
    for file in dir.files() {
        let target = project_root.join(file.path());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("ERROR: could not create '{}': {e}", parent.display()))?;
        }
        std::fs::write(&target, file.contents())
            .map_err(|e| format!("ERROR: could not write '{}': {e}", file.path().display()))?;
    }

    for subdir in dir.dirs() {
        std::fs::create_dir_all(project_root.join(subdir.path()))
            .map_err(|e| format!("ERROR: could not create '{}': {e}", subdir.path().display()))?;
        write_template(project_root, subdir)?;
    }

    Ok(())
}

fn ensure_git_repo(project_root: &Path) -> Result<(), String> {
    if project_root.join(".git").exists() {
        return Ok(());
    }
    std::process::Command::new("git")
        .arg("init")
        .current_dir(project_root)
        .output()
        .map_err(|e| format!("ERROR: could not run git init: {e}"))
        .and_then(|output| {
            if output.status.success() {
                Ok(())
            } else {
                Err(format!("ERROR: git init failed: {}", String::from_utf8_lossy(&output.stderr)))
            }
        })
}