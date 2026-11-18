use std::collections::{BTreeMap, HashMap};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use super::{definition, git, parse_args, require_non_empty, ToolGroup};
use crate::agent_runtime::{ToolCallContext, ToolDefinition};

const TERMINOLOGY_FILE: &str = ".echelon/terminology.md";

// Dependency and build folders that would flood the tree. Hidden folders are skipped too.
const SKIPPED_DIRS: &[&str] = &["node_modules", "target", "__pycache__", "dist", "build", "venv"];
const MAX_TREE_ENTRIES: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestEntry {
    purpose: String,
    version: u64,
}

#[derive(Deserialize)]
struct ReadArgs {
    file_path: String,
    #[serde(default)]
    include_git_info: bool,
}

#[derive(Deserialize)]
struct WriteArgs {
    file_path: String,
    content: String,
    version: u64,
}

#[derive(Deserialize)]
struct CreateFileArgs {
    file_path: String,
    file_purpose: String,
    #[serde(default)]
    initial_content: String,
}

#[derive(Deserialize)]
struct TermArgs {
    term: String,
    definition: String,
}

// Workspace files, all guarded by one manifest of purposes and versions.
pub struct FileTools {
    project_root: PathBuf,
    manifest_path: PathBuf,
    manifest: Mutex<HashMap<String, ManifestEntry>>,
}

impl FileTools {
    pub fn load(project_root: PathBuf) -> Self {
        let manifest_path = project_root.join(".echelon").join("manifest.json");
        let manifest = std::fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        Self {
            project_root,
            manifest_path,
            manifest: Mutex::new(manifest),
        }
    }

    // Lexical rather than canonicalize(): that needs the path to exist, so it
    // would skip the check for a create_file into a new nested folder.
    fn resolve_path(&self, file_path: &str) -> Result<PathBuf, String> {
        let joined = self.project_root.join(file_path);
        if !normalize_lexically(&joined).starts_with(normalize_lexically(&self.project_root)) {
            return Err("ERROR: path resolves outside project root".to_string());
        }
        Ok(joined)
    }

    async fn persist_manifest(&self, manifest: &HashMap<String, ManifestEntry>) {
        if let Some(parent) = self.manifest_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Ok(serialized) = serde_json::to_string_pretty(manifest) {
            let _ = tokio::fs::write(&self.manifest_path, serialized).await;
        }
    }

    async fn read(&self, args: &Value) -> Result<String, String> {
        let args: ReadArgs = parse_args("read", args)?;
        let path = self.resolve_path(&args.file_path)?;

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("ERROR: could not read '{}': {e}", args.file_path))?;

        let version = self
            .manifest
            .lock()
            .await
            .get(&args.file_path)
            .map_or(0, |e| e.version);

        let mut response = json!({ "content": content, "version": version });
        if args.include_git_info {
            response["git"] = self.git_info(&args.file_path).await;
        }
        Ok(response.to_string())
    }

    async fn git_info(&self, file_path: &str) -> Value {
        let log_args = ["log", "-5", "--format=%h%x09%an%x09%s", "--", file_path];
        let blame_args = ["blame", "--line-porcelain", "--", file_path];
        let (log, blame) = tokio::join!(
            git::run(&self.project_root, "log", &log_args),
            git::run(&self.project_root, "blame", &blame_args),
        );

        match (log, blame) {
            (Ok(log), Ok(blame)) => {
                json!({ "recent_commits": parse_log(&log), "line_ownership": parse_blame(&blame) })
            }
            // Blame fails for a file with no committed history yet.
            (Ok(log), Err(_)) => json!({ "recent_commits": parse_log(&log), "line_ownership": [] }),
            (Err(e), _) => json!({ "unavailable": e }),
        }
    }

    async fn write(&self, args: &Value) -> Result<String, String> {
        let args: WriteArgs = parse_args("write", args)?;
        let path = self.resolve_path(&args.file_path)?;

        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Err(format!(
                "ERROR: '{}' does not exist. Use create_file() for new files.",
                args.file_path
            ));
        }

        let mut manifest = self.manifest.lock().await;
        let (current_version, purpose) = manifest
            .get(&args.file_path)
            .map_or((0, String::new()), |e| (e.version, e.purpose.clone()));

        if current_version != args.version {
            let current_content = tokio::fs::read_to_string(&path).await.unwrap_or_default();
            return Err(json!({
                "error": "version conflict — file changed since your last read",
                "current_content": current_content,
                "current_version": current_version,
            })
            .to_string());
        }

        tokio::fs::write(&path, &args.content)
            .await
            .map_err(|e| format!("ERROR: write to '{}' failed: {e}", args.file_path))?;

        let new_version = current_version + 1;
        manifest.insert(
            args.file_path.clone(),
            ManifestEntry { purpose, version: new_version },
        );
        self.persist_manifest(&manifest).await;

        Ok(json!({ "written": args.file_path, "version": new_version }).to_string())
    }

    async fn create_file(&self, args: &Value) -> Result<String, String> {
        let args: CreateFileArgs = parse_args("create_file", args)?;
        let path = self.resolve_path(&args.file_path)?;

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("ERROR: could not create folder for '{}': {e}", args.file_path))?;
        }

        // create_new makes "does not exist yet" and "create" one atomic step.
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map_err(|e| match e.kind() {
                ErrorKind::AlreadyExists => {
                    format!("ERROR: '{}' already exists. Use write() to modify it.", args.file_path)
                }
                _ => format!("ERROR: could not create '{}': {e}", args.file_path),
            })?;

        file.write_all(args.initial_content.as_bytes())
            .await
            .map_err(|e| format!("ERROR: could not write '{}': {e}", args.file_path))?;

        let mut manifest = self.manifest.lock().await;
        manifest.insert(
            args.file_path.clone(),
            ManifestEntry { purpose: args.file_purpose, version: 1 },
        );
        self.persist_manifest(&manifest).await;

        Ok(json!({ "created": args.file_path, "version": 1 }).to_string())
    }

    async fn view_project_structure(&self) -> Result<String, String> {
        let purposes: HashMap<String, String> = self
            .manifest
            .lock()
            .await
            .iter()
            .map(|(path, entry)| (path.clone(), entry.purpose.clone()))
            .collect();
        let root = self.project_root.clone();

        tokio::task::spawn_blocking(move || render_tree(&root, &purposes))
            .await
            .map_err(|e| format!("ERROR: could not list project files: {e}"))
    }

    async fn update_terminology(&self, args: &Value) -> Result<String, String> {
        let args: TermArgs = parse_args("update_terminology", args)?;
        let (term, definition) = (args.term.trim(), args.definition.trim());
        require_non_empty("term", term)?;
        require_non_empty("definition", definition)?;

        let path = self.resolve_path(TERMINOLOGY_FILE)?;

        // Held across the read-modify-write so concurrent updates cannot interleave.
        let mut manifest = self.manifest.lock().await;
        let existing = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        let (updated, action) = upsert_term(&existing, term, definition);

        tokio::fs::write(&path, updated)
            .await
            .map_err(|e| format!("ERROR: could not update {TERMINOLOGY_FILE}: {e}"))?;

        let entry = manifest
            .entry(TERMINOLOGY_FILE.to_string())
            .or_insert_with(|| ManifestEntry {
                purpose: "project terminology".to_string(),
                version: 0,
            });
        entry.version += 1;
        self.persist_manifest(&manifest).await;

        Ok(format!("{action}: '{term}' in {TERMINOLOGY_FILE}"))
    }
}

fn parse_log(log: &str) -> Vec<Value> {
    log.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            Some(json!({ "commit": parts.next()?, "author": parts.next()?, "message": parts.next()? }))
        })
        .collect()
}

// --line-porcelain repeats the full header for every line, so one "author"
// header per line of the file gives per-author line counts.
fn parse_blame(blame: &str) -> Vec<Value> {
    let mut lines_by_author: BTreeMap<&str, usize> = BTreeMap::new();
    for author in blame.lines().filter_map(|line| line.strip_prefix("author ")) {
        *lines_by_author.entry(author).or_default() += 1;
    }

    let mut ownership: Vec<(&str, usize)> = lines_by_author.into_iter().collect();
    ownership.sort_by(|a, b| b.1.cmp(&a.1));
    ownership
        .into_iter()
        .map(|(author, lines)| json!({ "author": author, "lines": lines }))
        .collect()
}

struct Tree {
    lines: Vec<String>,
    truncated: bool,
}

fn render_tree(root: &Path, purposes: &HashMap<String, String>) -> String {
    let mut tree = Tree { lines: Vec::new(), truncated: false };
    walk(root, "", 0, purposes, &mut tree);

    if tree.lines.is_empty() {
        return "(empty)".to_string();
    }
    if tree.truncated {
        tree.lines.push(format!("... cut off after {MAX_TREE_ENTRIES} entries"));
    }
    tree.lines.join("\n")
}

// Symlinks are listed, never followed: file_type() does not resolve them.
fn walk(dir: &Path, rel: &str, depth: usize, purposes: &HashMap<String, String>, tree: &mut Tree) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else { continue };

        if file_type.is_dir() {
            if !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_str()) {
                dirs.push(name);
            }
        } else {
            files.push((name, file_type.is_symlink()));
        }
    }
    dirs.sort();
    files.sort();

    let indent = "  ".repeat(depth);
    let join = |name: &str| if rel.is_empty() { name.to_string() } else { format!("{rel}/{name}") };

    for name in dirs {
        if tree.lines.len() >= MAX_TREE_ENTRIES {
            tree.truncated = true;
            return;
        }
        tree.lines.push(format!("{indent}{name}/"));
        walk(&dir.join(&name), &join(&name), depth + 1, purposes, tree);
    }
    for (name, is_symlink) in files {
        if tree.lines.len() >= MAX_TREE_ENTRIES {
            tree.truncated = true;
            return;
        }
        let purpose = purposes
            .get(&join(&name))
            .filter(|p| !p.is_empty())
            .map(|p| format!(" — {p}"))
            .unwrap_or_default();
        let marker = if is_symlink { " (symlink, not followed)" } else { "" };
        tree.lines.push(format!("{indent}{name}{marker}{purpose}"));
    }
}

// Entries are `**term**: definition`, separated by blank lines. An existing
// term is replaced in place, its continuation lines dropped; a new one is appended.
fn upsert_term(existing: &str, term: &str, definition: &str) -> (String, &'static str) {
    let marker = format!("**{term}**");

    if !existing.lines().any(|line| line.starts_with(&marker)) {
        let separator = if existing.is_empty() || existing.ends_with("\n\n") { "" } else { "\n\n" };
        return (format!("{existing}{separator}{marker}: {definition}"), "ADDED");
    }

    let mut out: Vec<String> = Vec::new();
    let mut replacing = false;
    for line in existing.lines() {
        if line.starts_with(&marker) {
            out.push(format!("{marker}: {definition}"));
            replacing = true;
        } else if replacing && !line.trim().is_empty() && !line.starts_with("**") {
            // continuation of the definition being replaced
        } else {
            replacing = false;
            out.push(line.to_string());
        }
    }
    (out.join("\n"), "UPDATED")
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            other => result.push(other),
        }
    }
    result
}

#[async_trait::async_trait]
impl ToolGroup for FileTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![
            definition(
                "read",
                concat!(
                    "Read a file from the project. Returns JSON with the file's complete `content` and its current `version`. ",
                    "Always call read() before write(): write() requires the version returned here. ",
                    "With include_git_info it also returns `git`: the file's 5 most recent commits and how many lines each author currently owns (git blame; authors are agent roles). ",
                    "Use that to see who last worked on a file and why; it costs extra context, so leave it off otherwise. ",
                    "Use view_project_structure() first if unsure whether a file exists.",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Path to the file, relative to the project root. Example: 'src/auth/token.py'. Do not use absolute paths."
                        },
                        "include_git_info": {
                            "type": "boolean",
                            "description": "When true, also returns recent commits and per-author line ownership for the file. Defaults to false.",
                            "default": false
                        }
                    },
                    "required": ["file_path"]
                }),
            ),
            definition(
                "write",
                concat!(
                    "Overwrite an existing file with new, complete content. ",
                    "Always call read() first: write() requires the `version` that read() returned. ",
                    "If the file changed since you read it, the write is rejected and the response contains the file's current content and version — re-apply your change to that content and write again. ",
                    "This replaces the entire file; include everything that should remain, not just the changed portion. ",
                    "Make focused, single-purpose changes: one task per write() call. ",
                    "Use create_file() for files that do not yet exist. ",
                    "After a successful write, call commit() with a focused message.",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Path to the existing file to overwrite, relative to project root. The file must already exist."
                        },
                        "content": {
                            "type": "string",
                            "description": "The complete new content of the file. This replaces everything currently in the file — it is not a diff or a partial edit. Reproduce every line that should remain, including lines you are not changing, exactly as they should appear in the final file."
                        },
                        "version": {
                            "type": "integer",
                            "description": "The version returned by your most recent read(), create_file() or write() of this file."
                        }
                    },
                    "required": ["file_path", "content", "version"]
                }),
            ),
            definition(
                "create_file",
                concat!(
                    "Create a new file and register its purpose in the shared project manifest. ",
                    "The manifest is read by view_project_structure() to annotate every file for all agents — a precise file_purpose helps every downstream agent orient themselves. ",
                    "Fails if the file already exists — use write() to modify existing files. ",
                    "Parent directories are created automatically. ",
                    "Returns the new file's version (1), so you can write() to it without reading it first.",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Path for the new file, relative to the project root. Example: 'src/auth/token.py'. Must not already exist."
                        },
                        "file_purpose": {
                            "type": "string",
                            "description": "A precise description of this file's role. Stored in the project manifest and shown to every agent. 'JWT token creation, validation, and expiry logic for user authentication' is useful. 'auth stuff' is not. Use terms from terminology.md where applicable."
                        },
                        "initial_content": {
                            "type": "string",
                            "description": "Optional initial content. Leave empty to populate later with write().",
                            "default": ""
                        }
                    },
                    "required": ["file_path", "file_purpose"]
                }),
            ),
            definition(
                "view_project_structure",
                concat!(
                    "Return the project's file tree, read from disk, with each file annotated by its registered purpose where one exists. ",
                    "Use this to orient yourself at the start of a task, to find where a piece of logic lives, or to check whether a suitable file already exists before creating one. ",
                    "Hidden folders (.git, .echelon) and common dependency or build folders (node_modules, target, and similar) are skipped, symlinks are listed but not followed, and a very large tree is cut off with a notice.",
                ),
                json!({ "type": "object", "properties": {}, "required": [] }),
            ),
            definition(
                "update_terminology",
                concat!(
                    "Add or update a single term in terminology.md — the shared project glossary injected into every agent's context on every turn. ",
                    "Use this when introducing a new domain concept, acronym, or any term with scope for ambiguous interpretation. ",
                    "Existing terms are updated in place; new terms are appended. ",
                    "Definitions must be tight, project-specific, and written in plain language.",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "term": {
                            "type": "string",
                            "description": "The term to define or update. Use the exact canonical spelling — this becomes the authoritative name for this concept across the entire project."
                        },
                        "definition": {
                            "type": "string",
                            "description": "A precise, project-specific definition. Do not copy dictionary definitions. Example: 'Session Token — a signed string issued at login, encoding user_id and expiry timestamp, validated server-side on every authenticated request.'"
                        }
                    },
                    "required": ["term", "definition"]
                }),
            ),
        ]
    }

    async fn call(&self, name: &str, args: &Value, _ctx: &ToolCallContext) -> Result<String, String> {
        match name {
            "read" => self.read(args).await,
            "write" => self.write(args).await,
            "create_file" => self.create_file(args).await,
            "view_project_structure" => self.view_project_structure().await,
            "update_terminology" => self.update_terminology(args).await,
            other => Err(format!("ERROR: unknown tool '{other}'")),
        }
    }
}
