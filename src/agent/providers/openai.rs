//! OpenAI agent provider — Chat Completions API with streaming tool calls.

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

use super::{AgentProvider, AgentTurnResult, ParsedBlock};
use crate::agent::loop_runner::AgentEvent;
use crate::enrichment::ProviderError;

pub struct OpenAIAgentProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl OpenAIAgentProvider {
    pub fn new(api_key: String, model: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to create HTTP client");
        Self { client, api_key, model }
    }
}

#[async_trait]
impl AgentProvider for OpenAIAgentProvider {
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
            "max_completion_tokens": 4096,
            "stream": true,
            "stream_options": { "include_usage": true },
        });

        if !tool_defs.is_empty() {
            body["tools"] = Value::Array(tool_defs.to_vec());
        }

        let response = self.client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
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
                message: format!("OpenAI API error ({}): {}", status, body),
            });
        }

        parse_openai_sse(response, tx).await
    }

    fn translate_tool_definitions(&self, canonical_tools: &[Value]) -> Vec<Value> {
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
                ParsedBlock::ToolUse { id, name, input } => {
                    tool_calls.push(serde_json::json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": serde_json::to_string(input).unwrap_or_default(),
                        }
                    }));
                }
            }
        }

        let mut msg = serde_json::json!({ "role": "assistant" });
        if !content_text.is_empty() {
            msg["content"] = Value::String(content_text);
        } else {
            msg["content"] = Value::Null;
        }
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
        for (tool_call_id, _name, content) in results {
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": tool_call_id,
                "content": content,
            }));
        }
    }

    async fn compact_history(&self, prompt: &str) -> Result<String, ProviderError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_completion_tokens": 1024,
            "messages": [
                { "role": "system", "content": "You are a helpful assistant that summarizes conversations." },
                { "role": "user", "content": prompt },
            ],
        });

        let response = self.client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
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

        json.get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or(ProviderError::ParseError {
                message: "Empty compaction response".to_string(),
            })
    }

    fn display_name(&self) -> &str {
        "OpenAI"
    }
}

/// Parse OpenAI streaming SSE response with tool_calls support.
async fn parse_openai_sse(
    response: reqwest::Response,
    tx: &mpsc::Sender<AgentEvent>,
) -> Result<AgentTurnResult, ProviderError> {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut blocks: Vec<ParsedBlock> = Vec::new();

    let mut current_text = String::new();
    let mut tool_call_builders: std::collections::HashMap<usize, (String, String, String)> =
        std::collections::HashMap::new();
    let mut finish_reason = String::new();
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ProviderError::Network {
            message: format!("Stream error: {}", e),
        })?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line == "data: [DONE]" {
                continue;
            }
            if !line.starts_with("data: ") {
                continue;
            }
            let json_str = &line[6..];

            let event: Value = match serde_json::from_str(json_str) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Extract usage from the final chunk (stream_options: include_usage)
            if let Some(usage) = event.get("usage") {
                if let Some(t) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                    input_tokens = t as u32;
                }
                if let Some(t) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                    output_tokens = t as u32;
                }
            }

            let choice = match event.get("choices").and_then(|c| c.get(0)) {
                Some(c) => c,
                None => continue,
            };

            if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                finish_reason = reason.to_string();
            }

            let delta = match choice.get("delta") {
                Some(d) => d,
                None => continue,
            };

            // Text content
            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                current_text.push_str(content);
                let _ = tx.send(AgentEvent::Chunk(content.to_string())).await;
            }

            // Tool calls (incremental)
            if let Some(tool_calls) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
                for tc in tool_calls {
                    let index = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;

                    let entry = tool_call_builders
                        .entry(index)
                        .or_insert_with(|| (String::new(), String::new(), String::new()));

                    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                        entry.0 = id.to_string();
                    }
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                            entry.1 = name.to_string();
                        }
                        if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                            entry.2.push_str(args);
                        }
                    }
                }
            }
        }
    }

    // Flush text
    if !current_text.is_empty() {
        blocks.push(ParsedBlock::Text(current_text));
    }

    // Flush tool calls (sorted by index)
    let mut tool_indices: Vec<usize> = tool_call_builders.keys().copied().collect();
    tool_indices.sort();
    for idx in tool_indices {
        if let Some((id, name, args_json)) = tool_call_builders.remove(&idx) {
            let input: Value = serde_json::from_str(&args_json)
                .unwrap_or(Value::Object(serde_json::Map::new()));
            blocks.push(ParsedBlock::ToolUse { id, name, input });
        }
    }

    let has_tool_uses = blocks.iter().any(|b| matches!(b, ParsedBlock::ToolUse { .. }));
    let should_stop = finish_reason == "stop" || !has_tool_uses;

    Ok(AgentTurnResult { blocks, should_stop, input_tokens, output_tokens })
}
