use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<Message>,
    stream: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum StreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: serde_json::Value },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: usize,
        content_block: serde_json::Value,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        index: usize,
        delta: Delta,
    },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },
    #[serde(rename = "message_delta")]
    MessageDelta { delta: serde_json::Value },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "error")]
    Error { error: serde_json::Value },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Delta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(other)]
    Other,
}

pub enum StreamChunk {
    Text(String),
    Done,
    Error(String),
}

pub async fn stream_message(
    client: &Client,
    api_key: &str,
    system: &str,
    messages: &[Message],
    tx: mpsc::UnboundedSender<StreamChunk>,
) {
    let request = ApiRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 4096,
        system: system.to_string(),
        messages: messages.to_vec(),
        stream: true,
    };

    let response = match client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(StreamChunk::Error(format!("Request failed: {e}")));
            return;
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let _ = tx.send(StreamChunk::Error(format!("API {status}: {body}")));
        return;
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => {
                let _ = tx.send(StreamChunk::Error(format!("Stream error: {e}")));
                return;
            }
        };

        buffer.push_str(&String::from_utf8_lossy(&bytes));

        // Parse SSE events from buffer
        while let Some(event_end) = buffer.find("\n\n") {
            let event_str = buffer[..event_end].to_string();
            buffer = buffer[event_end + 2..].to_string();

            for line in event_str.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        let _ = tx.send(StreamChunk::Done);
                        return;
                    }
                    if let Ok(event) = serde_json::from_str::<StreamEvent>(data) {
                        match event {
                            StreamEvent::ContentBlockDelta {
                                delta: Delta::TextDelta { text },
                                ..
                            } => {
                                let _ = tx.send(StreamChunk::Text(text));
                            }
                            StreamEvent::MessageStop => {
                                let _ = tx.send(StreamChunk::Done);
                                return;
                            }
                            StreamEvent::Error { error } => {
                                let _ = tx.send(StreamChunk::Error(format!("API error: {error}")));
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    let _ = tx.send(StreamChunk::Done);
}

/// Non-streaming message for orchestrator use
pub async fn send_message(
    client: &Client,
    api_key: &str,
    system: &str,
    messages: &[Message],
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Req {
        model: String,
        max_tokens: u32,
        system: String,
        messages: Vec<Message>,
    }

    let request = Req {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 4096,
        system: system.to_string(),
        messages: messages.to_vec(),
    };

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("API {status}: {body}"));
    }

    #[derive(Deserialize)]
    struct Resp {
        content: Vec<ContentBlock>,
    }
    #[derive(Deserialize)]
    struct ContentBlock {
        text: Option<String>,
    }

    let resp: Resp = response.json().await.map_err(|e| format!("Parse error: {e}"))?;
    Ok(resp
        .content
        .iter()
        .filter_map(|c| c.text.as_ref())
        .cloned()
        .collect::<Vec<_>>()
        .join(""))
}
