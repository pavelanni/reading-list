//! MCP server: the `crate::query` functions as tools over streamable
//! HTTP at `/mcp`, behind a bearer-token check.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::schemars::JsonSchema;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::db::{Db, StoryKind, StoryStatus};
use crate::query::{self, DEFAULT_SEARCH_LIMIT, ListFilter};

/// Shorter tokens are refused at startup: the token is the only lock.
pub const MIN_TOKEN_LEN: usize = 16;

/// `list_stories` looks back at most this many days (100 years).
const MAX_SINCE_DAYS: u32 = 36_500;

/// Story status filter; `any` matches every status.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "lowercase")]
enum StatusFilter {
    #[default]
    Unread,
    Read,
    Dropped,
    Any,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "lowercase")]
enum KindParam {
    News,
    Idea,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "lowercase")]
enum StatusParam {
    Unread,
    Read,
    Dropped,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct ListParams {
    /// unread (default), read, dropped, or any.
    #[serde(default)]
    status: StatusFilter,
    /// Only stories updated in the last N days (0 = today). Default 7.
    since_days: Option<u32>,
    /// Only stories with an article carrying this tag, e.g. "diy".
    tag: Option<String>,
    /// news or idea (project ideas).
    kind: Option<KindParam>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct IdParam {
    /// The stable ID from an earlier tool result, e.g. 3 from `[id 3]`.
    id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct SearchParams {
    /// Words to look for; all must appear, in any order.
    query: String,
    /// Maximum hits per kind (stories, articles). Default 20, max 100.
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct SetStatusParams {
    /// Story ID.
    id: i64,
    /// unread, read, or dropped.
    status: StatusParam,
}

impl From<KindParam> for StoryKind {
    fn from(k: KindParam) -> Self {
        match k {
            KindParam::News => StoryKind::News,
            KindParam::Idea => StoryKind::Idea,
        }
    }
}

impl From<StatusParam> for StoryStatus {
    fn from(s: StatusParam) -> Self {
        match s {
            StatusParam::Unread => StoryStatus::Unread,
            StatusParam::Read => StoryStatus::Read,
            StatusParam::Dropped => StoryStatus::Dropped,
        }
    }
}

impl ListParams {
    fn into_filter(self) -> ListFilter {
        let status = match self.status {
            StatusFilter::Unread => Some(StoryStatus::Unread),
            StatusFilter::Read => Some(StoryStatus::Read),
            StatusFilter::Dropped => Some(StoryStatus::Dropped),
            StatusFilter::Any => None,
        };
        ListFilter {
            status,
            since_days: self
                .since_days
                .unwrap_or(ListFilter::default().since_days)
                .min(MAX_SINCE_DAYS),
            tag: self.tag.filter(|t| !t.trim().is_empty()),
            kind: self.kind.map(Into::into),
        }
    }
}

fn text(s: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(s)])
}

/// Successful results become pretty JSON; failures become a tool error
/// the agent can read (not a protocol error).
fn json_result<T: Serialize>(r: Result<T>) -> Result<CallToolResult, ErrorData> {
    match r.and_then(|v| serde_json::to_string_pretty(&v).context("encoding result")) {
        Ok(json) => Ok(text(json)),
        Err(e) => Ok(error_result(e)),
    }
}

fn error_result(e: anyhow::Error) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!("{e:#}"))])
}

/// One instance per MCP session; cloning shares the pool.
#[derive(Clone)]
pub struct ReadingListServer {
    db: Db,
}

#[tool_router]
impl ReadingListServer {
    pub fn new(db: Db) -> Self {
        ReadingListServer { db }
    }

    /// Lists stories. Each line starts with the story's
    /// stable ID in square brackets. Refer to stories
    /// by their ID when showing them to the user.
    /// Defaults: unread, last 7 days.
    /// Use status "any" with kind/tag for questions like
    /// "what DIY ideas did I save?".
    #[tool]
    async fn list_stories(
        &self,
        Parameters(p): Parameters<ListParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match query::list_stories(&self.db, &p.into_filter()).await {
            Ok(stories) => Ok(text(query::format_story_list(&stories))),
            Err(e) => Ok(error_result(e)),
        }
    }

    /// One story by ID: summary, and for each article its URL, what it
    /// added, and is_new (added since the story was last marked read).
    #[tool]
    async fn get_story(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, ErrorData> {
        json_result(query::get_story(&self.db, p.id).await)
    }

    /// One article by ID: full extracted text, the reader's note and share
    /// text, tags, statuses and any error.
    #[tool]
    async fn get_article(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, ErrorData> {
        json_result(query::get_article(&self.db, p.id).await)
    }

    /// Keyword search over story titles and summaries and article titles
    /// and text; every word must appear, in any order. Returns story and
    /// article IDs with snippets; the match is in [brackets].
    #[tool]
    async fn search(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let limit = p.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
        json_result(query::search(&self.db, &p.query, limit).await)
    }

    /// Marks a story unread, read, or dropped (by story ID). A read story
    /// becomes unread again when a new article adds information; a dropped
    /// story stays dropped.
    #[tool]
    async fn set_story_status(
        &self,
        Parameters(p): Parameters<SetStatusParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let status: StoryStatus = p.status.into();
        let r = query::set_story_status(&self.db, p.id, status).await;
        match r {
            Ok(()) => Ok(text(format!("Story {} is now {}.", p.id, status.as_str()))),
            Err(e) => Ok(error_result(e)),
        }
    }

    /// Articles whose page could not be fetched or whose processing failed,
    /// with the reason. Use reprocess_article to retry one.
    #[tool]
    async fn list_failed(&self) -> Result<CallToolResult, ErrorData> {
        json_result(query::list_failed(&self.db).await)
    }

    /// Queues one article (by article ID) to be fetched and processed again
    /// with a fresh retry budget.
    #[tool]
    async fn reprocess_article(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, ErrorData> {
        match query::reprocess_article(&self.db, p.id).await {
            Ok(()) => Ok(text(format!("Article {} queued for reprocessing.", p.id))),
            Err(e) => Ok(error_result(e)),
        }
    }
}

// The instructions must be a string literal for the macro.
#[tool_handler(
    name = "reading-list",
    instructions = "Reading list: links the reader saved from Telegram, \
grouped into stories with summaries. Story and article IDs are stable: \
list_stories lines start with one ([id 3]), other tools return them as fields. \
Pass those IDs to tools. \
Typical flow: list_stories, get_story, get_article for full text, then \
set_story_status read or dropped."
)]
impl rmcp::ServerHandler for ReadingListServer {}

/// Rejects requests whose `Authorization` header is not exactly
/// `Bearer <token>`. `ct_eq` takes the same time wherever the first wrong
/// byte is, so timing does not reveal how much of a guess was right.
async fn require_bearer(State(expected): State<Arc<str>>, req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .is_some_and(|v| bool::from(v.as_bytes().ct_eq(expected.as_bytes())));
    if ok {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response()
    }
}

/// Serves MCP on `cfg.mcp_listen_addr` until `shutdown` is cancelled.
pub async fn serve(db: Db, cfg: Arc<Config>, shutdown: CancellationToken) -> Result<()> {
    let listener = TcpListener::bind(cfg.mcp_listen_addr)
        .await
        .with_context(|| format!("binding MCP listener on {}", cfg.mcp_listen_addr))?;
    tracing::info!(addr = %cfg.mcp_listen_addr, "MCP server listening");
    serve_on(listener, db, cfg.mcp_bearer_token.clone(), shutdown).await
}

/// Serves MCP on an already bound listener (tests bind port 0).
pub async fn serve_on(
    listener: TcpListener,
    db: Db,
    token: String,
    shutdown: CancellationToken,
) -> Result<()> {
    // Secrets read from files often end with a newline; compare what the
    // length check saw, or every request would get 401.
    let token = token.trim();
    if token.len() < MIN_TOKEN_LEN {
        bail!("MCP_BEARER_TOKEN must be at least {MIN_TOKEN_LEN} characters");
    }
    // rmcp only accepts loopback Host headers by default (DNS-rebinding
    // protection). LAN clients use the server's address, and the bearer
    // token, which a rebinding page cannot know, already guards the
    // endpoint.
    let mut config = StreamableHttpServerConfig::default().disable_allowed_hosts();
    config.cancellation_token = shutdown.child_token();
    let service = StreamableHttpService::new(
        move || Ok(ReadingListServer::new(db.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let expected: Arc<str> = format!("Bearer {token}").into();
    let app = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(expected, require_bearer));
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await
        .context("MCP server")
}
