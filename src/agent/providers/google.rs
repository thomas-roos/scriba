//! Google Gemini agent provider — streamGenerateContent with function calling.

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

use super::{AgentProvider, AgentTurnResult, ParsedBlock};
use crate::agent::loop_runner::AgentEvent;
use crate::enrichment::ProviderError;

pub struct GoogleAgentProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl GoogleAgentProvider {
    pub fn new(api_key: String, model: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to create HTTP client");
        Self { client, api_key, model }
    }

    fn stream_url(&self) -> String {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?key={}&alt=sse",
            self.model, self.api_key
        )
    }

    fn generate_url(&self) -> String {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        )
    }
}

#[async_trait]
impl AgentProvider for GoogleAgentProvider {
    async fn send_turn(
        &self,
        system_prompt: &str,
        messages: &[Value],
        tool_defs: &[Value],
        tx: &mpsc::Sender<AgentEvent>,
    ) -> Result<AgentTurnResult, ProviderError> {
        let mut body = serde_json::json!({
            "contents": messages,
            "systemInstruction": {
                "parts": [{ "text": system_prompt }]
            },
            "generationConfig": {
                "temperature": 0.3,
                "maxOutputTokens": 4096,
            },
        });

        if !tool_defs.is_empty() {
            body["tools"] = serde_json::json!([{
                "functionDeclarations": tool_defs
            }]);
        }

        let response = self.client
            .post(&self.stream_url())
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
                message: format!("Gemini API error ({}): {}", status, body),
            });
        }

        parse_gemini_sse(response, tx).await
    }

    fn translate_tool_definitions(&self, canonical_tools: &[Value]) -> Vec<Value> {
        canonical_tools
            .iter()
            .map(|tool| {
                let mut decl = serde_json::json!({
                    "name": tool.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    "description": tool.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                });
                if let Some(schema) = tool.get("input_schema") {
                    decl["parameters"] = schema.clone();
                }
                decl
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
                "Assistant" => "model",
                _ => continue,
            };
            messages.push(serde_json::json!({
                "role": api_role,
                "parts": [{ "text": content }]
            }));
        }
        messages.push(serde_json::json!({
            "role": "user",
            "parts": [{ "text": user_message }]
        }));
        messages
    }

    fn append_assistant_message(&self, messages: &mut Vec<Value>, blocks: &[ParsedBlock]) {
        let mut parts: Vec<Value> = Vec::new();
        for block in blocks {
            match block {
                ParsedBlock::Text(text) => {
                    parts.push(serde_json::json!({ "text": text }));
                }
                ParsedBlock::ToolUse { name, input, .. } => {
                    parts.push(serde_json::json!({
                        "functionCall": {
                            "name": name,
                            "args": input,
                        }
                    }));
                }
            }
        }
        messages.push(serde_json::json!({
            "role": "model",
            "parts": parts,
        }));
    }

    fn append_tool_results(
        &self,
        messages: &mut Vec<Value>,
        results: &[(String, String, String)],
    ) {
        let parts: Vec<Value> = results
            .iter()
            .map(|(_id, name, content)| {
                serde_json::json!({
                    "functionResponse": {
                        "name": name,
                        "response": { "result": content }
                    }
                })
            })
            .collect();
        messages.push(serde_json::json!({
            "role": "user",
            "parts": parts,
        }));
    }

    async fn compact_history(&self, prompt: &str) -> Result<String, ProviderError> {
        let body = serde_json::json!({
            "contents": [{
                "parts": [{ "text": prompt }]
            }],
            "generationConfig": {
                "temperature": 0.3,
                "maxOutputTokens": 1024,
            },
        });

        let response = self.client
            .post(&self.generate_url())
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

        json.get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("text"))
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or(ProviderError::ParseError {
                message: "Empty compaction response".to_string(),
            })
    }

    fn display_name(&self) -> &str {
        "Google Gemini"
    }
}

/// Parse Gemini streaming SSE response with functionCall support.
async fn parse_gemini_sse(
    response: reqwest::Response,
    tx: &mpsc::Sender<AgentEvent>,
) -> Result<AgentTurnResult, ProviderError> {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut blocks: Vec<ParsedBlock> = Vec::new();
    let mut current_text = String::new();
    let mut tool_call_counter: u32 = 0;
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

            if !line.starts_with("data: ") {
                continue;
            }
            let json_str = &line[6..];

            let event: Value = match serde_json::from_str(json_str) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Some(error) = event.get("error") {
                let msg = error
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown Gemini error");
                return Err(ProviderError::Other { message: msg.to_string() });
            }

            // Extract usage metadata
            if let Some(metadata) = event.get("usageMetadata") {
                if let Some(t) = metadata.get("promptTokenCount").and_then(|v| v.as_u64()) {
                    input_tokens = t as u32;
                }
                if let Some(t) = metadata.get("candidatesTokenCount").and_then(|v| v.as_u64()) {
                    output_tokens = t as u32;
                }
            }

            let parts = event
                .get("candidates")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("content"))
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array());

            if let Some(parts) = parts {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        current_text.push_str(text);
                        let _ = tx.send(AgentEvent::Chunk(text.to_string())).await;
                    }
                    if let Some(fc) = part.get("functionCall") {
                        if !current_text.is_empty() {
                            blocks.push(ParsedBlock::Text(current_text.clone()));
                            current_text.clear();
                        }
                        let name = fc.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                        let args = fc.get("args").cloned().unwrap_or(Value::Object(serde_json::Map::new()));
                        tool_call_counter += 1;
                        let id = format!("gemini_tc_{}", tool_call_counter);
                        blocks.push(ParsedBlock::ToolUse { id, name, input: args });
                    }
                }
            }
        }
    }

    if !current_text.is_empty() {
        blocks.push(ParsedBlock::Text(current_text));
    }

    let has_tool_uses = blocks.iter().any(|b| matches!(b, ParsedBlock::ToolUse { .. }));
    let should_stop = !has_tool_uses;

    Ok(AgentTurnResult { blocks, should_stop, input_tokens, output_tokens })
}
