use chrono::Utc;
use reqwest::Client;
use tokio::sync::mpsc;

use crate::agent::{Agent, AgentEvent, AgentStatus};
use crate::api::Message;
use crate::orchestrator;

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Input,
    Running,
    Done,
}

pub struct App {
    pub mode: AppMode,
    pub input: String,
    pub agents: Vec<Agent>,
    pub selected_agent: usize,
    pub scroll_offset: u16,
    pub status_message: String,
    pub event_tx: mpsc::UnboundedSender<AgentEvent>,
    pub event_rx: mpsc::UnboundedReceiver<AgentEvent>,
    pub client: Client,
    pub api_key: String,
    pub elapsed_ms: u64,
    /// Shared conversation kernel - every agent sees this full history
    pub kernel: Vec<Message>,
}

impl App {
    pub fn new(api_key: String) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            mode: AppMode::Input,
            input: String::new(),
            agents: Vec::new(),
            selected_agent: 0,
            scroll_offset: 0,
            status_message: "Enter a task to decompose into parallel agents".to_string(),
            event_tx,
            event_rx,
            client: Client::new(),
            api_key,
            elapsed_ms: 0,
            kernel: Vec::new(),
        }
    }

    pub fn submit_task(&mut self) {
        let task = self.input.clone();
        self.input.clear();
        self.mode = AppMode::Running;
        self.status_message = "Decomposing task into parallel subtasks...".to_string();

        // Append to the shared kernel
        self.kernel.push(Message {
            role: "user".to_string(),
            content: task.clone(),
        });

        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let event_tx = self.event_tx.clone();
        let kernel = self.kernel.clone();

        tokio::spawn(async move {
            match orchestrator::decompose_task(&client, &api_key, &task, &kernel).await {
                Ok(subtasks) => {
                    // Create and spawn all agents concurrently, each with full kernel context
                    for (name, task_desc) in &subtasks {
                        let agent = Agent::new(name, task_desc);
                        let _ = event_tx.send(AgentEvent::StatusChange {
                            agent_id: format!("NEW:{}:{}:{}", agent.id, agent.name, agent.task),
                            status: AgentStatus::Queued,
                        });

                        crate::agent::spawn_agent(
                            &agent,
                            event_tx.clone(),
                            &kernel,
                        );
                    }
                }
                Err(e) => {
                    let _ = event_tx.send(AgentEvent::StatusChange {
                        agent_id: "ORCHESTRATOR".to_string(),
                        status: AgentStatus::Error(format!("Decomposition failed: {e}")),
                    });
                }
            }
        });
    }

    /// Fold completed agent results back into the kernel
    pub fn fold_results_into_kernel(&mut self) {
        let mut summary = String::new();
        for agent in &self.agents {
            if agent.status == AgentStatus::Done {
                summary.push_str(&format!("[{}]: {}\n\n", agent.name, agent.output));
            }
        }
        if !summary.is_empty() {
            self.kernel.push(Message {
                role: "assistant".to_string(),
                content: summary,
            });
        }
    }

    /// Synthesize results from all completed agents
    pub fn synthesize_results(&mut self) {
        // Fold agent outputs into kernel before synthesizing
        self.fold_results_into_kernel();

        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let event_tx = self.event_tx.clone();
        let kernel = self.kernel.clone();

        let results: Vec<(String, String)> = self
            .agents
            .iter()
            .filter(|a| a.status == AgentStatus::Done)
            .map(|a| (a.name.clone(), a.output.clone()))
            .collect();

        // Create a synthesis agent
        let mut synth = Agent::new("synthesizer", "Combine all agent results");
        synth.name = "Synthesizer".to_string();
        let synth_id = synth.id.clone();

        let _ = event_tx.send(AgentEvent::StatusChange {
            agent_id: format!("NEW:{}:{}:{}", synth.id, synth.name, synth.task),
            status: AgentStatus::Queued,
        });

        tokio::spawn(async move {
            let _ = event_tx.send(AgentEvent::StatusChange {
                agent_id: synth_id.clone(),
                status: AgentStatus::Running,
            });

            let mut result_text = String::from("Here are the results from parallel agents:\n\n");
            for (name, output) in &results {
                result_text.push_str(&format!("=== {name} ===\n{output}\n\n"));
            }
            result_text.push_str("Synthesize these into a coherent, unified response.");

            // Build messages: full kernel history + synthesis request
            let mut messages = kernel.clone();
            messages.push(Message {
                role: "user".to_string(),
                content: result_text,
            });

            let (stream_tx, mut stream_rx) = mpsc::unbounded_channel();
            let c2 = client.clone();
            let k2 = api_key.clone();
            tokio::spawn(async move {
                crate::api::stream_message(
                    &c2,
                    &k2,
                    "You are a synthesis agent. You have the full conversation history. \
                     Combine the parallel agent results into one clear, concise answer \
                     that accounts for everything discussed so far.",
                    &messages,
                    stream_tx,
                )
                .await;
            });

            while let Some(chunk) = stream_rx.recv().await {
                match chunk {
                    crate::api::StreamChunk::Text(text) => {
                        let _ = event_tx.send(AgentEvent::TextDelta {
                            agent_id: synth_id.clone(),
                            text,
                        });
                    }
                    crate::api::StreamChunk::Done => {
                        let _ = event_tx.send(AgentEvent::StatusChange {
                            agent_id: synth_id.clone(),
                            status: AgentStatus::Done,
                        });
                        let _ = event_tx.send(AgentEvent::Finished {
                            agent_id: synth_id.clone(),
                        });
                        return;
                    }
                    crate::api::StreamChunk::Error(e) => {
                        let _ = event_tx.send(AgentEvent::StatusChange {
                            agent_id: synth_id.clone(),
                            status: AgentStatus::Error(e),
                        });
                        return;
                    }
                }
            }
        });
    }

    pub fn process_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AgentEvent::StatusChange { agent_id, status } => {
                    // Check if this is a "new agent" registration
                    if let Some(rest) = agent_id.strip_prefix("NEW:") {
                        let parts: Vec<&str> = rest.splitn(3, ':').collect();
                        if parts.len() == 3 {
                            let mut agent = Agent::new(parts[1], parts[2]);
                            agent.id = parts[0].to_string();
                            agent.name = parts[1].to_string();
                            agent.task = parts[2].to_string();
                            agent.status = status;
                            self.agents.push(agent);
                            self.status_message = format!(
                                "Running {} agents concurrently",
                                self.agents.iter().filter(|a| a.status == AgentStatus::Running || a.status == AgentStatus::Queued).count()
                            );
                        }
                    } else if agent_id == "ORCHESTRATOR" {
                        if let AgentStatus::Error(e) = &status {
                            self.status_message = e.clone();
                            self.mode = AppMode::Input;
                        }
                    } else if let Some(agent) = self.agents.iter_mut().find(|a| a.id == agent_id) {
                        if status == AgentStatus::Running && agent.started_at.is_none() {
                            agent.started_at = Some(Utc::now().format("%H:%M:%S").to_string());
                        }
                        if status == AgentStatus::Done {
                            agent.finished_at = Some(Utc::now().format("%H:%M:%S").to_string());
                        }
                        agent.status = status;
                    }

                    // Check if all agents are done
                    self.update_completion_status();
                }
                AgentEvent::TextDelta { agent_id, text } => {
                    if let Some(agent) = self.agents.iter_mut().find(|a| a.id == agent_id) {
                        agent.tokens_received += 1;
                        agent.current_tool = None;
                        agent.output.push_str(&text);
                    }
                }
                AgentEvent::ToolUse {
                    agent_id,
                    tool,
                    detail,
                } => {
                    if let Some(agent) = self.agents.iter_mut().find(|a| a.id == agent_id) {
                        let label = if detail.is_empty() {
                            tool.clone()
                        } else {
                            format!("{tool}: {detail}")
                        };
                        agent.current_tool = Some(label.clone());
                        agent.output.push_str(&format!("\n[{label}]\n"));
                    }
                }
                AgentEvent::CostUpdate {
                    agent_id,
                    cost_usd,
                } => {
                    if let Some(agent) = self.agents.iter_mut().find(|a| a.id == agent_id) {
                        agent.cost_usd = cost_usd;
                    }
                }
                AgentEvent::Finished { .. } => {
                    self.update_completion_status();
                }
            }
        }
    }

    fn update_completion_status(&mut self) {
        let total = self.agents.len();
        let done = self
            .agents
            .iter()
            .filter(|a| matches!(a.status, AgentStatus::Done | AgentStatus::Error(_)))
            .count();
        let running = self
            .agents
            .iter()
            .filter(|a| a.status == AgentStatus::Running)
            .count();

        if total > 0 && done == total {
            self.mode = AppMode::Done;
            self.status_message = format!(
                "All {total} agents completed | q to quit | s to synthesize | n for new task"
            );
        } else if total > 0 {
            self.status_message = format!("{running} running, {done}/{total} complete");
        }
    }

    pub fn select_next(&mut self) {
        if !self.agents.is_empty() {
            self.selected_agent = (self.selected_agent + 1) % self.agents.len();
            self.scroll_offset = 0;
        }
    }

    pub fn select_prev(&mut self) {
        if !self.agents.is_empty() {
            self.selected_agent = if self.selected_agent == 0 {
                self.agents.len() - 1
            } else {
                self.selected_agent - 1
            };
            self.scroll_offset = 0;
        }
    }

    pub fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(3);
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(3);
    }
}
