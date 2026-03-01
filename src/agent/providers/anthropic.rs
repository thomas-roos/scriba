//! Anthropic agent provider — Claude Messages API with streaming tool use.

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

use super::{AgentProvider, AgentTurnResult, ParsedBlock};
use crate::agent::loop_runner::AgentEvent;
use crate::enrichment::ProviderError;

pub struct AnthropicAgentProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl AnthropicAgentProvider {
    pub fn new(api_key: String, model: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to create HTTP client");
        Self { client, api_key, model }
    }
}

#[async_trait]
impl AgentProvider for AnthropicAgentProvider {
    async fn send_turn(
        &self,
        system_prompt: &str,
        messages: &[Value],
        tool_defs: &[Value],
        tx: &mpsc::Sender<AgentEvent>,
    ) -> Result<AgentTurnResult, ProviderError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system_prompt,
            "tools": tool_defs,
            "messages": messages,
            "stream": true,
        });

        let response = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ProviderError::Timeout { seconds: 300 }
                } else {
                    ProviderError::Network { message: e.to_string() }
                }
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Other {
                message: format!("API error ({}): {}", status, body),
            });
        }

        parse_anthropic_sse(response, tx).await
    }

    fn translate_tool_definitions(&self, canonical_tools: &[Value]) -> Vec<Value> {
        // Anthropic format IS the canonical format — pass through
        canonical_tools.to_vec()
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
        let mut content: Vec<Value> = Vec::new();
        for block in blocks {
            match block {
                ParsedBlock::Text(text) => {
                    content.push(serde_json::json!({
                        "type": "text",
                        "text": text,
                    }));
                }
                ParsedBlock::ToolUse { id, name, input } => {
                    content.push(serde_json::json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input,
                    }));
                }
            }
        }
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": content,
        }));
    }

    fn append_tool_results(
        &self,
        messages: &mut Vec<Value>,
        results: &[(String, String, String)],
    ) {
        let tool_results: Vec<Value> = results
            .iter()
            .map(|(tool_id, _name, content)| {
                serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": tool_id,
                    "content": content,
                })
            })
            .collect();
        messages.push(serde_json::json!({
            "role": "user",
            "content": tool_results,
        }));
    }

    async fn compact_history(&self, prompt: &str) -> Result<String, ProviderError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": prompt }],
        });

        let response = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
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

        let text = json.get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|block| block.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("(summary unavailable)")
            .to_string();

        Ok(text)
    }

    fn display_name(&self) -> &str {
        "Anthropic"
    }
}

/// Parse a streaming SSE response from the Anthropic Messages API.
async fn parse_anthropic_sse(
    response: reqwest::Response,
    tx: &mpsc::Sender<AgentEvent>,
) -> Result<AgentTurnResult, ProviderError> {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut blocks: Vec<ParsedBlock> = Vec::new();
    let mut stop_reason = String::from("end_turn");
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;

    let mut current_text = String::new();
    let mut current_tool: Option<(String, String)> = None; // (id, name)
    let mut current_tool_input_json = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ProviderError::Network {
            message: format!("Stream error: {}", e),
        })?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if !line.starts_with("data: ") {
                continue;
            }
            let json_str = &line[6..];
            if json_str == "[DONE]" {
                continue;
            }

            let event: Value = match serde_json::from_str(json_str) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match event_type {
                "message_start" => {
                    if let Some(tokens) = event.get("message")
                        .and_then(|m| m.get("usage"))
                        .and_then(|u| u.get("input_tokens"))
                        .and_then(|t| t.as_u64())
                    {
                        input_tokens = tokens as u32;
                    }
                }
                "content_block_start" => {
                    let content_block = event.get("content_block").unwrap_or(&Value::Null);
                    let block_type =
                        content_block.get("type").and_then(|t| t.as_str()).unwrap_or("");

                    if block_type == "tool_use" {
                        if !current_text.is_empty() {
                            blocks.push(ParsedBlock::Text(current_text.clone()));
                            current_text.clear();
                        }
                        let id = content_block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = content_block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        current_tool = Some((id, name));
                        current_tool_input_json.clear();
                    }
                }
                "content_block_delta" => {
                    let delta = event.get("delta").unwrap_or(&Value::Null);
                    let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");

                    if delta_type == "text_delta" {
                        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                            current_text.push_str(text);
                            let _ = tx.send(AgentEvent::Chunk(text.to_string())).await;
                        }
                    } else if delta_type == "input_json_delta" {
                        if let Some(json_part) =
                            delta.get("partial_json").and_then(|t| t.as_str())
                        {
                            current_tool_input_json.push_str(json_part);
                        }
                    }
                }
                "content_block_stop" => {
                    if let Some((id, name)) = current_tool.take() {
                        let input: Value = serde_json::from_str(&current_tool_input_json)
                            .unwrap_or(Value::Object(serde_json::Map::new()));
                        blocks.push(ParsedBlock::ToolUse { id, name, input });
                        current_tool_input_json.clear();
                    }
                }
                "message_delta" => {
                    if let Some(reason) = event
                        .get("delta")
                        .and_then(|d| d.get("stop_reason"))
                        .and_then(|r| r.as_str())
                    {
                        stop_reason = reason.to_string();
                    }
                    if let Some(tokens) = event
                        .get("usage")
                        .and_then(|u| u.get("output_tokens"))
                        .and_then(|t| t.as_u64())
                    {
                        output_tokens = tokens as u32;
                    }
                }
                "error" => {
                    let msg = event
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("Unknown streaming error");
                    return Err(ProviderError::Other {
                        message: msg.to_string(),
                    });
                }
                _ => {}
            }
        }
    }

    // Flush remaining text
    if !current_text.is_empty() {
        blocks.push(ParsedBlock::Text(current_text));
    }

    let has_tool_uses = blocks.iter().any(|b| matches!(b, ParsedBlock::ToolUse { .. }));
    let should_stop = stop_reason == "end_turn" || !has_tool_uses;

    Ok(AgentTurnResult { blocks, should_stop, input_tokens, output_tokens })
}
