use chrono::Utc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::api::Message;

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
    /// Currently active tool (if any)
    pub current_tool: Option<String>,
    /// Cost in USD
    pub cost_usd: f64,
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
            current_tool: None,
            cost_usd: 0.0,
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
    ToolUse {
        agent_id: String,
        tool: String,
        detail: String,
    },
    CostUpdate {
        agent_id: String,
        cost_usd: f64,
    },
    Finished {
        agent_id: String,
    },
}

/// State tracked while parsing a single agent's stream-json output
struct ParseState {
    agent_id: String,
    last_text_len: usize,
    last_tool_count: usize,
}

/// Spawn an agent as a real Claude Code process
pub fn spawn_agent(
    agent: &Agent,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
    kernel: &[Message],
) {
    let agent_id = agent.id.clone();
    let task = agent.task.clone();
    let name = agent.name.clone();
    let kernel = kernel.to_vec();

    tokio::spawn(async move {
        let _ = event_tx.send(AgentEvent::StatusChange {
            agent_id: agent_id.clone(),
            status: AgentStatus::Running,
        });

        // Build kernel context for the system prompt
        let mut system_ctx = format!(
            "You are '{}', a focused subagent in a concurrent workflow. \
             Complete your specific task concisely and directly. \
             Current time: {}",
            name,
            Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        );

        if !kernel.is_empty() {
            system_ctx.push_str("\n\nConversation history (shared kernel):\n");
            for msg in &kernel {
                system_ctx.push_str(&format!("<{}>\n{}\n</{}>\n", msg.role, msg.content, msg.role));
            }
        }

        let child = Command::new("claude")
            .args([
                "-p", &task,
                "--append-system-prompt", &system_ctx,
                "--output-format", "stream-json",
                "--include-partial-messages",
                "--dangerously-skip-permissions",
                "--no-session-persistence",
                "--model", "sonnet",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                let _ = event_tx.send(AgentEvent::StatusChange {
                    agent_id: agent_id.clone(),
                    status: AgentStatus::Error(format!("Failed to spawn claude: {e}")),
                });
                let _ = event_tx.send(AgentEvent::Finished { agent_id });
                return;
            }
        };

        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout).lines();
        let mut state = ParseState {
            agent_id: agent_id.clone(),
            last_text_len: 0,
            last_tool_count: 0,
        };

        while let Ok(Some(line)) = reader.next_line().await {
            let events = parse_stream_line(&line, &mut state);
            for event in events {
                let _ = event_tx.send(event);
            }
        }

        // If the process exits without a result event, check exit status
        let status = child.wait().await;
        match status {
            Ok(s) if s.success() => {
                // Send done if we haven't already
                let _ = event_tx.send(AgentEvent::StatusChange {
                    agent_id: agent_id.clone(),
                    status: AgentStatus::Done,
                });
                let _ = event_tx.send(AgentEvent::Finished { agent_id });
            }
            Ok(s) => {
                let _ = event_tx.send(AgentEvent::StatusChange {
                    agent_id: agent_id.clone(),
                    status: AgentStatus::Error(format!("claude exited with {s}")),
                });
                let _ = event_tx.send(AgentEvent::Finished { agent_id });
            }
            Err(e) => {
                let _ = event_tx.send(AgentEvent::StatusChange {
                    agent_id: agent_id.clone(),
                    status: AgentStatus::Error(format!("wait error: {e}")),
                });
                let _ = event_tx.send(AgentEvent::Finished { agent_id });
            }
        }
    });
}

fn parse_stream_line(line: &str, state: &mut ParseState) -> Vec<AgentEvent> {
    let v: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match msg_type {
        "assistant" => {
            let mut events = vec![];
            if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                let mut full_text = String::new();
                let mut tool_count = 0;

                for block in content {
                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match block_type {
                        "text" => {
                            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                full_text.push_str(text);
                            }
                        }
                        "tool_use" => {
                            tool_count += 1;
                            // Only emit event for new tool uses
                            if tool_count > state.last_tool_count {
                                let name = block
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("unknown");
                                let input = block.get("input");
                                let detail = summarize_tool_input(name, input);
                                events.push(AgentEvent::ToolUse {
                                    agent_id: state.agent_id.clone(),
                                    tool: name.to_string(),
                                    detail,
                                });
                            }
                        }
                        _ => {}
                    }
                }

                state.last_tool_count = tool_count;

                // Send only new text (partial messages contain cumulative text)
                if full_text.len() > state.last_text_len {
                    let new_text = full_text[state.last_text_len..].to_string();
                    state.last_text_len = full_text.len();
                    events.push(AgentEvent::TextDelta {
                        agent_id: state.agent_id.clone(),
                        text: new_text,
                    });
                }
            }
            events
        }

        "result" => {
            let subtype = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
            let cost = v.get("cost_usd").and_then(|c| c.as_f64());

            let mut events = vec![];

            // Capture final result text if we missed any
            if let Some(result_text) = v.get("result").and_then(|r| r.as_str()) {
                if result_text.len() > state.last_text_len {
                    let new_text = result_text[state.last_text_len..].to_string();
                    if !new_text.is_empty() {
                        events.push(AgentEvent::TextDelta {
                            agent_id: state.agent_id.clone(),
                            text: new_text,
                        });
                    }
                }
                // Reset for next turn if needed
                state.last_text_len = 0;
                state.last_tool_count = 0;
            }

            if let Some(cost) = cost {
                events.push(AgentEvent::CostUpdate {
                    agent_id: state.agent_id.clone(),
                    cost_usd: cost,
                });
            }

            if subtype == "error" {
                let error_msg = v
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("Unknown error");
                events.push(AgentEvent::StatusChange {
                    agent_id: state.agent_id.clone(),
                    status: AgentStatus::Error(error_msg.to_string()),
                });
            } else {
                events.push(AgentEvent::StatusChange {
                    agent_id: state.agent_id.clone(),
                    status: AgentStatus::Done,
                });
            }
            events.push(AgentEvent::Finished {
                agent_id: state.agent_id.clone(),
            });
            events
        }

        _ => vec![],
    }
}

fn summarize_tool_input(tool_name: &str, input: Option<&serde_json::Value>) -> String {
    let input = match input {
        Some(v) => v,
        None => return String::new(),
    };

    match tool_name {
        "Read" => input
            .get("file_path")
            .and_then(|p| p.as_str())
            .map(|p| shorten_path(p))
            .unwrap_or_default(),
        "Edit" => input
            .get("file_path")
            .and_then(|p| p.as_str())
            .map(|p| shorten_path(p))
            .unwrap_or_default(),
        "Write" => input
            .get("file_path")
            .and_then(|p| p.as_str())
            .map(|p| shorten_path(p))
            .unwrap_or_default(),
        "Bash" => input
            .get("command")
            .and_then(|c| c.as_str())
            .map(|c| c.chars().take(50).collect::<String>())
            .unwrap_or_default(),
        "Glob" => input
            .get("pattern")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string(),
        "Grep" => input
            .get("pattern")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string(),
        _ => format!("{}", tool_name),
    }
}

fn shorten_path(path: &str) -> String {
    // Show just the last 2 components
    let parts: Vec<&str> = path.rsplit('/').take(2).collect();
    parts.into_iter().rev().collect::<Vec<_>>().join("/")
}
