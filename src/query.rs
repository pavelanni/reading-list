//! Read and triage logic shared by the MCP tools (and later Telegram
//! commands). SQL is in `crate::db`; this module adds defaults,
//! validation, and the rules that are not plain queries.

use anyhow::{Result, bail};
use serde::Serialize;

use crate::db::{Db, FetchStatus, ProcessStatus, StoryArticleRow, StoryKind, StoryStatus};

/// Default and maximum number of search hits per table.
pub const DEFAULT_SEARCH_LIMIT: u32 = 20;
pub const MAX_SEARCH_LIMIT: u32 = 100;

/// Filters for [`list_stories`]. `None` matches anything.
#[derive(Debug, Clone, PartialEq)]
pub struct ListFilter {
    pub status: Option<StoryStatus>,
    pub since_days: u32,
    pub tag: Option<String>,
    pub kind: Option<StoryKind>,
}

impl Default for ListFilter {
    /// Unread stories updated in the last 7 days.
    fn default() -> Self {
        ListFilter {
            status: Some(StoryStatus::Unread),
            since_days: 7,
            tag: None,
            kind: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StorySummaryOut {
    pub id: i64,
    pub title: String,
    pub kind: StoryKind,
    pub status: StoryStatus,
    pub tags: Vec<String>,
    pub article_count: i64,
    pub one_line: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryDetailOut {
    pub id: i64,
    pub title: String,
    pub kind: StoryKind,
    pub status: StoryStatus,
    pub summary: String,
    pub tags: Vec<String>,
    pub last_read_at: Option<String>,
    pub articles: Vec<StoryArticleOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryArticleOut {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub saved_at: String,
    /// What this article added to the story; empty = nothing new.
    pub adds: Vec<String>,
    /// Added something after the story was last read.
    pub is_new: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArticleOut {
    pub id: i64,
    pub url: String,
    pub original_url: String,
    pub title: Option<String>,
    pub text: Option<String>,
    pub note: Option<String>,
    pub share_text: Option<String>,
    pub tags: Vec<String>,
    pub saved_at: String,
    pub fetch_status: FetchStatus,
    pub process_status: ProcessStatus,
    pub story_id: Option<i64>,
    pub one_line_summary: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchOut {
    pub stories: Vec<StoryHit>,
    pub articles: Vec<ArticleHit>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryHit {
    pub id: i64,
    pub title: String,
    /// Matching text with the match in `[brackets]`.
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArticleHit {
    pub id: i64,
    pub story_id: Option<i64>,
    pub url: String,
    pub title: Option<String>,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FailedOut {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub story_id: Option<i64>,
    pub fetch_status: FetchStatus,
    pub process_status: ProcessStatus,
    pub attempts: i64,
    pub error: Option<String>,
}

pub async fn list_stories(db: &Db, filter: &ListFilter) -> Result<Vec<StorySummaryOut>> {
    let tag = filter.tag.as_deref().map(normalize_tag);
    db.list_stories(
        filter.status,
        filter.since_days,
        filter.kind,
        tag.as_deref(),
    )
    .await
}

pub async fn get_story(db: &Db, id: i64) -> Result<StoryDetailOut> {
    let Some(head) = db.story_head(id).await? else {
        bail!("story {id} not found");
    };
    let articles = db
        .story_articles(id)
        .await?
        .into_iter()
        .map(|row| story_article(row, head.last_read_at.as_deref()))
        .collect();
    Ok(StoryDetailOut {
        id: head.id,
        title: head.title,
        kind: head.kind,
        status: head.status,
        summary: head.summary,
        tags: head.tags,
        last_read_at: head.last_read_at,
        articles,
    })
}

pub async fn get_article(db: &Db, id: i64) -> Result<ArticleOut> {
    match db.article_detail(id).await? {
        Some(article) => Ok(article),
        None => bail!("article {id} not found"),
    }
}

/// Keyword search over stories and articles. The query is matched as one
/// phrase, so FTS5 operators in it are plain words, not syntax.
pub async fn search(db: &Db, query: &str, limit: u32) -> Result<SearchOut> {
    if query.trim().is_empty() {
        bail!("search query is empty");
    }
    let phrase = fts_phrase(query);
    let limit = limit.clamp(1, MAX_SEARCH_LIMIT);
    Ok(SearchOut {
        stories: db.search_stories(&phrase, limit).await?,
        articles: db.search_articles(&phrase, limit).await?,
    })
}

pub async fn set_story_status(db: &Db, id: i64, status: StoryStatus) -> Result<()> {
    if !db.set_story_status(id, status).await? {
        bail!("story {id} not found");
    }
    Ok(())
}

pub async fn list_failed(db: &Db) -> Result<Vec<FailedOut>> {
    db.failed_articles().await
}

pub async fn reprocess_article(db: &Db, id: i64) -> Result<()> {
    match db.reprocess_article(id).await? {
        None => bail!("article {id} not found"),
        Some(ProcessStatus::Processing) => {
            bail!("article {id} is being processed right now; try again in a minute")
        }
        Some(_) => Ok(()),
    }
}

/// Tags are stored lowercase without `#`; accept `#DIY` from an agent too.
fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_start_matches('#').to_lowercase()
}

/// Quotes user input as one FTS5 phrase: `"` inside is doubled, so no
/// input can produce a syntax error.
fn fts_phrase(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
}

fn story_article(row: StoryArticleRow, last_read_at: Option<&str>) -> StoryArticleOut {
    let adds: Vec<String> = row
        .added_text
        .as_deref()
        .unwrap_or("")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    // Timestamps are 'YYYY-MM-DD HH:MM:SS' UTC text, so string order is
    // time order.
    let is_new = !adds.is_empty()
        && match (row.added_at.as_deref(), last_read_at) {
            (Some(added), Some(read)) => added > read,
            (Some(_), None) => true,
            (None, _) => false,
        };
    StoryArticleOut {
        id: row.id,
        url: row.url,
        title: row.title,
        saved_at: row.saved_at,
        adds,
        is_new,
    }
}

/// Renders `list_stories` results as the text an agent shows the reader:
/// a numbered list for display, each line carrying the stable story ID
/// the agent must use in later calls.
///
/// Learning hook (Pavel): see the tests below for the exact format.
pub fn format_story_list(stories: &[StorySummaryOut]) -> String {
    if stories.is_empty() {
        return "No stories match these filters.".to_owned();
    }
    (1..)
        .zip(stories)
        .map(|(n, s)| {
            let mut entry = format!(
                "{n}. [id {}] {} ({}, {}, {})",
                s.id,
                s.title,
                s.kind,
                s.status,
                article_string(s.article_count)
            );
            for tag in &s.tags {
                entry.push_str(" #");
                entry.push_str(tag);
            }
            if !s.one_line.is_empty() {
                entry.push_str("\n   ");
                entry.push_str(&s.one_line);
            }
            entry
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn article_string(count: i64) -> String {
    if count == 1 {
        "1 article".to_string()
    } else {
        format!("{count} articles")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn story(id: i64, title: &str, tags: &[&str], one_line: &str) -> StorySummaryOut {
        StorySummaryOut {
            id,
            title: title.into(),
            kind: StoryKind::News,
            status: StoryStatus::Unread,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            article_count: 2,
            one_line: one_line.into(),
        }
    }

    #[test]
    fn fts_phrase_doubles_quotes() {
        assert_eq!(fts_phrase(r#"say "hi""#), r#""say ""hi""""#);
    }

    #[test]
    fn normalize_tag_strips_hash_and_case() {
        assert_eq!(normalize_tag(" #DIY "), "diy");
    }

    #[test]
    fn formats_numbered_list_with_ids() {
        let text = format_story_list(&[
            story(
                7,
                "ESP32-C5 boards ship",
                &["diy", "esp32"],
                "Dual-band chip.",
            ),
            story(3, "HF incident", &[], "Tokens leaked."),
        ]);
        assert_eq!(
            text,
            "1. [id 7] ESP32-C5 boards ship (news, unread, 2 articles) #diy #esp32\n   \
             Dual-band chip.\n\
             2. [id 3] HF incident (news, unread, 2 articles)\n   \
             Tokens leaked."
        );
    }

    #[test]
    fn omits_empty_summary_and_says_one_article() {
        let mut s = story(5, "Quiet", &[], "");
        s.article_count = 1;
        assert_eq!(
            format_story_list(&[s]),
            "1. [id 5] Quiet (news, unread, 1 article)"
        );
    }

    #[test]
    fn empty_list_has_a_message() {
        assert_eq!(format_story_list(&[]), "No stories match these filters.");
    }
}
