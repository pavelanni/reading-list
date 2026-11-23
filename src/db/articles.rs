//! Saving articles, merging duplicates, and mapping bot replies to articles.

use anyhow::{Result, bail};
use sqlx::{Sqlite, Transaction};

use super::Db;

/// An article as parsed from a Telegram message, before it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewArticle {
    /// Normalized URL; the deduplication key.
    pub url: String,
    /// URL exactly as Pavel sent it.
    pub original_url: String,
    pub tags: Vec<String>,
    pub note: Option<String>,
    pub share_text: Option<String>,
}

/// What `save_article` did, so the bot can word its confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveOutcome {
    pub article_id: i64,
    /// `false` when the URL was already saved and this call merged into it.
    pub created: bool,
    /// Tags newly attached by this call (already-present tags are omitted).
    pub added_tags: Vec<String>,
}

impl Db {
    /// Saves an article, or merges tags and note into the existing row with
    /// the same URL. A merged note re-queues an already processed article.
    pub async fn save_article(&self, new: NewArticle) -> Result<SaveOutcome> {
        let note = clean_note(new.note.as_deref());
        let mut tx = self.pool.begin().await?;

        let existing = sqlx::query_scalar!("SELECT id FROM article WHERE url = ?", new.url)
            .fetch_optional(&mut *tx)
            .await?;

        let (article_id, created) = match existing {
            Some(id) => {
                if let Some(note) = note {
                    append_note(&mut tx, id, note).await?;
                }
                // Keep the first share text we saw; fill it in if it was missing.
                sqlx::query!(
                    "UPDATE article SET share_text = COALESCE(share_text, ?) WHERE id = ?",
                    new.share_text,
                    id
                )
                .execute(&mut *tx)
                .await?;
                (id, false)
            }
            None => {
                let id = sqlx::query_scalar!(
                    "INSERT INTO article (url, original_url, note, share_text)
                     VALUES (?, ?, ?, ?) RETURNING id",
                    new.url,
                    new.original_url,
                    note,
                    new.share_text
                )
                .fetch_one(&mut *tx)
                .await?;
                (id, true)
            }
        };

        let added_tags = attach_tags(&mut tx, article_id, &new.tags).await?;
        tx.commit().await?;
        Ok(SaveOutcome {
            article_id,
            created,
            added_tags,
        })
    }

    /// Adds a note (from a reply to a bot confirmation) and tags to an
    /// existing article, re-queueing it if it was already processed.
    pub async fn add_note(&self, article_id: i64, note: &str, tags: &[String]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let found = sqlx::query_scalar!("SELECT id FROM article WHERE id = ?", article_id)
            .fetch_optional(&mut *tx)
            .await?;
        if found.is_none() {
            bail!("article {article_id} not found");
        }
        if let Some(note) = clean_note(Some(note)) {
            append_note(&mut tx, article_id, note).await?;
        }
        attach_tags(&mut tx, article_id, tags).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Remembers which articles a bot confirmation message refers to.
    pub async fn record_bot_reply(
        &self,
        telegram_message_id: i32,
        article_ids: &[i64],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for id in article_ids {
            sqlx::query!(
                "INSERT OR IGNORE INTO bot_reply (telegram_message_id, article_id) VALUES (?, ?)",
                telegram_message_id,
                id
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Articles referred to by a bot confirmation message; empty if unknown.
    pub async fn articles_for_bot_reply(&self, telegram_message_id: i32) -> Result<Vec<i64>> {
        let ids = sqlx::query_scalar!(
            "SELECT article_id FROM bot_reply WHERE telegram_message_id = ?",
            telegram_message_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(ids)
    }
}

/// Treats a missing or blank note as no note.
fn clean_note(note: Option<&str>) -> Option<&str> {
    note.map(str::trim).filter(|n| !n.is_empty())
}

/// Appends `note` on a new line. If the article was already processed, it
/// goes back to `pending` with a fresh retry budget so the note is applied.
async fn append_note(tx: &mut Transaction<'_, Sqlite>, article_id: i64, note: &str) -> Result<()> {
    sqlx::query!(
        "UPDATE article SET
             note = CASE WHEN note IS NULL OR note = '' THEN ?1 ELSE note || char(10) || ?1 END,
             attempts = CASE WHEN process_status = 'done' THEN 0 ELSE attempts END,
             transient_failures = CASE WHEN process_status = 'done' THEN 0 ELSE transient_failures END,
             retry_at = CASE WHEN process_status = 'done' THEN NULL ELSE retry_at END,
             process_status = CASE WHEN process_status = 'done' THEN 'pending' ELSE process_status END
         WHERE id = ?2",
        note,
        article_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Attaches tags to an article, returning those that were not attached yet.
async fn attach_tags(
    tx: &mut Transaction<'_, Sqlite>,
    article_id: i64,
    tags: &[String],
) -> Result<Vec<String>> {
    let mut added = Vec::new();
    for tag in tags {
        sqlx::query!("INSERT OR IGNORE INTO tag (name) VALUES (?)", tag)
            .execute(&mut **tx)
            .await?;
        let result = sqlx::query!(
            "INSERT OR IGNORE INTO article_tag (article_id, tag_id)
             SELECT ?, id FROM tag WHERE name = ?",
            article_id,
            tag
        )
        .execute(&mut **tx)
        .await?;
        if result.rows_affected() == 1 {
            added.push(tag.clone());
        }
    }
    Ok(added)
}
