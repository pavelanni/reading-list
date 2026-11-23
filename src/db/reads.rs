//! Read and triage queries behind the MCP tools (`crate::query`).

use anyhow::Result;

use super::stories::BRIEF_CHARS;
use super::{Db, FetchStatus, ProcessStatus, StoryKind, StoryStatus};
use crate::query::{ArticleHit, ArticleOut, FailedOut, StoryHit, StorySummaryOut};

/// Separates tags in `group_concat`. Tags are free text after `#`, so a
/// comma could be part of one; the ASCII unit separator cannot.
const TAG_SEP: char = '\u{1f}';

fn split_tags(joined: Option<String>) -> Vec<String> {
    match joined {
        Some(s) if !s.is_empty() => s.split(TAG_SEP).map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}

/// A story's own fields, for `get_story`.
#[derive(Debug, Clone)]
pub struct StoryHeadRow {
    pub id: i64,
    pub title: String,
    pub kind: StoryKind,
    pub status: StoryStatus,
    pub summary: String,
    pub last_read_at: Option<String>,
    pub tags: Vec<String>,
}

/// One article of a story with what it added (`added_text` is `None` when
/// the article has no addition row yet).
#[derive(Debug, Clone)]
pub struct StoryArticleRow {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub saved_at: String,
    pub added_text: Option<String>,
    pub added_at: Option<String>,
}

impl Db {
    /// Stories matching the filters, newest update first. `None` filters
    /// match anything; `tag` must already be normalized (lowercase, no `#`).
    pub async fn list_stories(
        &self,
        status: Option<StoryStatus>,
        since_days: u32,
        kind: Option<StoryKind>,
        tag: Option<&str>,
    ) -> Result<Vec<StorySummaryOut>> {
        let since = format!("-{since_days} days");
        let sep = TAG_SEP.to_string();
        let rows = sqlx::query!(
            r#"SELECT s.id AS "id!", s.title, s.kind AS "kind: StoryKind",
                      s.status AS "status: StoryStatus",
                      substr(s.summary, 1, ?1) AS "one_line!: String",
                      (SELECT count(*) FROM article a WHERE a.story_id = s.id)
                          AS "article_count!: i64",
                      (SELECT group_concat(name, ?2) FROM (
                           SELECT DISTINCT t.name FROM tag t
                           JOIN article_tag at ON at.tag_id = t.id
                           JOIN article a ON a.id = at.article_id
                           WHERE a.story_id = s.id ORDER BY t.name))
                          AS "tags: String"
               FROM story s
               WHERE (?3 IS NULL OR s.status = ?3)
                 AND s.updated_at >= datetime('now', ?4)
                 AND (?5 IS NULL OR s.kind = ?5)
                 AND (?6 IS NULL OR EXISTS (
                      SELECT 1 FROM article a
                      JOIN article_tag at ON at.article_id = a.id
                      JOIN tag t ON t.id = at.tag_id
                      WHERE a.story_id = s.id AND t.name = ?6))
               ORDER BY s.updated_at DESC, s.id DESC"#,
            BRIEF_CHARS,
            sep,
            status,
            since,
            kind,
            tag
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| StorySummaryOut {
                id: r.id,
                title: r.title,
                kind: r.kind,
                status: r.status,
                tags: split_tags(r.tags),
                article_count: r.article_count,
                one_line: r.one_line,
            })
            .collect())
    }

    pub async fn story_head(&self, id: i64) -> Result<Option<StoryHeadRow>> {
        let sep = TAG_SEP.to_string();
        let row = sqlx::query!(
            r#"SELECT s.id AS "id!", s.title, s.kind AS "kind: StoryKind",
                      s.status AS "status: StoryStatus", s.summary, s.last_read_at,
                      (SELECT group_concat(name, ?1) FROM (
                           SELECT DISTINCT t.name FROM tag t
                           JOIN article_tag at ON at.tag_id = t.id
                           JOIN article a ON a.id = at.article_id
                           WHERE a.story_id = s.id ORDER BY t.name))
                          AS "tags: String"
               FROM story s WHERE s.id = ?2"#,
            sep,
            id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| StoryHeadRow {
            id: r.id,
            title: r.title,
            kind: r.kind,
            status: r.status,
            summary: r.summary,
            last_read_at: r.last_read_at,
            tags: split_tags(r.tags),
        }))
    }

    /// Articles of a story in the order they were saved.
    pub async fn story_articles(&self, story_id: i64) -> Result<Vec<StoryArticleRow>> {
        let rows = sqlx::query_as!(
            StoryArticleRow,
            r#"SELECT a.id AS "id!", a.url, a.title, a.saved_at,
                      sa.added_text AS "added_text?", sa.created_at AS "added_at?"
               FROM article a
               LEFT JOIN story_addition sa ON sa.article_id = a.id
               WHERE a.story_id = ?
               ORDER BY a.saved_at, a.id"#,
            story_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn article_detail(&self, id: i64) -> Result<Option<ArticleOut>> {
        let row = sqlx::query!(
            r#"SELECT id AS "id!", url, original_url, title, extracted_text, note,
                      share_text, saved_at, fetch_status AS "fetch_status: FetchStatus",
                      process_status AS "process_status: ProcessStatus",
                      story_id, one_line_summary, error
               FROM article WHERE id = ?"#,
            id
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(r) = row else { return Ok(None) };
        Ok(Some(ArticleOut {
            tags: self.article_tags(r.id).await?,
            id: r.id,
            url: r.url,
            original_url: r.original_url,
            title: r.title,
            text: r.extracted_text,
            note: r.note,
            share_text: r.share_text,
            saved_at: r.saved_at,
            fetch_status: r.fetch_status,
            process_status: r.process_status,
            story_id: r.story_id,
            one_line_summary: r.one_line_summary,
            error: r.error,
        }))
    }

    /// Full-text search over story titles and summaries. `fts_query` is
    /// passed to `MATCH` as is.
    pub async fn search_stories(&self, fts_query: &str, limit: u32) -> Result<Vec<StoryHit>> {
        let rows = sqlx::query_as!(
            StoryHit,
            r#"SELECT s.id AS "id!", s.title,
                      snippet(story_fts, -1, '[', ']', '…', 12) AS "snippet!: String"
               FROM story_fts JOIN story s ON s.id = story_fts.rowid
               WHERE story_fts MATCH ?
               ORDER BY rank LIMIT ?"#,
            fts_query,
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Full-text search over article titles and extracted text.
    pub async fn search_articles(&self, fts_query: &str, limit: u32) -> Result<Vec<ArticleHit>> {
        let rows = sqlx::query_as!(
            ArticleHit,
            r#"SELECT a.id AS "id!", a.story_id, a.url, a.title,
                      snippet(article_fts, -1, '[', ']', '…', 12) AS "snippet!: String"
               FROM article_fts JOIN article a ON a.id = article_fts.rowid
               WHERE article_fts MATCH ?
               ORDER BY rank LIMIT ?"#,
            fts_query,
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Sets a story's status; `read` also records when. Returns false when
    /// the story does not exist.
    pub async fn set_story_status(&self, id: i64, status: StoryStatus) -> Result<bool> {
        let done = sqlx::query!(
            "UPDATE story SET status = ?1,
                 last_read_at = CASE WHEN ?1 = 'read' THEN CURRENT_TIMESTAMP
                                     ELSE last_read_at END
             WHERE id = ?2",
            status,
            id
        )
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Articles whose page could not be fetched or whose processing failed.
    pub async fn failed_articles(&self) -> Result<Vec<FailedOut>> {
        let rows = sqlx::query_as!(
            FailedOut,
            r#"SELECT id AS "id!", url, title, story_id,
                      fetch_status AS "fetch_status: FetchStatus",
                      process_status AS "process_status: ProcessStatus",
                      attempts, error
               FROM article
               WHERE process_status = 'failed' OR fetch_status = 'failed'
               ORDER BY saved_at DESC, id DESC"#
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Queues an article for a fresh fetch and processing run with a new
    /// attempt budget. An article being processed is left alone. Returns
    /// the status afterwards (`Processing` = refused), `None` if unknown.
    pub async fn reprocess_article(&self, id: i64) -> Result<Option<ProcessStatus>> {
        let mut tx = self.pool.begin().await?;
        // A write first, so SQLite takes the write lock up front.
        sqlx::query!(
            "UPDATE article SET fetch_status = 'pending', process_status = 'pending',
                                attempts = 0, error = NULL
             WHERE id = ? AND process_status <> 'processing'",
            id
        )
        .execute(&mut *tx)
        .await?;
        let before = sqlx::query_scalar!(
            r#"SELECT process_status AS "s: ProcessStatus" FROM article WHERE id = ?"#,
            id
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(before)
    }
}
