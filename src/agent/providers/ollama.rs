//! Ollama agent provider — /api/chat with OpenAI-compatible tool format, NDJSON streaming.
//!
//! Graceful degradation: if the model doesn't support tools (e.g. `mistral:latest`),
//! treat a text-only response as a final answer.

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

use super::{AgentProvider, AgentTurnResult, ParsedBlock};
use crate::agent::loop_runner::AgentEvent;
use crate::enrichment::ProviderError;

pub struct OllamaAgentProvider {
    client: Client,
    endpoint: String,
    model: String,
}

impl OllamaAgentProvider {
    pub fn new(endpoint: String, model: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            client,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            model,
        }
    }
}

#[async_trait]
impl AgentProvider for OllamaAgentProvider {
    async fn send_turn(
        &self,
        system_prompt: &str,
        messages: &[Value],
        tool_defs: &[Value],
        tx: &mpsc::Sender<AgentEvent>,
    ) -> Result<AgentTurnResult, ProviderError> {
        let mut full_messages = vec![serde_json::json!({
            "role": "system",
            "content": system_prompt,
        })];
        full_messages.extend_from_slice(messages);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": full_messages,
            "stream": true,
        });

        if !tool_defs.is_empty() {
            body["tools"] = Value::Array(tool_defs.to_vec());
        }

        let url = format!("{}/api/chat", self.endpoint);
        let response = self.client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ProviderError::Timeout { seconds: 300 }
                } else if e.is_connect() {
                    ProviderError::Network {
                        message: format!("Ollama not running at {}", self.endpoint),
                    }
                } else {
                    ProviderError::Network { message: e.to_string() }
                }
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Other {
                message: format!("Ollama API error ({}): {}", status, body),
            });
        }

        parse_ollama_ndjson(response, tx).await
    }

    fn translate_tool_definitions(&self, canonical_tools: &[Value]) -> Vec<Value> {
        // Ollama uses the same format as OpenAI for tools
        canonical_tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        "description": tool.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                        "parameters": tool.get("input_schema").cloned().unwrap_or(serde_json::json!({"type": "object", "properties": {}})),
                    }
                })
            })
            .collect()
    }

    fn build_messages_from_history(
        &self,
        history: &[(String, String)],
        user_message: &str,
    ) -> Vec<Value> {
        let mut messages: Vec<Value> = Vec::new();
        for (role, content) in history {
            let api_role = match role.as_str() {
                "User" => "user",
                "Assistant" => "assistant",
                _ => continue,
            };
            messages.push(serde_json::json!({
                "role": api_role,
                "content": content,
            }));
        }
        messages.push(serde_json::json!({
            "role": "user",
            "content": user_message,
        }));
        messages
    }

    fn append_assistant_message(&self, messages: &mut Vec<Value>, blocks: &[ParsedBlock]) {
        let mut content_text = String::new();
        let mut tool_calls: Vec<Value> = Vec::new();

        for block in blocks {
            match block {
                ParsedBlock::Text(text) => content_text.push_str(text),
                ParsedBlock::ToolUse { name, input, .. } => {
                    tool_calls.push(serde_json::json!({
                        "function": {
                            "name": name,
                            "arguments": input,
                        }
                    }));
                }
            }
        }

        let mut msg = serde_json::json!({
            "role": "assistant",
            "content": content_text,
        });
        if !tool_calls.is_empty() {
            msg["tool_calls"] = Value::Array(tool_calls);
        }
        messages.push(msg);
    }

    fn append_tool_results(
        &self,
        messages: &mut Vec<Value>,
        results: &[(String, String, String)],
    ) {
        for (_tool_call_id, _name, content) in results {
            messages.push(serde_json::json!({
                "role": "tool",
                "content": content,
            }));
        }
    }

    async fn compact_history(&self, prompt: &str) -> Result<String, ProviderError> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": "You are a helpful assistant that summarizes conversations." },
                { "role": "user", "content": prompt },
            ],
            "stream": false,
        });

        let url = format!("{}/api/chat", self.endpoint);
        let response = self.client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network { message: e.to_string() })?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return Err(ProviderError::Other {
                message: format!("API error ({}): {}", status, body_text),
            });
        }

        let json: Value = response.json().await.map_err(|e| ProviderError::ParseError {
            message: e.to_string(),
        })?;

        json.get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
            .ok_or(ProviderError::ParseError {
                message: "Empty compaction response".to_string(),
            })
    }

    fn display_name(&self) -> &str {
        "Ollama"
    }
}

/// Parse Ollama NDJSON streaming response with tool_calls support.
async fn parse_ollama_ndjson(
    response: reqwest::Response,
    tx: &mpsc::Sender<AgentEvent>,
) -> Result<AgentTurnResult, ProviderError> {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut blocks: Vec<ParsedBlock> = Vec::new();
    let mut current_text = String::new();
    let mut tool_call_counter: u32 = 0;
    // Ollama doesn't report token usage in streaming — we report 0/0
    let input_tokens: u32 = 0;
    let output_tokens: u32 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ProviderError::Network {
            message: format!("Stream error: {}", e),
        })?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            let event: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Some(error) = event.get("error").and_then(|e| e.as_str()) {
                return Err(ProviderError::Other { message: error.to_string() });
            }

            // Stream text content
            if let Some(content) = event
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
            {
                if !content.is_empty() {
                    current_text.push_str(content);
                    let _ = tx.send(AgentEvent::Chunk(content.to_string())).await;
                }
            }

            // Check for tool_calls in the message
            if let Some(tool_calls) = event
                .get("message")
                .and_then(|m| m.get("tool_calls"))
                .and_then(|tc| tc.as_array())
            {
                if !current_text.is_empty() {
                    blocks.push(ParsedBlock::Text(current_text.clone()));
                    current_text.clear();
                }
                for tc in tool_calls {
                    let func = tc.get("function").unwrap_or(&Value::Null);
                    let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                    let args = func.get("arguments").cloned().unwrap_or(Value::Object(serde_json::Map::new()));
                    tool_call_counter += 1;
                    let id = format!("ollama_tc_{}", tool_call_counter);
                    blocks.push(ParsedBlock::ToolUse { id, name, input: args });
                }
            }

            // Check if done
            if event.get("done").and_then(|d| d.as_bool()).unwrap_or(false) {
                if !current_text.is_empty() {
                    blocks.push(ParsedBlock::Text(std::mem::take(&mut current_text)));
                }

                let has_tool_uses = blocks.iter().any(|b| matches!(b, ParsedBlock::ToolUse { .. }));
                let should_stop = !has_tool_uses;
                return Ok(AgentTurnResult { blocks, should_stop, input_tokens, output_tokens });
            }
        }
    }

    if !current_text.is_empty() {
        blocks.push(ParsedBlock::Text(current_text));
    }

    Ok(AgentTurnResult { blocks, should_stop: true, input_tokens, output_tokens })
}
