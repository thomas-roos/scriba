//! Anthropic Claude API client.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::provider::{LlmProvider, ProviderError};

/// Anthropic Claude API client.
pub struct AnthropicClient {
    client: Client,
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<Message<'a>>,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: String,
}

#[derive(Deserialize)]
struct AnthropicErrorResponse {
    error: Option<AnthropicErrorDetail>,
}

#[derive(Deserialize)]
struct AnthropicErrorDetail {
    message: String,
}

impl AnthropicClient {
    pub fn new(api_key: &str, model: Option<&str>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            api_key: api_key.to_string(),
            model: model
                .unwrap_or("claude-sonnet-4-6")
                .to_string(),
        }
    }

    async fn send_request(&self, prompt: &str, max_tokens: u32) -> Result<String, ProviderError> {
        let request = AnthropicRequest {
            model: &self.model,
            max_tokens,
            messages: vec![Message {
                role: "user",
                content: prompt,
            }],
        };

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ProviderError::Timeout { seconds: 300 }
                } else if e.is_connect() {
                    ProviderError::Network {
                        message: e.to_string(),
                    }
                } else {
                    ProviderError::Other {
                        message: e.to_string(),
                    }
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let error_msg = serde_json::from_str::<AnthropicErrorResponse>(&body)
                .ok()
                .and_then(|e| e.error)
                .map(|e| e.message)
                .unwrap_or(body);

            return match status.as_u16() {
                401 => Err(ProviderError::AuthFailure {
                    message: error_msg,
                }),
                429 => Err(ProviderError::RateLimited {
                    message: error_msg,
                }),
                _ => Err(ProviderError::Other {
                    message: format!("HTTP {}: {}", status, error_msg),
                }),
            };
        }

        let result: AnthropicResponse =
            response.json().await.map_err(|e| ProviderError::ParseError {
                message: e.to_string(),
            })?;

        result
            .content
            .into_iter()
            .next()
            .map(|block| block.text)
            .ok_or(ProviderError::ParseError {
                message: "Empty response from Anthropic".to_string(),
            })
    }
}

#[async_trait]
impl LlmProvider for AnthropicClient {
    async fn generate(&self, prompt: &str) -> Result<String, ProviderError> {
        let json_prompt = format!(
            "{}\n\nRespond with valid JSON only. No markdown fences, no explanation.",
            prompt
        );
        self.send_request(&json_prompt, 4096).await
    }

    async fn generate_text(&self, prompt: &str) -> Result<String, ProviderError> {
        self.send_request(prompt, 2048).await
    }

    async fn generate_text_stream(
        &self,
        prompt: &str,
        tx: tokio::sync::mpsc::Sender<String>,
    ) -> Result<(), ProviderError> {
        use futures_util::StreamExt;

        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "stream": true,
            "messages": [{"role": "user", "content": prompt}]
        });

        let response = self
            .client
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
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Other {
                message: format!("Anthropic streaming error: {}", body),
            });
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ProviderError::Network { message: e.to_string() })?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].trim().to_string();
                buffer = buffer[newline_pos + 1..].to_string();

                if line.starts_with("data: ") {
                    let json_str = &line[6..];
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(json_str) {
                        if event.get("type").and_then(|t| t.as_str()) == Some("content_block_delta") {
                            if let Some(text) = event
                                .get("delta")
                                .and_then(|d| d.get("text"))
                                .and_then(|t| t.as_str())
                            {
                                let _ = tx.send(text.to_string()).await;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn health_check(&self) -> Result<(), ProviderError> {
        let request = AnthropicRequest {
            model: &self.model,
            max_tokens: 10,
            messages: vec![Message {
                role: "user",
                content: "Say OK",
            }],
        };

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| ProviderError::Network {
                message: e.to_string(),
            })?;

        let status = response.status();
        if status.as_u16() == 401 {
            return Err(ProviderError::AuthFailure {
                message: "Invalid API key".to_string(),
            });
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Other {
                message: format!("HTTP {}: {}", status, body),
            });
        }

        Ok(())
    }

    fn display_name(&self) -> &str {
        "Anthropic (Claude)"
    }

    fn model(&self) -> &str {
        &self.model
    }
}
