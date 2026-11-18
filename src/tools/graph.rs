use std::fs::File;
use std::sync::Arc;
use std::collections::HashMap;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{definition, parse_args, require_non_empty, ToolGroup};
use crate::agent_runtime::{ToolCallContext, ToolDefinition};
use crate::prompts::Task;

const EXCHANGE_NODE_TEMPLATE: &str = include_str!("../../prompts/prompt_templates/exchange_node.md");

#[derive(PartialEq, Clone)]
pub enum State {
    Waiting,
    Executing,
    Completed(String),
}

pub struct Graph {
    pub nodes: HashMap<String, Task>,
    pub ord_nodes: Vec<String>,
    pub node_state: HashMap<String, State>
}

#[derive(Deserialize)]
struct CompleteTaskArgs {
    summary: String,
}

#[derive(Deserialize)]
struct ExchangeArgs {
    target_agent: String,
    message: String,
    context: String,
}

#[derive(Deserialize)]
struct RequestSkillArgs {
    need: String,
}

// Every tool that changes the orchestrator's graph state.
pub struct GraphTools {
    graph: Arc<std::sync::Mutex<Graph>>,
}


impl Graph{

    pub fn topological_sort(&mut self) {
        let mut dependents: HashMap<&String, usize> = self.nodes.keys().map(|k| (k, 0)).collect();

        for task in self.nodes.values() {
        let parents = &task.depends_on;
            for parent in parents {
                *dependents.entry(parent).or_insert(0) += 1;
            
        }
        }


        let total_nodes = dependents.len();
        let mut result: Vec<String> = Vec::with_capacity(total_nodes);


        for (&node, &count) in &dependents {
            if count == 0 {
                result.push(node.clone());
            }
        }


        let mut cursor = 0;
        while cursor < result.len() {
            
            let parents = &self.nodes.get(&result[cursor]).unwrap().depends_on;
            cursor += 1;

            if parents.len() > 0 {
                for parent in parents {
                    if let Some(count) = dependents.get_mut(parent) {
                        *count -= 1;
                        if *count == 0 {
                            result.push(parent.clone());
                        }
                    }
                }
            }
        }

        if result.len() != total_nodes {
            eprintln!("Cycle detected in self.nodes");
        }

        //result.reverse();
        
        self.ord_nodes = result;
    }


    pub fn check_dependencies_state(&self, node: &String) -> State{
        let parents = &self.nodes.get(node).unwrap().depends_on;

        let total = parents.iter().map(|k| self.node_state.get(k).unwrap());
        let mut has_executing = false;

        for state in total {
            match *state {
                State::Waiting => return State::Waiting, // Short-circuit immediately
                State::Executing => has_executing = true,
                State::Completed(_) => {}
            }
        }

        if has_executing {
            State::Executing
        } else {
            State::Completed(String::from(""))
        }
    }

    fn complete_task(&mut self, node: &String, summary: String) {

        *self.node_state.get_mut(node).unwrap() = State::Completed(summary);
    }

    fn create_exchange_node(&mut self, ctx: &ToolCallContext, target_agent: String, message: String, context:String) {
        let caller_agent = ctx.agent_role.clone();
        let caller_id = ctx.node_id.clone();
        let node_id = format!("exchange_node_from-{caller_agent}({caller_id})-to-{target_agent}");
        let filename = format!("{caller_id}_{caller_agent}_{target_agent}");
        let file_path = ctx.project_root.join(&filename);
        let err_str: String = match File::create(file_path) {
            Ok(_) => String::from("The file has alreasy been created, no need to do it yourself"),
            Err(_) => format!("The file {filename} was not created, please create it yourself")
        };
        

        let mut old_task_prompt = String::from(self.nodes.get(&caller_id).unwrap().task_prompt.as_ref());
        old_task_prompt.push_str(&format!("You have already attempted this task. You found a discrepancy while attempting it. You communicated this discrepancy to the {target_agent} with the message - {message}. The agent reviewed your concern and replied to you in the file .echelon/conversations/{filename}. Attempt reading the file before starting the task."));
        {
        let new_task: &mut Task = self.nodes.get_mut(&caller_id).unwrap();
        new_task.task_prompt = Arc::from(old_task_prompt); 
        new_task.depends_on.push(node_id.clone());
        }
        *self.node_state.get_mut(&caller_id).unwrap() = State::Waiting; 
        self.ord_nodes.push(caller_id.clone());
        
        self.node_state.insert(node_id.clone(), State::Waiting);
        let task_prompt: Arc<str> = EXCHANGE_NODE_TEMPLATE
                                                .replace("{{caller_agent}}", &caller_agent)
                                                .replace("{{message}}", &message)
                                                .replace("{{context}}", &context)
                                                .replace("{{creation_status}}", &err_str).into();
        
        let task: Task = Task{agent:target_agent.clone(), story_points: 1, depends_on: vec!(), task_prompt: task_prompt, temperature:0.05};
        self.nodes.insert(node_id.clone(), task);
        self.ord_nodes.push(node_id);


        

    }

    fn request_skill(&mut self, agent: String, caller_id: String, need:String) {
        let mut node_id = String::from(format!("skill_request-{agent}-x"));
        while self.nodes.contains_key(&node_id) {
            node_id.push('x');
        }
        self.node_state.insert(node_id.clone(), State::Waiting);

        let task_prompt: Arc<str> = format!("This skill is being called by the {agent} agent. Create a skill for the following requirements - {need}").into();
        let task: Task = Task {agent:String::from("skill_creator"), story_points: 1, depends_on: vec![caller_id.clone()], 
        task_prompt: task_prompt, temperature: 0.1};
        self.nodes.insert(node_id.clone(), task);
        self.ord_nodes.push(node_id);
    }

}


impl GraphTools {
    pub fn new(graph: Arc<std::sync::Mutex<Graph>>) -> Self {
        Self { graph }
    }

    async fn complete_task(&self, args: &Value, ctx: &ToolCallContext) -> Result<String, String> {
        let args: CompleteTaskArgs = parse_args("complete_task", args)?;
        require_non_empty("summary", &args.summary)?;

        self.graph.lock().unwrap().complete_task(&ctx.node_id, args.summary);
        Ok("TASK COMPLETE".to_string())
    }

    async fn create_exchange_node(&self, args: &Value, ctx: &ToolCallContext) -> Result<String, String> {
        let args: ExchangeArgs = parse_args("create_exchange_node", args)?;
        require_non_empty("target_agent", &args.target_agent)?;
        require_non_empty("message", &args.message)?;
        require_non_empty("context", &args.context)?;

        // Opening an exchange also completes the current task. --> Cancelling this because calling complete_task overwrites state -> Wcich should be Waiting.
    //   /  let summary = format!("Opened an exchange with {}: {}", args.target_agent, args.message);
        self.graph.lock().unwrap().create_exchange_node(ctx, args.target_agent, args.message, args.context);
        //self.graph.lock().unwrap().complete_task(&ctx.node_id, summary);

        Ok("EXCHANGE OPENED".to_string())
    }

    async fn request_skill(&self, args: &Value, ctx: &ToolCallContext) -> Result<String, String> {
        let args: RequestSkillArgs = parse_args("request_skill", args)?;
        require_non_empty("need", &args.need)?;

        self.graph.lock().unwrap().request_skill(ctx.agent_role.clone(), ctx.node_id.clone(), args.need);
        Ok("SKILL REQUESTED".to_string())
    }
}

#[async_trait::async_trait]
impl ToolGroup for GraphTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![
            definition(
                "complete_task",
                concat!(
                    "Signal to the orchestrator that your current task is fully complete. ",
                    "Call this as the final action of every task — after all writes are done and committed. ",
                    "Do not make further changes to project files after calling complete_task(). ",
                    "The summary is injected as carryover context for downstream agents — write it as if briefing the next agent on what you did, what decisions you made, and anything they should know before starting their work.",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "summary": {
                            "type": "string",
                            "description": "2-3 sentences describing: what was produced, any significant decisions made and why, and anything the next agent should be aware of. Example: 'Created functional-requirements.md covering 12 feature areas and 4 explicit exclusions. Assumed multi-tenancy is in scope based on enterprise client mention — flagged in assumptions section for user review. Terminology.md seeded with 8 domain terms.'"
                        }
                    },
                    "required": ["summary"]
                }),
            ),
            definition(
                "create_exchange_node",
                concat!(
                    "Open a bounded request/response exchange with another agent. ",
                    "Use this when your task depends on information or a decision that belongs to another agent, when you detect a collision with another agent's work, or when you need a review or clarification. ",
                    "The orchestrator will trigger the target agent with your message on its next cycle. ",
                    "Your current task ends when the exchange is opened. ",
                    "Exchanges that exceed 10 messages are automatically escalated to the architect. ",
                    "Always include the specific context (document excerpt, code section, prior decision) that the target agent needs — they have no other way to know what you are referring to.",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "target_agent": {
                            "type": "string",
                            "description": "The agent to open an exchange with. Valid values: pm, ux_designer, ui_designer, interaction_designer, architect, mid_level_engineer, security_expert, backend_engineer, frontend_engineer, qa_engineer, automation_engineer, deployment_engineer, user"
                        },
                        "message": {
                            "type": "string",
                            "description": "Your question, request, or finding — specific and actionable. Do not ask vague questions."
                        },
                        "context": {
                            "type": "string",
                            "description": "The specific artifact section, code block, or prior decision this exchange is about. Example: paste the relevant lines from the architecture doc, the function in question, or the requirement that is unclear. Never leave this empty."
                        }
                    },
                    "required": ["target_agent", "message", "context"]
                }),
            ),
            definition(
                "request_skill",
                concat!(
                    "Request a skill to be created and made available for your tasks. ",
                    "Call this during your ingestion task when you identify a knowledge gap — a specific technique, pattern, or library your tasks require that your system prompt and LLD do not fully cover. ",
                    "The skill creator agent produces the skill and it is injected into all your subsequent tasks for this project. ",
                    "Multiple tasks can benefit from the same skill — request it once. ",
                    "Do not request skills for anything your system prompt, LLD, or general professional knowledge already covers.",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "need": {
                            "type": "string",
                            "description": "A precise description of what you need to know and why. Include: the specific technique or pattern required, which of your tasks need it, and what exactly is missing from your current context. Be specific — vague needs produce vague skills. Example: 'I need to verify Stripe webhook signatures in tasks payment_webhook_handler and payment_refund_webhook. I know to call stripe.webhooks.constructEvent() but need the correct header extraction, error handling pattern, and how to structure this in an Express middleware.'"
                        }
                    },
                    "required": ["need"]
                }),
            ),
        ]
    }

    async fn call(&self, name: &str, args: &Value, ctx: &ToolCallContext) -> Result<String, String> {
        match name {
            "complete_task" => self.complete_task(args, ctx).await,
            "create_exchange_node" => self.create_exchange_node(args, ctx).await,
            "request_skill" => self.request_skill(args, ctx).await,
            other => Err(format!("ERROR: unknown tool '{other}'")),
        }
    }
}
