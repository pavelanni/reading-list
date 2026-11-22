//! Stories, the worker's queue bookkeeping, and storing model results.

use anyhow::{Context, Result};

use super::{Db, FetchStatus, ProcessStatus, StoryKind, StoryStatus};
use crate::llm::prompts::{Assignment, CompareResult, StoryBrief};
use crate::status;

/// Characters of a story summary shown in the assign prompt.
const BRIEF_CHARS: i64 = 200;

/// An article claimed by the worker.
#[derive(Debug, Clone, PartialEq)]
pub struct ArticleRow {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub extracted_text: Option<String>,
    /// The note as it was when the article was claimed.
    pub note: Option<String>,
    pub share_text: Option<String>,
    pub fetch_status: FetchStatus,
    pub attempts: i64,
    /// Transient failures so far (timeouts, 5xx, rate limits).
    pub transient_failures: i64,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoryRow {
    pub id: i64,
    pub title: String,
    pub kind: StoryKind,
    pub summary: String,
    pub status: StoryStatus,
}

impl Db {
    /// Takes the oldest pending article and marks it `processing`. One
    /// `UPDATE … RETURNING` statement, so two workers can never claim the
    /// same row.
    pub async fn claim_next_pending(&self) -> Result<Option<ArticleRow>> {
        let row = sqlx::query!(
            r#"UPDATE article SET process_status = 'processing'
               WHERE id = (SELECT id FROM article
                           WHERE process_status = 'pending'
                             AND (retry_at IS NULL OR retry_at <= datetime('now'))
                           ORDER BY id LIMIT 1)
               RETURNING id AS "id!", url AS "url!", title, extracted_text, note,
                         share_text, fetch_status AS "fetch_status!: FetchStatus",
                         attempts AS "attempts!",
                         transient_failures AS "transient_failures!""#
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let tags = self.article_tags(row.id).await?;
        Ok(Some(ArticleRow {
            id: row.id,
            url: row.url,
            title: row.title,
            extracted_text: row.extracted_text,
            note: row.note,
            share_text: row.share_text,
            fetch_status: row.fetch_status,
            attempts: row.attempts,
            transient_failures: row.transient_failures,
            tags,
        }))
    }

    /// Tag names of an article, sorted.
    pub async fn article_tags(&self, article_id: i64) -> Result<Vec<String>> {
        let tags = sqlx::query_scalar!(
            "SELECT t.name FROM tag t JOIN article_tag at ON at.tag_id = t.id
             WHERE at.article_id = ? ORDER BY t.name",
            article_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(tags)
    }

    /// Puts articles left `processing` by a crash or shutdown back in the
    /// queue. Called once at startup; returns how many were reset.
    pub async fn reset_stuck_processing(&self) -> Result<u64> {
        let done = sqlx::query!(
            "UPDATE article SET process_status = 'pending' WHERE process_status = 'processing'"
        )
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected())
    }

    /// Stores a downloaded page. Keeps the old title if the page has none.
    pub async fn store_fetched(
        &self,
        article_id: i64,
        title: Option<&str>,
        text: &str,
    ) -> Result<()> {
        sqlx::query!(
            "UPDATE article SET fetch_status = 'ok', title = COALESCE(?, title),
                                extracted_text = ?
             WHERE id = ?",
            title,
            text,
            article_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Records that the page could not be downloaded. Processing goes on
    /// without the text.
    pub async fn store_fetch_failure(&self, article_id: i64, reason: &str) -> Result<()> {
        sqlx::query!(
            "UPDATE article SET fetch_status = 'failed', fetch_error = ? WHERE id = ?",
            reason,
            article_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Returns a claimed article to the queue without counting an attempt
    /// (quota, rate limit, network trouble).
    pub async fn release(&self, article_id: i64) -> Result<()> {
        sqlx::query!(
            "UPDATE article SET process_status = 'pending'
             WHERE id = ? AND process_status = 'processing'",
            article_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Counts a failed attempt. `attempts` includes this one; the new
    /// status comes from [`status::after_failed_attempt`].
    pub async fn record_failure(
        &self,
        article_id: i64,
        attempts: i64,
        error: &str,
    ) -> Result<ProcessStatus> {
        let next = status::after_failed_attempt(attempts);
        sqlx::query!(
            "UPDATE article SET attempts = ?, error = ?, process_status = ? WHERE id = ?",
            attempts,
            error,
            next,
            article_id
        )
        .execute(&self.pool)
        .await?;
        Ok(next)
    }

    /// Counts a transient failure. `failures` includes this one. Below
    /// the cap the article waits [`status::transient_backoff`] before it can
    /// be claimed again; at the cap it is marked failed.
    pub async fn record_transient(
        &self,
        article_id: i64,
        failures: i64,
        error: &str,
    ) -> Result<ProcessStatus> {
        let next = status::after_transient_failure(failures);
        // An SQLite date modifier such as "+120 seconds".
        let wait = format!("+{} seconds", status::transient_backoff(failures).as_secs());
        sqlx::query!(
            "UPDATE article SET transient_failures = ?, error = ?, process_status = ?,
                                retry_at = datetime('now', ?)
             WHERE id = ?",
            failures,
            error,
            next,
            wait,
            article_id
        )
        .execute(&self.pool)
        .await?;
        Ok(next)
    }

    /// Stories updated in the last 30 days (any status), newest first.
    pub async fn recent_stories(&self, limit: i64) -> Result<Vec<StoryBrief>> {
        let rows = sqlx::query!(
            r#"SELECT id AS "id!", title, kind AS "kind: StoryKind",
                      status AS "status: StoryStatus",
                      substr(summary, 1, ?) AS "one_line!: String"
               FROM story
               WHERE updated_at >= datetime('now', '-30 days')
               ORDER BY updated_at DESC, id DESC
               LIMIT ?"#,
            BRIEF_CHARS,
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| StoryBrief {
                id: r.id,
                title: r.title,
                kind: r.kind,
                status: r.status,
                one_line: r.one_line,
            })
            .collect())
    }

    pub async fn story(&self, id: i64) -> Result<Option<StoryRow>> {
        let row = sqlx::query_as!(
            StoryRow,
            r#"SELECT id AS "id!", title, kind AS "kind: StoryKind", summary,
                      status AS "status: StoryStatus"
               FROM story WHERE id = ?"#,
            id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Stores the result of processing `article` in one transaction and
    /// returns the story ID. Creates the story for [`Assignment::New`].
    ///
    /// This is the single "article processed" point: later sinks
    /// (Hindsight, alerts) hook in here.
    ///
    /// If the note changed while the article was being processed, the
    /// article goes back to `pending` so the new note is applied.
    pub async fn apply_comparison(
        &self,
        article: &ArticleRow,
        target: &Assignment,
        result: &CompareResult,
    ) -> Result<i64> {
        let mut tx = self.pool.begin().await?;

        // Write first: SQLite then takes the write lock at the start of the
        // transaction instead of failing to upgrade a read lock later.
        sqlx::query!(
            "DELETE FROM story_addition WHERE article_id = ?",
            article.id
        )
        .execute(&mut *tx)
        .await?;

        let story_id = match target {
            Assignment::Existing(id) => *id,
            Assignment::New(new) => {
                sqlx::query_scalar!(
                    "INSERT INTO story (title, kind) VALUES (?, ?) RETURNING id",
                    new.title,
                    new.kind
                )
                .fetch_one(&mut *tx)
                .await?
            }
        };

        let adds: Vec<&str> = result
            .adds
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        let added_text = adds.join("\n");
        let added_info = !adds.is_empty();

        sqlx::query!(
            "INSERT INTO story_addition (story_id, article_id, added_text) VALUES (?, ?, ?)",
            story_id,
            article.id,
            added_text
        )
        .execute(&mut *tx)
        .await?;

        let current = sqlx::query_scalar!(
            r#"SELECT status AS "status: StoryStatus" FROM story WHERE id = ?"#,
            story_id
        )
        .fetch_optional(&mut *tx)
        .await?
        .with_context(|| format!("story {story_id} not found"))?;
        let next = status::story_status_after_merge(current, added_info);

        sqlx::query!(
            "UPDATE story SET
                 summary = ?, status = ?, updated_at = CURRENT_TIMESTAMP,
                 last_new_info_at = CASE WHEN ? THEN CURRENT_TIMESTAMP
                                         ELSE last_new_info_at END
             WHERE id = ?",
            result.updated_summary,
            next,
            added_info,
            story_id
        )
        .execute(&mut *tx)
        .await?;

        // `IS` compares NULLs as equal, unlike `=`.
        sqlx::query!(
            "UPDATE article SET
                 story_id = ?, one_line_summary = ?,
                 error = CASE WHEN fetch_status = 'failed' THEN error END,
                 process_status = CASE WHEN note IS ? THEN 'done' ELSE 'pending' END
             WHERE id = ?",
            story_id,
            result.one_line_summary,
            article.note,
            article.id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(story_id)
    }
}
