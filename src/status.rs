//! Status rules: pure decisions the worker and the database code apply.
//! No I/O here, so every rule is a plain unit test.

use crate::db::ProcessStatus::Failed;
use crate::db::ProcessStatus::Pending;
use crate::db::{ProcessStatus, StoryStatus};

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
}
