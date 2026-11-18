//! Downloading a page and extracting its readable text.

use std::time::Duration;

use dom_smoothie::{Readability, TextMode};
use reqwest::header::{CONTENT_TYPE, USER_AGENT};
use thiserror::Error;
use url::Url;

/// Largest body we download. Bigger pages are rejected, not truncated:
/// a cut-off HTML document extracts badly.
pub const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
/// Extracted text shorter than this is a login wall or an error page.
const MIN_TEXT_CHARS: usize = 200;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// Many sites serve bots a stripped or blocked page.
const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";

/// Readable content of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub title: Option<String>,
    pub text: String,
}

/// Why a page could not be used. `Display` is stored as the article's
/// `error`, so messages are short and human-readable.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FetchError {
    #[error("HTTP {0}")]
    Http(u16),
    #[error("not an HTML page ({0})")]
    NotHtml(String),
    #[error("page larger than {} MB", MAX_BODY_BYTES / (1024 * 1024))]
    TooLarge,
    #[error("timed out")]
    Timeout,
    #[error("network error: {0}")]
    Network(String),
    #[error("no readable text")]
    Empty,
}

impl From<reqwest::Error> for FetchError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            return FetchError::Timeout;
        }
        // reqwest's top-level message is generic ("error sending request");
        // the useful part ("connection refused", "dns error") is in the
        // source chain, so join the whole chain.
        // `successors` hands the closure `&&dyn Error`; deref once so the
        // borrow comes from `err` itself, not from the short-lived outer `&`.
        let chain: Vec<String> =
            std::iter::successors(Some(&err as &dyn std::error::Error), |e| (*e).source())
                .map(ToString::to_string)
                .collect();
        FetchError::Network(chain.join(": "))
    }
}

/// Downloads `url` and extracts title and readable text.
pub async fn fetch_article(client: &reqwest::Client, url: &Url) -> Result<Fetched, FetchError> {
    // `?` on a reqwest::Error converts it through the `From` impl above.
    let mut resp = client
        .get(url.clone())
        .header(USER_AGENT, BROWSER_USER_AGENT)
        .timeout(FETCH_TIMEOUT) // covers connect, headers and the whole body
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        return Err(FetchError::Http(status.as_u16()));
    }
    // No Content-Type at all: assume HTML and let extraction decide.
    if let Some(mime) = mime_type(&resp)
        && mime != "text/html"
        && mime != "application/xhtml+xml"
    {
        return Err(FetchError::NotHtml(mime));
    }

    // Read chunk by chunk and stop at the cap. `resp.bytes()` would buffer
    // whatever the server sends, and Content-Length can be missing or wrong.
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(FetchError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }

    // Most pages are UTF-8; for the rest, replacement characters are fine
    // for a summary.
    let html = String::from_utf8_lossy(&body).into_owned();
    let base = url.to_string();
    // Parsing a large page is CPU work that would stall every other task on
    // this runtime thread, so it runs on Tokio's blocking pool.
    tokio::task::spawn_blocking(move || extract(&html, &base))
        .await
        .unwrap_or_else(|join_err| {
            tracing::warn!(%join_err, "extraction panicked");
            Err(FetchError::Empty)
        })
}

/// The `type/subtype` part of Content-Type, lowercased, without parameters.
fn mime_type(resp: &reqwest::Response) -> Option<String> {
    let value = resp.headers().get(CONTENT_TYPE)?.to_str().ok()?;
    let mime = value.split(';').next().unwrap_or_default().trim();
    Some(mime.to_ascii_lowercase())
}

/// Readability extraction. Synchronous and CPU-bound.
fn extract(html: &str, url: &str) -> Result<Fetched, FetchError> {
    let cfg = dom_smoothie::Config {
        // Keeps paragraph breaks, which the model reads better than one blob.
        text_mode: TextMode::Formatted,
        ..Default::default()
    };
    let mut readability =
        Readability::new(html, Some(url), Some(cfg)).map_err(|_| FetchError::Empty)?;
    let article = readability.parse().map_err(|_| FetchError::Empty)?;

    let text = article.text_content.trim().to_owned();
    // Count characters, not bytes: Cyrillic letters are 2 bytes in UTF-8.
    if text.chars().count() < MIN_TEXT_CHARS {
        return Err(FetchError::Empty);
    }
    let title = Some(article.title.trim().to_owned()).filter(|t| !t.is_empty());
    Ok(Fetched { title, text })
}
