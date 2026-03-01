//! Google Gemini API client.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::provider::{LlmProvider, ProviderError};

/// Google Gemini API client.
pub struct GoogleClient {
    client: Client,
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct GeminiRequest<'a> {
    contents: Vec<GeminiContent<'a>>,
    #[serde(rename = "generationConfig")]
    generation_config: GenerationConfig<'a>,
}

#[derive(Serialize)]
struct GeminiContent<'a> {
    parts: Vec<GeminiPart<'a>>,
}

#[derive(Serialize)]
struct GeminiPart<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct GenerationConfig<'a> {
    temperature: f32,
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "responseMimeType")]
    response_mime_type: Option<&'a str>,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
    error: Option<GeminiErrorDetail>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiContentResponse,
}

#[derive(Deserialize)]
struct GeminiContentResponse {
    parts: Vec<GeminiPartResponse>,
}

#[derive(Deserialize)]
struct GeminiPartResponse {
    text: String,
}

#[derive(Deserialize)]
struct GeminiErrorDetail {
    message: String,
    #[allow(dead_code)]
    #[serde(default)]
    code: u32,
}

impl GoogleClient {
    pub fn new(api_key: &str, model: Option<&str>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            api_key: api_key.to_string(),
            model: model.unwrap_or("gemini-2.5-flash").to_string(),
        }
    }

    fn api_url(&self) -> String {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        )
    }

    async fn send_request(
        &self,
        prompt: &str,
        max_tokens: u32,
        json_mode: bool,
    ) -> Result<String, ProviderError> {
        let response_mime_type = if json_mode {
            Some("application/json")
        } else {
            None
        };

        let request = GeminiRequest {
            contents: vec![GeminiContent {
                parts: vec![GeminiPart { text: prompt }],
            }],
            generation_config: GenerationConfig {
                temperature: 0.3,
                max_output_tokens: max_tokens,
                response_mime_type,
            },
        };

        let response = self
            .client
            .post(&self.api_url())
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
            let error_msg = serde_json::from_str::<GeminiResponse>(&body)
                .ok()
                .and_then(|r| r.error)
                .map(|e| e.message)
                .unwrap_or(body);

            return match status.as_u16() {
                401 | 403 => Err(ProviderError::AuthFailure {
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

        let result: GeminiResponse =
            response.json().await.map_err(|e| ProviderError::ParseError {
                message: e.to_string(),
            })?;

        if let Some(error) = result.error {
            return Err(ProviderError::Other {
                message: error.message,
            });
        }

        result
            .candidates
            .and_then(|c| c.into_iter().next())
            .and_then(|c| c.content.parts.into_iter().next())
            .map(|p| p.text)
            .ok_or(ProviderError::ParseError {
                message: "Empty response from Google".to_string(),
            })
    }
}

#[async_trait]
impl LlmProvider for GoogleClient {
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

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?key={}&alt=sse",
            self.model, self.api_key
        );

        let request = GeminiRequest {
            contents: vec![GeminiContent {
                parts: vec![GeminiPart { text: prompt }],
            }],
            generation_config: GenerationConfig {
                temperature: 0.3,
                max_output_tokens: 4096,
                response_mime_type: None,
            },
        };

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
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
                message: format!("Google streaming error: {}", body),
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
                        if let Some(text) = event
                            .get("candidates")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("content"))
                            .and_then(|c| c.get("parts"))
                            .and_then(|p| p.get(0))
                            .and_then(|p| p.get("text"))
                            .and_then(|t| t.as_str())
                        {
                            let _ = tx.send(text.to_string()).await;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn health_check(&self) -> Result<(), ProviderError> {
        let request = GeminiRequest {
            contents: vec![GeminiContent {
                parts: vec![GeminiPart { text: "Say OK" }],
            }],
            generation_config: GenerationConfig {
                temperature: 0.3,
                max_output_tokens: 10,
                response_mime_type: None,
            },
        };

        let response = self
            .client
            .post(&self.api_url())
            .header("Content-Type", "application/json")
            .json(&request)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| ProviderError::Network {
                message: e.to_string(),
            })?;

        let status = response.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
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
        "Google (Gemini)"
    }

    fn model(&self) -> &str {
        &self.model
    }
}
