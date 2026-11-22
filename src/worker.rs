//! Background worker: fetch, assign to a story, compare, store.

use std::time::Duration;

use anyhow::Result;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::bot::Notifier;
use crate::db::{ArticleRow, Db, FetchStatus, ProcessStatus};
use crate::fetch::fetch_article;
use crate::llm::prompts::{
    ArticleInput, AssignResult, Assignment, CompareResult, MAX_RECENT_STORIES, assign_prompt,
    compare_prompt,
};
use crate::llm::{Llm, LlmError};
use crate::net::WebClient;
use crate::status::{self, Disposition};

pub use crate::status::{Pause, QUOTA_MESSAGE};

/// What one call to [`process_one`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Processed {
    /// Nothing was pending.
    Idle,
    /// Article stored in a story.
    Done(i64),
    /// Article back in the queue (temporary error or attempts left).
    Retry(i64),
    /// Article gave up after too many bad answers.
    Failed(i64),
    /// Summaries paused for every article; this one is back in the queue.
    Paused(Pause),
}

/// How long the worker waits in each situation. Tests use short delays.
#[derive(Debug, Clone, Copy)]
pub struct Delays {
    pub idle: Duration,
    pub retry: Duration,
    pub quota: Duration,
}

impl Default for Delays {
    fn default() -> Self {
        Delays {
            idle: Duration::from_secs(30),
            retry: Duration::from_secs(60),
            quota: Duration::from_secs(15 * 60),
        }
    }
}

/// Processes articles until `shutdown` is cancelled.
pub async fn run<L: Llm, N: Notifier>(
    db: Db,
    http: WebClient,
    llm: L,
    notifier: N,
    delays: Delays,
    shutdown: CancellationToken,
) {
    // The pause already reported, so each pause is announced once. Any
    // other outcome ends the pause; a new or different one is announced.
    let mut reported: Option<Pause> = None;
    loop {
        // A cancelled `process_one` drops its open transaction, which rolls
        // back; the article stays `processing` until the next startup.
        let outcome = tokio::select! {
            _ = shutdown.cancelled() => return,
            outcome = process_one(&db, &http, &llm) => outcome,
        };
        if !matches!(outcome, Ok(Processed::Paused(_))) {
            reported = None;
        }
        let wait = match outcome {
            Ok(Processed::Done(id)) => {
                tracing::info!(article_id = id, "article processed");
                None
            }
            Ok(Processed::Failed(id)) => {
                tracing::warn!(article_id = id, "article failed");
                None
            }
            Ok(Processed::Idle) => Some(delays.idle),
            Ok(Processed::Retry(id)) => {
                tracing::info!(article_id = id, "article will be retried");
                Some(delays.retry)
            }
            Ok(Processed::Paused(pause)) => {
                if reported != Some(pause) {
                    notifier.notify(&pause.message()).await;
                    reported = Some(pause);
                }
                Some(delays.quota)
            }
            Err(err) => {
                tracing::error!("worker error: {err:#}");
                Some(delays.idle)
            }
        };
        if let Some(wait) = wait {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(wait) => {}
            }
        }
    }
}

/// Claims the next pending article and runs it through the pipeline.
pub async fn process_one<L: Llm>(db: &Db, http: &WebClient, llm: &L) -> Result<Processed> {
    let Some(mut article) = db.claim_next_pending().await? else {
        return Ok(Processed::Idle);
    };
    let id = article.id;
    let failures = article.transient_failures + 1;
    match process_claimed(db, http, llm, &mut article).await {
        Ok(done) => Ok(done),
        Err(err) => {
            // A database error. Count it as transient, so one that repeats
            // for this article gives up at the cap instead of looping (and
            // re-billing model calls) forever.
            if let Err(record_err) = db
                .record_transient(id, failures, &format!("database error: {err:#}"))
                .await
            {
                tracing::error!(article_id = id, "recording failure failed: {record_err:#}");
            }
            Err(err)
        }
    }
}

async fn process_claimed<L: Llm>(
    db: &Db,
    http: &WebClient,
    llm: &L,
    article: &mut ArticleRow,
) -> Result<Processed> {
    let id = article.id;
    if article.fetch_status == FetchStatus::Pending
        && fetch(db, http, article).await? == Fetch::Deferred
    {
        // No text yet: try the page again later instead of summarizing
        // from the URL alone.
        return Ok(Processed::Retry(id));
    }

    let err = match assign_and_compare(db, llm, article).await? {
        Ok((target, result)) => {
            db.apply_comparison(article, &target, &result).await?;
            return Ok(Processed::Done(id));
        }
        Err(err) => err,
    };
    match status::classify_llm_error(&err) {
        Disposition::Pause(pause) => {
            tracing::warn!(article_id = id, "summaries paused: {err}");
            db.release(id).await?;
            Ok(Processed::Paused(pause))
        }
        Disposition::Transient => {
            tracing::warn!(article_id = id, "temporary model error: {err}");
            let status = db
                .record_transient(id, article.transient_failures + 1, &err.to_string())
                .await?;
            Ok(outcome(id, status))
        }
        Disposition::Attempt => {
            let status = db
                .record_failure(id, article.attempts + 1, &err.to_string())
                .await?;
            Ok(outcome(id, status))
        }
    }
}

/// `Retry` while the article is still queued, `Failed` once it gave up.
fn outcome(id: i64, status: ProcessStatus) -> Processed {
    match status {
        ProcessStatus::Failed => Processed::Failed(id),
        _ => Processed::Retry(id),
    }
}

/// Whether the pipeline can go on after [`fetch`].
#[derive(Debug, PartialEq, Eq)]
enum Fetch {
    /// Text stored, or a permanent failure stored: summarize now.
    Ready,
    /// A transient failure: the article waits and is fetched again.
    Deferred,
}

/// Downloads the page and stores the text, or the reason it failed.
async fn fetch(db: &Db, http: &WebClient, article: &mut ArticleRow) -> Result<Fetch> {
    let url = match Url::parse(&article.url) {
        Ok(url) => url,
        Err(e) => {
            db.store_fetch_failure(article.id, &format!("invalid URL: {e}"))
                .await?;
            article.fetch_status = FetchStatus::Failed;
            return Ok(Fetch::Ready);
        }
    };
    match fetch_article(http, &url).await {
        Ok(page) => {
            db.store_fetched(article.id, page.title.as_deref(), &page.text)
                .await?;
            if page.title.is_some() {
                article.title = page.title;
            }
            article.extracted_text = Some(page.text);
            article.fetch_status = FetchStatus::Ok;
            Ok(Fetch::Ready)
        }
        Err(err)
            if status::is_transient_fetch_error(&err)
                && status::after_transient_failure(article.transient_failures + 1)
                    == ProcessStatus::Pending =>
        {
            tracing::info!(article_id = article.id, "fetch failed, will retry: {err}");
            db.record_transient(article.id, article.transient_failures + 1, &err.to_string())
                .await?;
            Ok(Fetch::Deferred)
        }
        Err(err) => {
            tracing::info!(article_id = article.id, "fetch failed: {err}");
            db.store_fetch_failure(article.id, &err.to_string()).await?;
            article.fetch_status = FetchStatus::Failed;
            Ok(Fetch::Ready)
        }
    }
}

/// The two model calls. The outer `Result` is for database errors, the
/// inner one for model errors, which the caller handles case by case.
async fn assign_and_compare<L: Llm>(
    db: &Db,
    llm: &L,
    article: &ArticleRow,
) -> Result<Result<(Assignment, CompareResult), LlmError>> {
    let input = ArticleInput {
        title: article.title.as_deref(),
        url: &article.url,
        text: article.extracted_text.as_deref(),
        note: article.note.as_deref(),
        share_text: article.share_text.as_deref(),
        tags: &article.tags,
    };

    let recent = db.recent_stories(MAX_RECENT_STORIES as i64).await?;
    let (system, user) = assign_prompt(&input, &recent);
    let value = match llm.chat_json(&system, &user).await {
        Ok((value, usage)) => {
            db.record_model_call(article.id, "assign", llm.model(), &usage)
                .await?;
            value
        }
        Err(err) => return Ok(Err(err)),
    };
    let target = match AssignResult::parse(value) {
        Ok(target) => target,
        Err(err) => return Ok(Err(err)),
    };

    let (title, kind, summary) = match &target {
        Assignment::Existing(story_id) => match db.story(*story_id).await? {
            Some(s) => (s.title, s.kind, s.summary),
            None => {
                return Ok(Err(LlmError::BadResponse(format!(
                    "unknown story_id {story_id}"
                ))));
            }
        },
        Assignment::New(new) => (new.title.clone(), new.kind, new.summary.clone()),
    };

    let (system, user) = compare_prompt(&input, &title, kind, &summary);
    let value = match llm.chat_json(&system, &user).await {
        Ok((value, usage)) => {
            db.record_model_call(article.id, "compare", llm.model(), &usage)
                .await?;
            value
        }
        Err(err) => return Ok(Err(err)),
    };
    Ok(CompareResult::parse(value).map(|result| (target, result)))
}
