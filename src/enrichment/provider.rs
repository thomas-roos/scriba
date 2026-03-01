//! Common LLM provider trait and types.

use async_trait::async_trait;
use std::fmt;
use thiserror::Error;

/// Errors that can occur when interacting with an LLM provider.
#[derive(Error, Debug)]
pub enum ProviderError {
    #[error("Authentication failed: {message}")]
    AuthFailure { message: String },

    #[error("Rate limited: {message}")]
    RateLimited { message: String },

    #[error("Request timeout after {seconds}s")]
    Timeout { seconds: u64 },

    #[error("Network error: {message}")]
    Network { message: String },

    #[error("Failed to parse response: {message}")]
    ParseError { message: String },

    #[error("Provider error: {message}")]
    Other { message: String },
}

/// Supported LLM provider kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Anthropic,
    OpenAI,
    Google,
    Ollama,
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderKind::Anthropic => write!(f, "Anthropic"),
            ProviderKind::OpenAI => write!(f, "OpenAI"),
            ProviderKind::Google => write!(f, "Google"),
            ProviderKind::Ollama => write!(f, "Ollama"),
        }
    }
}

impl ProviderKind {
    /// Default model for this provider.
    pub fn default_model(&self) -> &str {
        match self {
            ProviderKind::Anthropic => "claude-sonnet-4-6",
            ProviderKind::OpenAI => "gpt-5.2",
            ProviderKind::Google => "gemini-2.5-flash",
            ProviderKind::Ollama => "mistral:latest",
        }
    }
}

/// Trait for LLM providers that can generate text from prompts.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Generate a JSON response from a prompt.
    async fn generate(&self, prompt: &str) -> Result<String, ProviderError>;

    /// Generate a plain text response (no JSON format constraint).
    async fn generate_text(&self, prompt: &str) -> Result<String, ProviderError>;

    /// Stream a plain text response, sending chunks through the channel.
    /// Default implementation falls back to non-streaming (one big chunk).
    async fn generate_text_stream(
        &self,
        prompt: &str,
        tx: tokio::sync::mpsc::Sender<String>,
    ) -> Result<(), ProviderError> {
        let response = self.generate_text(prompt).await?;
        let _ = tx.send(response).await;
        Ok(())
    }

    /// Check if the provider is reachable and configured.
    async fn health_check(&self) -> Result<(), ProviderError>;

    /// Provider display name (for UI).
    fn display_name(&self) -> &str;

    /// Model name in use.
    fn model(&self) -> &str;
}
