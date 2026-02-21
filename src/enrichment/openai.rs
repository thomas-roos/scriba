//! OpenAI API client.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::provider::{LlmProvider, ProviderError};

/// OpenAI API client.
pub struct OpenAIClient {
    client: Client,
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ResponseFormat<'a> {
    r#type: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat<'a>>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct OpenAIErrorResponse {
    error: Option<OpenAIErrorDetail>,
}

#[derive(Deserialize)]
struct OpenAIErrorDetail {
    message: String,
}

impl OpenAIClient {
    pub fn new(api_key: &str, model: Option<&str>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            api_key: api_key.to_string(),
            model: model.unwrap_or("gpt-4o").to_string(),
        }
    }

    async fn send_request(
        &self,
        prompt: &str,
        max_tokens: u32,
        json_mode: bool,
    ) -> Result<String, ProviderError> {
        let response_format = if json_mode {
            Some(ResponseFormat {
                r#type: "json_object",
            })
        } else {
            None
        };

        let request = ChatRequest {
            model: &self.model,
            messages: vec![ChatMessage {
                role: "user",
                content: prompt,
            }],
            temperature: 0.3,
            max_tokens,
            response_format,
        };

        let response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
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
            let error_msg = serde_json::from_str::<OpenAIErrorResponse>(&body)
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

        let result: ChatResponse =
            response.json().await.map_err(|e| ProviderError::ParseError {
                message: e.to_string(),
            })?;

        result
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .ok_or(ProviderError::ParseError {
                message: "Empty response from OpenAI".to_string(),
            })
    }
}

#[async_trait]
impl LlmProvider for OpenAIClient {
    async fn generate(&self, prompt: &str) -> Result<String, ProviderError> {
        self.send_request(prompt, 4096, true).await
    }

    async fn generate_text(&self, prompt: &str) -> Result<String, ProviderError> {
        self.send_request(prompt, 2048, false).await
    }

    async fn generate_text_stream(
        &self,
        prompt: &str,
        tx: tokio::sync::mpsc::Sender<String>,
    ) -> Result<(), ProviderError> {
        use futures_util::StreamExt;

        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0.3,
            "max_tokens": 4096,
            "stream": true
        });

        let response = self
            .client
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
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Other {
                message: format!("OpenAI streaming error: {}", body),
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

                if line == "data: [DONE]" {
                    return Ok(());
                }

                if line.starts_with("data: ") {
                    let json_str = &line[6..];
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(json_str) {
                        if let Some(content) = event
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("delta"))
                            .and_then(|d| d.get("content"))
                            .and_then(|t| t.as_str())
                        {
                            let _ = tx.send(content.to_string()).await;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn health_check(&self) -> Result<(), ProviderError> {
        let request = ChatRequest {
            model: &self.model,
            messages: vec![ChatMessage {
                role: "user",
                content: "Say OK",
            }],
            temperature: 0.3,
            max_tokens: 10,
            response_format: None,
        };

        let response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
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
        "OpenAI (GPT)"
    }

    fn model(&self) -> &str {
        &self.model
    }
}
