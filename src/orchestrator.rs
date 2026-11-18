use std::path::{Path,PathBuf};
use std::time::Duration;
use std::sync::LazyLock;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use tokio::task::JoinSet;
use regex::Regex;

use crate::agent_runtime::{AgentRuntime};
use crate::model_client::{ModelConfig, ModelClient, SharedModelClient};
use crate::providers::MockModelClient;
use crate::tools::graph::{Graph, State};
use crate::prompts::{Module, load_module, AgentPrompt, agent_system_prompt};
use crate::tool_execution_handler::{ToolExecutionHandler};


static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{(\{.*?\})\}").unwrap()
});

const CONCURRENY_LIMIT: usize = 5; //Wil be fetched using env variable later


pub async fn start(provider: Option<&str>, project_path: PathBuf){

    let model_client = get_model_client(provider);
    execute_module(".echelon/tasks/bootstrap-module.yaml", &project_path, model_client).await;
}


async fn execute_module(module_path: &str, project_path: &Path, model_client: Arc<dyn ModelClient>) {
    let module = load_module(project_path.join(module_path)).unwrap();
    let graph = Arc::new(std::sync::Mutex::new(create_graph_from_module(module)));
    //let top_ord_nodes = graph.lock().await.ord_nodes.clone();
    let mut set =JoinSet::new();
    let graph_clone = Arc::clone(&graph);
    let tool_execution_handler = Arc::new(ToolExecutionHandler::new(project_path, graph_clone));
    loop {
        while set.len() < CONCURRENY_LIMIT {
            
            let curr_node = graph.lock().unwrap().ord_nodes.pop();
            if let Some(node) = curr_node{
                if matches!(graph.lock().unwrap().node_state.get(&node), Some(State::Completed(_))) {
                    continue
                }
                
                let mut parents_state = graph.lock().unwrap().check_dependencies_state(&node);
                if parents_state == State::Waiting {
                    panic!("Somehow we picked up a child node - check the topological sort and actual expected value");
                }
                
                

                while parents_state == State::Executing {
                    println!("{node} is waiting");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    parents_state = graph.lock().unwrap().check_dependencies_state(&node);
                }

                *graph.lock().unwrap().node_state.get_mut(&node).unwrap() = State::Executing;                
                let graph_clone = Arc::clone(&graph);
                
                let handler = Arc::clone(&tool_execution_handler);
                let model_client_clone = Arc::clone(&model_client);
                set.spawn(execute_node_or_wait(node.clone(), handler, graph_clone,model_client_clone, project_path.to_path_buf()));
                println!("{node} has been spawned");
            }
            else {
                println!("Empty queue!");
                break;
            }
        }
        match set.join_next().await {
            Some(Ok(())) => continue,
            Some(Err(e)) => eprintln!("Task failed: {e}"),
            None => break,
        }
    }

}

async fn execute_node_or_wait(node: String, handler: Arc<ToolExecutionHandler>, graph: Arc<Mutex<Graph>>,  model_client: Arc<dyn ModelClient>, project_path: PathBuf){

    let (agent_name, task_prompt, temperature)= {
    let lock = graph.lock().unwrap(); 
    let task = lock.nodes.get(&node).unwrap();
    (task.agent.clone(), Arc::clone(&task.task_prompt), task.temperature)
    };
    let content = resolve_task_prompt(task_prompt, &project_path);
    let agent: &AgentPrompt = agent_system_prompt(&agent_name).unwrap();
    let schema = handler.definitions();
    let system_prompt = agent.system_prompt;
    let model_config = ModelConfig { provider: String::from("local"), model: String::from("None"), temperature: Some(temperature), enable_thinking: Some(true) } ;

    AgentRuntime::run(system_prompt, schema, content, agent_name, model_config, model_client, handler, node, 20, project_path).await;

}

fn resolve_task_prompt(task_prompt:Arc<str>, project_path: &PathBuf) -> Arc<str>{
    let resolved: Cow<str> = PATTERN.replace_all(&task_prompt, |cap: &regex::Captures| {
    let path = project_path.join(&cap[1]);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|_| format!("Unable to fetch {}", &cap[1]))
        });
    match resolved {
    Cow::Borrowed(_) => task_prompt,      // reuse the Arc, zero copy
    Cow::Owned(s) => Arc::from(s),   // one copy, only when placeholders matched
    }
}

fn get_model_client(provider: Option<&str>) -> SharedModelClient {
    match provider {
        Some(_) => MockModelClient::all_tools_script(),
        None => MockModelClient::all_tools_script()
    }
}


fn create_graph_from_module(module: Module) -> Graph{
    // ord_nodes = topologically_sort_nodes(&module.tasks);
    let states = module.tasks.keys().map(|k| (k.clone(), State::Waiting)).collect();
    let mut g = Graph{
        nodes: module.tasks,
        ord_nodes: vec!(),
        node_state: states
    };
    g.topological_sort();
    g
}