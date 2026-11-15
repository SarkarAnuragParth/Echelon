use echelon_runtime::scaffold;

fn usage() -> ! {
    eprintln!("usage: echelon <init|start> [project_dir]");
    eprintln!("  init   scaffold a project and exit");
    eprintln!("  start  scaffold if needed, then run the daemon");
    std::process::exit(1);
}

fn project_root(args: &[String]) -> std::path::PathBuf {
    args.get(2)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("could not read current directory"))
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(command) = args.get(1) else { usage() };
    let root = project_root(&args);

    match command.as_str() {
        "init" => match scaffold::ensure(&root).await {
            Ok(()) => println!("Project ready at {}", root.display()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        // start always ensures scaffolding itself — a first-time user who
        // goes straight to `echelon start` never needs to have run init.
        // Idempotent, so this is a no-op on an already-scaffolded project.
        "start" => {
            if let Err(e) = scaffold::ensure(&root).await {
                eprintln!("{e}");
                std::process::exit(1);
            }
            // Orchestrator, AgentRuntime wiring, and the daemon loop are not
            // yet built — see DECISION_REFERENCE for what's still open.
            println!("Project ready at {}. Daemon not yet implemented.", root.display());
        }
        _ => usage(),
    }
}
