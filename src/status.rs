//! Status rules: pure decisions the worker and the database code apply.
//! No I/O here, so every rule is a plain unit test.

use crate::db::ProcessStatus::Failed;
use crate::db::ProcessStatus::Pending;
use crate::db::{ProcessStatus, StoryStatus};
use crate::fetch::FetchError;
use crate::llm::LlmError;
use std::time::Duration;

/// Failed attempts (bad model answers) before an article is given up on.
pub const MAX_ATTEMPTS: i64 = 3;

/// Status of an article after a failed attempt. `attempts` already
/// counts the attempt that just failed (1 after the first failure).
/// Below [`MAX_ATTEMPTS`] the article goes back to the queue; at or
/// above it, the article is marked failed.
pub fn after_failed_attempt(attempts: i64) -> ProcessStatus {
    if attempts < MAX_ATTEMPTS {
        Pending
    } else {
        Failed
    }
}

/// Story status after an article was merged into it. `added_info` is
/// true when the article brought something new. A `read` story with new
/// information becomes `unread` again; a `dropped` story stays dropped;
/// otherwise the status does not change.
pub fn story_status_after_merge(current: StoryStatus, added_info: bool) -> StoryStatus {
    match (current, added_info) {
        (StoryStatus::Read, true) => StoryStatus::Unread,
        (StoryStatus::Read, false) | (StoryStatus::Unread, _) | (StoryStatus::Dropped, _) => {
            current
        }
    }
}

pub const QUOTA_MESSAGE: &str = "Summaries paused: OpenRouter quota reached.";

/// Why summaries are paused. These errors are about the setup, not the
/// article, so they never use up an article's attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pause {
    /// HTTP 402: out of credits.
    Quota,
    /// HTTP 401/403/404: API key revoked or wrong, or model name unknown.
    Rejected(u16),
}

impl Pause {
    /// Message for Pavel, sent once per pause.
    pub fn message(self) -> String {
        match self {
            Pause::Quota => QUOTA_MESSAGE.to_owned(),
            Pause::Rejected(code) => format!(
                "Summaries paused: OpenRouter rejected the request (HTTP {code}). \
                 Check the API key and model name."
            ),
        }
    }
}

/// What a model error means for the article it happened on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// About the setup, not the article: pause all summaries, leave the
    /// article's counters alone.
    Pause(Pause),
    /// Likely to pass by itself: retry this article later with backoff,
    /// at most [`MAX_TRANSIENT_FAILURES`] times.
    Transient,
    /// About this article: use one of its [`MAX_ATTEMPTS`].
    Attempt,
}

/// How the worker treats a model error.
pub fn classify_llm_error(err: &LlmError) -> Disposition {
    match err {
        LlmError::QuotaExhausted => Disposition::Pause(Pause::Quota),
        LlmError::Http(code @ (401 | 403 | 404), _) => Disposition::Pause(Pause::Rejected(*code)),
        LlmError::Http(500..=599, _) | LlmError::RateLimited | LlmError::Network(_) => {
            Disposition::Transient
        }
        LlmError::BadResponse(_) | LlmError::Http(..) => Disposition::Attempt, // covers 400, 413, 422: the request for this article is bad
    }
}

/// True when fetching again later may succeed. Permanent failures are
/// stored and the article is summarized from its URL, title and note.
pub fn is_transient_fetch_error(err: &FetchError) -> bool {
    matches!(
        err,
        FetchError::Http(429 | 408)
            | FetchError::Http(500..=599)
            | FetchError::Timeout
            | FetchError::Network(_)
    )
}

/// Transient failures before an article is given up on. With
/// [`transient_backoff`] that is about two hours of retrying (1+2+4+…+60 min).
pub const MAX_TRANSIENT_FAILURES: i64 = 8;

/// Status after a transient failure; `failures` counts this one.
pub fn after_transient_failure(failures: i64) -> ProcessStatus {
    if failures < MAX_TRANSIENT_FAILURES {
        ProcessStatus::Pending
    } else {
        ProcessStatus::Failed
    }
}

/// Wait before retrying after `failures` transient failures (1 after the
/// first): 1 min, doubling, at most 1 h.
pub fn transient_backoff(failures: i64) -> Duration {
    // Clamp before shifting: a huge exponent would overflow.
    let doublings = failures.clamp(1, 7) - 1;
    Duration::from_secs(60 << doublings).min(Duration::from_secs(60 * 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_below_the_limit_retry() {
        assert_eq!(after_failed_attempt(1), ProcessStatus::Pending);
        assert_eq!(after_failed_attempt(2), ProcessStatus::Pending);
    }

    #[test]
    fn failure_at_or_past_the_limit_gives_up() {
        assert_eq!(after_failed_attempt(3), ProcessStatus::Failed);
        assert_eq!(after_failed_attempt(4), ProcessStatus::Failed);
    }

    #[test]
    fn read_story_with_new_info_resurfaces() {
        assert_eq!(
            story_status_after_merge(StoryStatus::Read, true),
            StoryStatus::Unread
        );
    }

    #[test]
    fn read_story_without_new_info_stays_read() {
        assert_eq!(
            story_status_after_merge(StoryStatus::Read, false),
            StoryStatus::Read
        );
    }

    #[test]
    fn unread_story_stays_unread() {
        assert_eq!(
            story_status_after_merge(StoryStatus::Unread, true),
            StoryStatus::Unread
        );
        assert_eq!(
            story_status_after_merge(StoryStatus::Unread, false),
            StoryStatus::Unread
        );
    }

    #[test]
    fn dropped_story_stays_dropped() {
        assert_eq!(
            story_status_after_merge(StoryStatus::Dropped, true),
            StoryStatus::Dropped
        );
        assert_eq!(
            story_status_after_merge(StoryStatus::Dropped, false),
            StoryStatus::Dropped
        );
    }

    #[test]
    fn setup_errors_pause() {
        assert_eq!(
            classify_llm_error(&LlmError::QuotaExhausted),
            Disposition::Pause(Pause::Quota)
        );
        for code in [401, 403, 404] {
            assert_eq!(
                classify_llm_error(&LlmError::Http(code, String::new())),
                Disposition::Pause(Pause::Rejected(code)),
                "HTTP {code}"
            );
        }
    }

    #[test]
    fn passing_errors_are_transient() {
        let transient = [
            LlmError::RateLimited,
            LlmError::Network("connection reset".into()),
            LlmError::Http(500, String::new()),
            LlmError::Http(502, String::new()),
            LlmError::Http(503, String::new()),
            LlmError::Http(504, String::new()),
        ];
        for err in transient {
            assert_eq!(classify_llm_error(&err), Disposition::Transient, "{err}");
        }
    }

    #[test]
    fn article_errors_use_an_attempt() {
        let article = [
            LlmError::BadResponse("not JSON".into()),
            LlmError::Http(400, "context too long".into()),
            LlmError::Http(413, String::new()),
            LlmError::Http(422, String::new()),
        ];
        for err in article {
            assert_eq!(classify_llm_error(&err), Disposition::Attempt, "{err}");
        }
    }

    #[test]
    fn fetch_errors_that_may_pass_are_transient() {
        let transient = [
            FetchError::Timeout,
            FetchError::Network("dns error".into()),
            FetchError::Http(429),
            FetchError::Http(408),
            FetchError::Http(500),
            FetchError::Http(502),
            FetchError::Http(503),
            FetchError::Http(504),
        ];
        for err in transient {
            assert!(is_transient_fetch_error(&err), "{err}");
        }
    }

    #[test]
    fn permanent_fetch_errors_are_not_retried() {
        let permanent = [
            FetchError::Http(404),
            FetchError::Http(410),
            FetchError::Http(403),
            FetchError::NotHtml("application/pdf".into()),
            FetchError::TooLarge,
            FetchError::Empty,
            FetchError::Blocked(crate::net::Blocked("10.0.0.1".parse().unwrap())),
        ];
        for err in permanent {
            assert!(!is_transient_fetch_error(&err), "{err}");
        }
    }

    #[test]
    fn transient_failures_give_up_at_the_cap() {
        assert_eq!(after_transient_failure(1), ProcessStatus::Pending);
        assert_eq!(
            after_transient_failure(MAX_TRANSIENT_FAILURES - 1),
            ProcessStatus::Pending
        );
        assert_eq!(
            after_transient_failure(MAX_TRANSIENT_FAILURES),
            ProcessStatus::Failed
        );
    }

    #[test]
    fn backoff_doubles_up_to_an_hour() {
        let mins = |n| transient_backoff(n).as_secs() / 60;
        assert_eq!([mins(1), mins(2), mins(3), mins(6)], [1, 2, 4, 32]);
        assert_eq!(mins(7), 60);
        assert_eq!(mins(1_000), 60);
    }
}
