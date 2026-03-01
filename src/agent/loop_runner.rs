//! Agent loop — orchestrates tool-use chat with the Anthropic Messages API.
//!
//! The loop sends messages to the LLM, processes tool_use blocks, executes tools,
//! appends results, and continues until the model returns `end_turn` or we hit
//! the iteration limit.

use crate::database::Database;
use super::tools;
use serde_json::Value;
use tokio::sync::mpsc;

/// Events emitted by the agent loop for the TUI to display.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Status message (e.g. "Thinking...")
    Status(String),
    /// Text chunk from the assistant's response
    Chunk(String),
    /// Agent is calling a tool
    ToolCall { name: String, input_summary: String },
    /// Tool returned a result
    ToolResult { name: String, output_summary: String },
    /// Token usage from the API response
    Usage { input_tokens: u32, output_tokens: u32 },
    /// Agent finished
    Done,
    /// Error occurred
    Error(String),
}

/// Content block in a message (text or tool use/result).
#[derive(Debug, Clone)]
pub enum MessageContent {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

/// A message in the agent conversation.
#[derive(Debug, Clone)]
pub struct AgentMessage {
    pub role: String,
    pub content: Vec<MessageContent>,
}

const MAX_ITERATIONS: usize = 10;

/// Run the agent loop. Streams events to `tx` for the TUI.
///
/// `system_prompt` — the system message for the agent.
/// `history` — previous conversation turns (user/assistant pairs).
/// `user_message` — the latest user message.
/// `api_key` — Anthropic API key.
/// `model` — model name (e.g. "claude-opus-4-6").
/// `tx` — channel to send events to the TUI.
pub async fn run_agent_loop(
    system_prompt: String,
    history: Vec<(String, String)>,
    user_message: String,
    api_key: String,
    model: String,
    tx: mpsc::Sender<AgentEvent>,
) {
    let db = match Database::new() {
        Ok(db) => db,
        Err(e) => {
            let _ = tx.send(AgentEvent::Error(format!("Database error: {}", e))).await;
            return;
        }
    };

    let tool_defs = tools::all_tool_definitions();

    // Build initial messages from history
    let mut messages: Vec<Value> = Vec::new();
    for (role, content) in &history {
        let api_role = match role.as_str() {
            "User" => "user",
            "Assistant" => "assistant",
            _ => continue, // skip System messages in history
        };
        messages.push(serde_json::json!({
            "role": api_role,
            "content": content,
        }));
    }
    // Add the new user message
    messages.push(serde_json::json!({
        "role": "user",
        "content": user_message,
    }));

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(AgentEvent::Error(format!("HTTP client error: {}", e))).await;
            return;
        }
    };

    let mut iterations = 0;
    let mut has_emitted_text = false;

    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            let _ = tx.send(AgentEvent::Status("Reached maximum iterations".to_string())).await;
            let _ = tx.send(AgentEvent::Done).await;
            break;
        }

        // If this is a continuation after tool use and we already emitted text,
        // add a line break so the next text doesn't stick to the previous one.
        if iterations > 1 && has_emitted_text {
            let _ = tx.send(AgentEvent::Chunk("\n\n".to_string())).await;
        }

        let _ = tx.send(AgentEvent::Status("Thinking...".to_string())).await;

        // Build the API request
        let body = serde_json::json!({
            "model": model,
            "max_tokens": 4096,
            "system": system_prompt,
            "tools": tool_defs,
            "messages": messages,
            "stream": true,
        });

        let response = match client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(AgentEvent::Error(format!("Network error: {}", e))).await;
                return;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let _ = tx.send(AgentEvent::Error(format!("API error ({}): {}", status, body))).await;
            return;
        }

        // Parse the streaming response
        let parse_result = parse_streaming_response(response, &tx).await;
        let (content_blocks, stop_reason, input_tokens, output_tokens) = match parse_result {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(AgentEvent::Error(format!("Parse error: {}", e))).await;
                return;
            }
        };

        // Emit usage so the TUI can update the context window bar
        let _ = tx.send(AgentEvent::Usage { input_tokens, output_tokens }).await;

        // Track whether this iteration produced any text
        let iteration_had_text = content_blocks.iter().any(|b| matches!(b, ParsedBlock::Text(t) if !t.is_empty()));
        if iteration_had_text {
            has_emitted_text = true;
        }

        // Build the assistant message to append to conversation
        let mut assistant_content: Vec<Value> = Vec::new();
        let mut tool_uses: Vec<(String, String, Value)> = Vec::new(); // (id, name, input)

        for block in &content_blocks {
            match block {
                ParsedBlock::Text(text) => {
                    assistant_content.push(serde_json::json!({
                        "type": "text",
                        "text": text,
                    }));
                }
                ParsedBlock::ToolUse { id, name, input } => {
                    assistant_content.push(serde_json::json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input,
                    }));
                    tool_uses.push((id.clone(), name.clone(), input.clone()));
                }
            }
        }

        // Append assistant message
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": assistant_content,
        }));

        if stop_reason == "end_turn" || tool_uses.is_empty() {
            let _ = tx.send(AgentEvent::Done).await;
            break;
        }

        // Execute each tool and build tool_result messages
        let mut tool_results: Vec<Value> = Vec::new();
        for (tool_id, tool_name, tool_input) in &tool_uses {
            let input_summary = summarize_input(tool_name, tool_input);
            let _ = tx.send(AgentEvent::ToolCall {
                name: tool_name.clone(),
                input_summary: input_summary.clone(),
            }).await;

            let result = tools::execute_tool(tool_name, tool_input, &db);
            let output_summary = tools::summarize_tool_result(tool_name, &result);

            let _ = tx.send(AgentEvent::ToolResult {
                name: tool_name.clone(),
                output_summary: output_summary.clone(),
            }).await;

            tool_results.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": tool_id,
                "content": result,
            }));
        }

        // Append tool results as a user message
        messages.push(serde_json::json!({
            "role": "user",
            "content": tool_results,
        }));
    }
}

/// Summarize tool input for display.
fn summarize_input(name: &str, input: &Value) -> String {
    match name {
        "search_transcripts" => {
            let query = input.get("query").and_then(|v| v.as_str()).unwrap_or("?");
            format!("\"{}\"", query)
        }
        "get_recording" | "get_transcript" => {
            let id = input.get("id")
                .or_else(|| input.get("recording_id"))
                .and_then(|v| v.as_i64())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".to_string());
            format!("id={}", id)
        }
        "get_entity" => {
            let id = input.get("id").and_then(|v| v.as_i64())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".to_string());
            format!("id={}", id)
        }
        "get_recordings_for_entity" => {
            let id = input.get("entity_id").and_then(|v| v.as_i64())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".to_string());
            format!("entity_id={}", id)
        }
        "list_recordings" => {
            if let Some(limit) = input.get("limit").and_then(|v| v.as_i64()) {
                format!("limit={}", limit)
            } else {
                "all".to_string()
            }
        }
        "list_entities" => {
            let mut parts = Vec::new();
            if let Some(t) = input.get("entity_type").and_then(|v| v.as_str()) {
                parts.push(format!("type={}", t));
            }
            if let Some(l) = input.get("limit").and_then(|v| v.as_i64()) {
                parts.push(format!("limit={}", l));
            }
            if parts.is_empty() { "all".to_string() } else { parts.join(", ") }
        }
        _ => String::new(),
    }
}

// ─── Streaming SSE Parser ────────────────────────────────────────────────────

#[derive(Debug)]
enum ParsedBlock {
    Text(String),
    ToolUse { id: String, name: String, input: Value },
}

/// Parse a streaming SSE response from the Anthropic API.
/// Extracts text deltas (streamed to tx) and tool_use blocks.
/// Returns the collected content blocks and the stop_reason.
async fn parse_streaming_response(
    response: reqwest::Response,
    tx: &mpsc::Sender<AgentEvent>,
) -> Result<(Vec<ParsedBlock>, String, u32, u32), String> {
    use futures_util::StreamExt;

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut blocks: Vec<ParsedBlock> = Vec::new();
    let mut stop_reason = String::from("end_turn");
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;

    // Track current content blocks being built
    let mut current_text = String::new();
    let mut current_tool: Option<(String, String)> = None; // (id, name)
    let mut current_tool_input_json = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Stream error: {}", e))?;
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
                    let block_type = content_block.get("type").and_then(|t| t.as_str()).unwrap_or("");

                    if block_type == "tool_use" {
                        // Flush any accumulated text
                        if !current_text.is_empty() {
                            blocks.push(ParsedBlock::Text(current_text.clone()));
                            current_text.clear();
                        }
                        let id = content_block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let name = content_block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
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
                        if let Some(json_part) = delta.get("partial_json").and_then(|t| t.as_str()) {
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
                    if let Some(reason) = event.get("delta")
                        .and_then(|d| d.get("stop_reason"))
                        .and_then(|r| r.as_str())
                    {
                        stop_reason = reason.to_string();
                    }
                    if let Some(tokens) = event.get("usage")
                        .and_then(|u| u.get("output_tokens"))
                        .and_then(|t| t.as_u64())
                    {
                        output_tokens = tokens as u32;
                    }
                }
                "error" => {
                    let msg = event.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("Unknown streaming error");
                    return Err(msg.to_string());
                }
                _ => {}
            }
        }
    }

    // Flush remaining text
    if !current_text.is_empty() {
        blocks.push(ParsedBlock::Text(current_text));
    }

    Ok((blocks, stop_reason, input_tokens, output_tokens))
}
