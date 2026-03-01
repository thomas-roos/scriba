//! Agent loop — orchestrates tool-use chat across any LLM provider.
//!
//! The loop sends messages to the LLM via an `AgentProvider`, processes tool_use
//! blocks, executes tools, appends results, and continues until the model signals
//! it is done or we hit the iteration limit.

use crate::database::Database;
use super::providers::{AgentProvider, ParsedBlock};
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

const MAX_ITERATIONS: usize = 10;

/// Run the agent loop. Streams events to `tx` for the TUI.
///
/// `provider` — the LLM provider to use (Anthropic, OpenAI, Google, Ollama).
/// `system_prompt` — the system message for the agent.
/// `history` — previous conversation turns (user/assistant pairs).
/// `user_message` — the latest user message.
/// `tx` — channel to send events to the TUI.
pub async fn run_agent_loop(
    provider: Box<dyn AgentProvider>,
    system_prompt: String,
    history: Vec<(String, String)>,
    user_message: String,
    tx: mpsc::Sender<AgentEvent>,
) {
    let mut db = match Database::new() {
        Ok(db) => db,
        Err(e) => {
            let _ = tx.send(AgentEvent::Error(format!("Database error: {}", e))).await;
            return;
        }
    };

    let canonical_tools = tools::all_tool_definitions();
    let tool_defs = provider.translate_tool_definitions(&canonical_tools);

    // Build initial messages from history using the provider's format
    let mut messages = provider.build_messages_from_history(&history, &user_message);

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

        // Send the turn to the provider
        let turn_result = match provider.send_turn(&system_prompt, &messages, &tool_defs, &tx).await {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(AgentEvent::Error(format!("{} error: {}", provider.display_name(), e))).await;
                return;
            }
        };

        // Emit usage so the TUI can update the context window bar
        let _ = tx.send(AgentEvent::Usage {
            input_tokens: turn_result.input_tokens,
            output_tokens: turn_result.output_tokens,
        }).await;

        // Track whether this iteration produced any text
        let iteration_had_text = turn_result.blocks.iter().any(|b| matches!(b, ParsedBlock::Text(t) if !t.is_empty()));
        if iteration_had_text {
            has_emitted_text = true;
        }

        // Collect tool uses
        let tool_uses: Vec<(String, String, Value)> = turn_result.blocks.iter()
            .filter_map(|b| match b {
                ParsedBlock::ToolUse { id, name, input } => Some((id.clone(), name.clone(), input.clone())),
                _ => None,
            })
            .collect();

        // Append the assistant message using the provider's format
        provider.append_assistant_message(&mut messages, &turn_result.blocks);

        if turn_result.should_stop || tool_uses.is_empty() {
            let _ = tx.send(AgentEvent::Done).await;
            break;
        }

        // Execute each tool and collect results
        let mut tool_results: Vec<(String, String, String)> = Vec::new();
        for (tool_id, tool_name, tool_input) in &tool_uses {
            let input_summary = tools::summarize_input(tool_name, tool_input);
            let _ = tx.send(AgentEvent::ToolCall {
                name: tool_name.clone(),
                input_summary: input_summary.clone(),
            }).await;

            let result = tools::execute_tool(tool_name, tool_input, &mut db);
            let output_summary = tools::summarize_tool_result(tool_name, &result);

            let _ = tx.send(AgentEvent::ToolResult {
                name: tool_name.clone(),
                output_summary: output_summary.clone(),
            }).await;

            tool_results.push((tool_id.clone(), tool_name.clone(), result));
        }

        // Append tool results using the provider's format
        provider.append_tool_results(&mut messages, &tool_results);
    }
}
