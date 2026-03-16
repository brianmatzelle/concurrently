use reqwest::Client;

use crate::api::{self, Message};

/// Decompose a high-level task into parallel subtasks using Claude.
/// Returns a list of (agent_name, task_description) pairs.
/// The kernel provides full conversation history for context.
pub async fn decompose_task(
    client: &Client,
    api_key: &str,
    task: &str,
    kernel: &[Message],
) -> Result<Vec<(String, String)>, String> {
    let system = r#"You are a task decomposition engine. You receive the full conversation history (the "kernel") so you understand the context of what has been discussed.

Given the latest task, break it into 2-5 independent subtasks that can be executed IN PARALLEL by separate agents. Each agent will also receive the full conversation kernel.

Reply ONLY with a JSON array. Each element: {"name": "short-agent-name", "task": "detailed task description"}

Rules:
- Each subtask must be independent (no dependencies between them)
- Agent names should be short, lowercase, descriptive (e.g. "researcher", "code-reviewer", "test-writer")
- Task descriptions should be self-contained with all necessary context
- Reference prior conversation context when relevant to the subtask
- Aim for 3-4 subtasks for most queries
- If the task is simple and cannot be parallelized, return a single subtask

Example output:
[{"name": "analyzer", "task": "Analyze the error logs..."}, {"name": "reviewer", "task": "Review the code for..."}]"#;

    // Build messages: full kernel + the new task
    let mut messages = kernel.to_vec();
    messages.push(Message {
        role: "user".to_string(),
        content: format!("[DECOMPOSE THIS TASK]: {task}"),
    });

    let response = api::send_message(client, api_key, system, &messages).await?;

    // Parse the JSON response - find the array in the response
    let trimmed = response.trim();
    let json_str = if trimmed.starts_with('[') {
        trimmed.to_string()
    } else if let Some(start) = trimmed.find('[') {
        if let Some(end) = trimmed.rfind(']') {
            trimmed[start..=end].to_string()
        } else {
            return Err("Could not find JSON array in response".to_string());
        }
    } else {
        return Err(format!("No JSON array in response: {trimmed}"));
    };

    #[derive(serde::Deserialize)]
    struct SubTask {
        name: String,
        task: String,
    }

    let subtasks: Vec<SubTask> =
        serde_json::from_str(&json_str).map_err(|e| format!("JSON parse error: {e}"))?;

    Ok(subtasks.into_iter().map(|s| (s.name, s.task)).collect())
}
