use chrono::Utc;
use reqwest::Client;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::api::{self, Message, StreamChunk};

#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    Queued,
    Running,
    Done,
    Error(String),
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Queued => write!(f, "QUEUED"),
            AgentStatus::Running => write!(f, "RUNNING"),
            AgentStatus::Done => write!(f, "DONE"),
            AgentStatus::Error(e) => write!(f, "ERROR: {e}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub task: String,
    pub status: AgentStatus,
    pub output: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub tokens_received: usize,
}

impl Agent {
    pub fn new(name: &str, task: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string()[..8].to_string(),
            name: name.to_string(),
            task: task.to_string(),
            status: AgentStatus::Queued,
            output: String::new(),
            started_at: None,
            finished_at: None,
            tokens_received: 0,
        }
    }
}

/// Message sent from agent tasks back to the main event loop
pub enum AgentEvent {
    StatusChange {
        agent_id: String,
        status: AgentStatus,
    },
    TextDelta {
        agent_id: String,
        text: String,
    },
    Finished {
        agent_id: String,
    },
}

pub fn spawn_agent(
    agent: &Agent,
    client: Client,
    api_key: String,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
    kernel: &[Message],
) {
    let agent_id = agent.id.clone();
    let task = agent.task.clone();
    let name = agent.name.clone();
    let kernel = kernel.to_vec();

    tokio::spawn(async move {
        // Signal running
        let _ = event_tx.send(AgentEvent::StatusChange {
            agent_id: agent_id.clone(),
            status: AgentStatus::Running,
        });

        let system_prompt = format!(
            "You are '{}', a focused subagent in a concurrent workflow. \
             You share context with other agents via the conversation history below. \
             Complete your specific task concisely and directly. \
             Do not use markdown headers. Be brief but thorough. \
             Current time: {}",
            name,
            Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        );

        // Build messages: full kernel history + this agent's specific subtask
        let mut messages = kernel;
        messages.push(Message {
            role: "user".to_string(),
            content: format!(
                "[SUBTASK for agent '{}']: {}",
                name, task
            ),
        });

        let (stream_tx, mut stream_rx) = mpsc::unbounded_channel();

        let api_client = client.clone();
        let api_key_clone = api_key.clone();
        let sys = system_prompt.clone();
        let msgs = messages.clone();
        tokio::spawn(async move {
            api::stream_message(&api_client, &api_key_clone, &sys, &msgs, stream_tx).await;
        });

        // Forward stream chunks as agent events
        while let Some(chunk) = stream_rx.recv().await {
            match chunk {
                StreamChunk::Text(text) => {
                    let _ = event_tx.send(AgentEvent::TextDelta {
                        agent_id: agent_id.clone(),
                        text,
                    });
                }
                StreamChunk::Done => {
                    let _ = event_tx.send(AgentEvent::StatusChange {
                        agent_id: agent_id.clone(),
                        status: AgentStatus::Done,
                    });
                    let _ = event_tx.send(AgentEvent::Finished {
                        agent_id: agent_id.clone(),
                    });
                    return;
                }
                StreamChunk::Error(e) => {
                    let _ = event_tx.send(AgentEvent::StatusChange {
                        agent_id: agent_id.clone(),
                        status: AgentStatus::Error(e),
                    });
                    let _ = event_tx.send(AgentEvent::Finished {
                        agent_id: agent_id.clone(),
                    });
                    return;
                }
            }
        }
    });
}
