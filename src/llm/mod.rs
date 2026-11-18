//! Language-model access: the [`Llm`] trait and its OpenRouter implementation.

mod client;

pub use client::OpenRouterClient;

use std::future::Future;

use thiserror::Error;

/// Tokens (and cost, when the provider reports it) for one call.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// In US dollars, as reported by OpenRouter.
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LlmError {
    /// HTTP 402: out of credits. The worker pauses and tells Pavel once.
    #[error("quota exhausted")]
    QuotaExhausted,
    /// HTTP 429: retry later with backoff.
    #[error("rate limited")]
    RateLimited,
    /// The call succeeded but the content is not the JSON we asked for.
    #[error("bad model response: {0}")]
    BadResponse(String),
    #[error("HTTP {0}: {1}")]
    Http(u16, String),
    #[error("network error: {0}")]
    Network(String),
}

/// A chat model that answers in JSON.
///
/// Used through generics (`L: Llm`), not `dyn Llm`: traits whose methods
/// return `impl Future` are not `dyn`-compatible. `+ Send` lets the call run
/// inside `tokio::spawn`.
pub trait Llm: Send + Sync {
    fn chat_json(
        &self,
        system: &str,
        user: &str,
    ) -> impl Future<Output = Result<(serde_json::Value, Usage), LlmError>> + Send;

    /// Model name, recorded with each call.
    fn model(&self) -> &str;
}
