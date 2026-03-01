//! Agent provider trait and factory.
//!
//! Defines the `AgentProvider` abstraction that lets the agent loop work
//! identically across Anthropic, OpenAI, Google Gemini, and Ollama.

pub mod anthropic;
pub mod google;
pub mod ollama;
pub mod openai;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

use super::loop_runner::AgentEvent;
use crate::core::config::{CloudProvider, EnrichmentConfig, EnrichmentMode};
use crate::enrichment::ProviderError;

/// A parsed content block returned by a single LLM turn.
#[derive(Debug)]
pub enum ParsedBlock {
    Text(String),
    ToolUse { id: String, name: String, input: Value },
}

/// Result of a single agent turn (one API call).
pub struct AgentTurnResult {
    /// Content blocks parsed from the response.
    pub blocks: Vec<ParsedBlock>,
    /// Whether the model signaled it is done (no more tool calls desired).
    pub should_stop: bool,
    /// Token usage from the API response (for context window tracking).
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Trait implemented by each provider to handle one turn of the agent loop.
#[async_trait]
pub trait AgentProvider: Send + Sync {
    /// Send one turn to the LLM. Streams text chunks via `tx` during the call.
    async fn send_turn(
        &self,
        system_prompt: &str,
        messages: &[Value],
        tool_defs: &[Value],
        tx: &mpsc::Sender<AgentEvent>,
    ) -> Result<AgentTurnResult, ProviderError>;

    /// Convert canonical (Anthropic-format) tool definitions to this provider's format.
    fn translate_tool_definitions(&self, canonical_tools: &[Value]) -> Vec<Value>;

    /// Build the initial messages array from conversation history + new user message.
    fn build_messages_from_history(
        &self,
        history: &[(String, String)],
        user_message: &str,
    ) -> Vec<Value>;

    /// Append the assistant's response blocks to the messages array.
    fn append_assistant_message(&self, messages: &mut Vec<Value>, blocks: &[ParsedBlock]);

    /// Append tool results to the messages array.
    /// Each tuple is (tool_call_id, tool_name, result_content).
    fn append_tool_results(
        &self,
        messages: &mut Vec<Value>,
        results: &[(String, String, String)],
    );

    /// Make a non-streaming API call to summarize messages for compaction.
    /// Default implementation returns an error (provider doesn't support compaction).
    async fn compact_history(
        &self,
        _prompt: &str,
    ) -> Result<String, ProviderError> {
        Err(ProviderError::Other {
            message: "Compaction not supported by this provider".to_string(),
        })
    }

    /// Provider display name for status messages.
    fn display_name(&self) -> &str;
}

/// Create an `AgentProvider` from enrichment configuration.
pub fn create_agent_provider(config: &EnrichmentConfig) -> Box<dyn AgentProvider> {
    match &config.mode {
        EnrichmentMode::Cloud { provider, api_key, model } => {
            let effective_key = if api_key.is_empty() {
                std::env::var(provider.env_var_name()).unwrap_or_default()
            } else {
                api_key.clone()
            };
            let model_name = model
                .as_deref()
                .unwrap_or_else(|| provider.default_model())
                .to_string();

            match provider {
                CloudProvider::Anthropic => {
                    Box::new(anthropic::AnthropicAgentProvider::new(effective_key, model_name))
                }
                CloudProvider::OpenAI => {
                    Box::new(openai::OpenAIAgentProvider::new(effective_key, model_name))
                }
                CloudProvider::Google => {
                    Box::new(google::GoogleAgentProvider::new(effective_key, model_name))
                }
            }
        }
        EnrichmentMode::Local { ollama_endpoint, ollama_model } => {
            Box::new(ollama::OllamaAgentProvider::new(
                ollama_endpoint.clone(),
                ollama_model.clone(),
            ))
        }
    }
}
