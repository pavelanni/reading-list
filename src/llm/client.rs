//! OpenRouter through its OpenAI-compatible chat completions API.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{Llm, LlmError, Usage};

/// Generous: cheap models on busy providers can take a while.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Error bodies can be HTML pages; keep stored messages short.
const MAX_ERROR_CHARS: usize = 300;

/// No `#[derive(Debug)]`: the struct holds the API key, and a derived
/// `Debug` would print it in any `{:?}` log line.
pub struct OpenRouterClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl OpenRouterClient {
    /// `base_url` is e.g. `https://openrouter.ai/api/v1`. Pass the process-wide
    /// `reqwest::Client` so all calls share one connection pool.
    pub fn new(http: reqwest::Client, base_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            http,
            endpoint: format!("{}/chat/completions", base_url.trim_end_matches('/')),
            api_key: api_key.to_owned(),
            model: model.to_owned(),
        }
    }
}

// Request body. Fields borrow (`&'a str`) instead of owning: serializing
// needs only a view of the strings, so nothing is copied per call.
#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [Message<'a>; 2],
    response_format: ResponseFormat,
    usage: UsageRequest,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")] // `type` is a Rust keyword
    kind: &'static str,
}

/// Asks OpenRouter to include the cost in `usage`.
#[derive(Serialize)]
struct UsageRequest {
    include: bool,
}

// Response body: only the fields we read. Serde ignores the rest.
#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    usage: Option<ApiUsage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
}

/// Every field is optional: a missing count must not reject an answer
/// that was already billed. `#[serde(default)]` fills in 0.
#[derive(Deserialize)]
struct ApiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    cost: Option<f64>,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: String,
}

impl Llm for OpenRouterClient {
    async fn chat_json(
        &self,
        system: &str,
        user: &str,
    ) -> Result<(serde_json::Value, Usage), LlmError> {
        let request = ChatRequest {
            model: &self.model,
            messages: [
                Message {
                    role: "system",
                    content: system,
                },
                Message {
                    role: "user",
                    content: user,
                },
            ],
            response_format: ResponseFormat {
                kind: "json_object",
            },
            usage: UsageRequest { include: true },
        };
        let resp = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .timeout(CALL_TIMEOUT)
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;

        let status = resp.status();
        let body = resp.text().await.map_err(network_error)?;
        match status.as_u16() {
            200..=299 => {}
            402 => return Err(LlmError::QuotaExhausted),
            429 => return Err(LlmError::RateLimited),
            code => return Err(LlmError::Http(code, error_message(&body))),
        }

        let parsed: ChatResponse = serde_json::from_str(&body)
            .map_err(|e| LlmError::BadResponse(format!("unexpected response shape: {e}")))?;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .ok_or_else(|| LlmError::BadResponse("no message content".into()))?;
        let value = serde_json::from_str(strip_code_fence(&content))
            .map_err(|e| LlmError::BadResponse(format!("content is not JSON: {e}")))?;

        let usage = parsed.usage.map_or_else(Usage::default, |u| Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            cost: u.cost,
        });
        Ok((value, usage))
    }

    fn model(&self) -> &str {
        &self.model
    }
}

fn network_error(err: reqwest::Error) -> LlmError {
    if err.is_timeout() {
        LlmError::Network("timed out".into())
    } else {
        LlmError::Network(err.to_string())
    }
}

/// OpenRouter's `{"error": {"message": …}}`, or the start of the raw body.
fn error_message(body: &str) -> String {
    match serde_json::from_str::<ErrorBody>(body) {
        Ok(parsed) => parsed.error.message,
        Err(_) => body.chars().take(MAX_ERROR_CHARS).collect(),
    }
}

/// Some models wrap JSON in a Markdown fence even in JSON mode:
/// "```json\n{…}\n```". Return the inside; leave anything else unchanged.
fn strip_code_fence(content: &str) -> &str {
    let trimmed = content.trim();
    let Some(inner) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let inner = inner.strip_prefix("json").unwrap_or(inner);
    inner.strip_suffix("```").unwrap_or(inner).trim()
}
