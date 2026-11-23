//! Read and triage logic behind the MCP tools.

use reading_list::db::{Db, FetchStatus, NewArticle, ProcessStatus, StoryKind, StoryStatus};
use reading_list::query::{self, ListFilter};
use sqlx::Row;

async fn db() -> Db {
    Db::in_memory().await.unwrap()
}

/// Inserts a story last updated `days_ago` days ago.
async fn story(db: &Db, title: &str, kind: &str, status: &str, days_ago: i64) -> i64 {
    sqlx::query(
        "INSERT INTO story (title, kind, status, summary, updated_at)
         VALUES (?, ?, ?, ?, datetime('now', ?)) RETURNING id",
    )
    .bind(title)
    .bind(kind)
    .bind(status)
    .bind(format!("Summary of {title}."))
    .bind(format!("-{days_ago} days"))
    .fetch_one(db.pool())
    .await
    .unwrap()
    .get(0)
}

/// Saves an article with `tags` and attaches it to `story_id`.
async fn article(db: &Db, url: &str, story_id: i64, tags: &[&str]) -> i64 {
    let id = db
        .save_article(NewArticle {
            url: url.into(),
            original_url: url.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            note: None,
            share_text: None,
        })
        .await
        .unwrap()
        .article_id;
    sqlx::query("UPDATE article SET story_id = ?, process_status = 'done' WHERE id = ?")
        .bind(story_id)
        .bind(id)
        .execute(db.pool())
        .await
        .unwrap();
    id
}

async fn addition(db: &Db, story_id: i64, article_id: i64, text: &str, at: &str) {
    sqlx::query(
        "INSERT INTO story_addition (story_id, article_id, added_text, created_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(story_id)
    .bind(article_id)
    .bind(text)
    .bind(at)
    .execute(db.pool())
    .await
    .unwrap();
}

fn ids(stories: &[query::StorySummaryOut]) -> Vec<i64> {
    stories.iter().map(|s| s.id).collect()
}

#[tokio::test]
async fn list_defaults_unread_last_7_days() {
    let db = db().await;
    let fresh = story(&db, "Fresh", "news", "unread", 1).await;
    story(&db, "Read", "news", "read", 1).await;
    story(&db, "Old", "news", "unread", 10).await;
    article(&db, "https://ex.com/1", fresh, &["diy"]).await;
    article(&db, "https://ex.com/2", fresh, &["diy", "ai"]).await;

    let got = query::list_stories(&db, &ListFilter::default())
        .await
        .unwrap();

    assert_eq!(ids(&got), vec![fresh]);
    assert_eq!(got[0].article_count, 2);
    assert_eq!(got[0].tags, vec!["ai", "diy"]);
    assert_eq!(got[0].one_line, "Summary of Fresh.");
}

#[tokio::test]
async fn list_filters_by_kind_and_tag() {
    let db = db().await;
    let news = story(&db, "News", "news", "unread", 1).await;
    let idea = story(&db, "Idea", "idea", "read", 1).await;
    let other = story(&db, "Other idea", "idea", "unread", 1).await;
    article(&db, "https://ex.com/1", news, &["diy"]).await;
    article(&db, "https://ex.com/2", idea, &["diy"]).await;
    article(&db, "https://ex.com/3", other, &["ai"]).await;

    let filter = ListFilter {
        status: None,
        kind: Some(StoryKind::Idea),
        tag: Some("#DIY".into()),
        ..ListFilter::default()
    };
    let got = query::list_stories(&db, &filter).await.unwrap();

    assert_eq!(ids(&got), vec![idea]);
}

#[tokio::test]
async fn get_story_marks_new_since_last_read() {
    let db = db().await;
    let s = story(&db, "Story", "news", "unread", 0).await;
    sqlx::query("UPDATE story SET last_read_at = '2026-01-02 00:00:00' WHERE id = ?")
        .bind(s)
        .execute(db.pool())
        .await
        .unwrap();
    let old = article(&db, "https://ex.com/old", s, &[]).await;
    let fresh = article(&db, "https://ex.com/fresh", s, &[]).await;
    let nothing = article(&db, "https://ex.com/nothing", s, &[]).await;
    addition(&db, s, old, "old fact", "2026-01-01 00:00:00").await;
    addition(
        &db,
        s,
        fresh,
        "new fact\nsecond fact",
        "2026-01-03 00:00:00",
    )
    .await;
    addition(&db, s, nothing, "", "2026-01-03 00:00:00").await;

    let got = query::get_story(&db, s).await.unwrap();

    let new: Vec<(i64, bool)> = got.articles.iter().map(|a| (a.id, a.is_new)).collect();
    assert_eq!(new, vec![(old, false), (fresh, true), (nothing, false)]);
    assert_eq!(got.articles[1].adds, vec!["new fact", "second fact"]);
    assert!(got.articles[2].adds.is_empty());
    assert_eq!(got.summary, "Summary of Story.");
}

#[tokio::test]
async fn get_story_unknown_id_errors() {
    let db = db().await;
    let err = query::get_story(&db, 999).await.unwrap_err();
    assert!(err.to_string().contains("999"), "{err}");
}

#[tokio::test]
async fn set_read_records_last_read_at() {
    let db = db().await;
    let s = story(&db, "Story", "news", "unread", 0).await;

    query::set_story_status(&db, s, StoryStatus::Read)
        .await
        .unwrap();

    let got = query::get_story(&db, s).await.unwrap();
    assert_eq!(got.status, StoryStatus::Read);
    assert!(got.last_read_at.is_some());
    assert!(
        query::set_story_status(&db, 999, StoryStatus::Read)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn search_finds_article_text() {
    let db = db().await;
    let s = story(&db, "Boards", "news", "unread", 0).await;
    let a = article(&db, "https://ex.com/z", s, &[]).await;
    db.store_fetched(
        a,
        Some("Dev boards"),
        "The quick zephyr board ships in May.",
    )
    .await
    .unwrap();

    let got = query::search(&db, "zephyr", 20).await.unwrap();

    assert_eq!(got.articles.len(), 1);
    assert_eq!(got.articles[0].id, a);
    assert_eq!(got.articles[0].story_id, Some(s));
    assert!(got.articles[0].snippet.contains("[zephyr]"), "{:?}", got);

    let by_story = query::search(&db, "boards", 20).await.unwrap();
    assert_eq!(by_story.stories[0].id, s);
}

#[tokio::test]
async fn search_with_quotes_does_not_error() {
    let db = db().await;
    let got = query::search(&db, r#"he said "hi" AND (x* OR"#, 20)
        .await
        .unwrap();
    assert!(got.stories.is_empty() && got.articles.is_empty());
}

#[tokio::test]
async fn reprocess_resets_attempts() {
    let db = db().await;
    let s = story(&db, "Story", "news", "unread", 0).await;
    let a = article(&db, "https://ex.com/f", s, &[]).await;
    sqlx::query(
        "UPDATE article SET process_status = 'failed', fetch_status = 'failed',
                            attempts = 3, error = 'bad JSON' WHERE id = ?",
    )
    .bind(a)
    .execute(db.pool())
    .await
    .unwrap();
    let failed = query::list_failed(&db).await.unwrap();
    assert_eq!(failed.iter().map(|f| f.id).collect::<Vec<_>>(), vec![a]);
    assert_eq!(failed[0].error.as_deref(), Some("bad JSON"));

    query::reprocess_article(&db, a).await.unwrap();

    let got = query::get_article(&db, a).await.unwrap();
    assert_eq!(got.process_status, ProcessStatus::Pending);
    assert_eq!(got.fetch_status, FetchStatus::Pending);
    assert_eq!(got.error, None);
    let attempts: i64 = sqlx::query("SELECT attempts FROM article WHERE id = ?")
        .bind(a)
        .fetch_one(db.pool())
        .await
        .unwrap()
        .get(0);
    assert_eq!(attempts, 0);
    assert!(query::list_failed(&db).await.unwrap().is_empty());
}

#[tokio::test]
async fn reprocess_refuses_article_in_progress() {
    let db = db().await;
    let s = story(&db, "Story", "news", "unread", 0).await;
    let a = article(&db, "https://ex.com/p", s, &[]).await;
    sqlx::query("UPDATE article SET process_status = 'processing' WHERE id = ?")
        .bind(a)
        .execute(db.pool())
        .await
        .unwrap();

    assert!(query::reprocess_article(&db, a).await.is_err());
    assert!(query::reprocess_article(&db, 999).await.is_err());
}

#[tokio::test]
async fn get_article_returns_text_note_and_tags() {
    let db = db().await;
    let id = db
        .save_article(NewArticle {
            url: "https://ex.com/a".into(),
            original_url: "https://t.co/x".into(),
            tags: vec!["diy".into()],
            note: Some("check the pinout".into()),
            share_text: Some("Cool board".into()),
        })
        .await
        .unwrap()
        .article_id;
    db.store_fetched(id, Some("Title"), "Body text")
        .await
        .unwrap();

    let got = query::get_article(&db, id).await.unwrap();

    assert_eq!(got.text.as_deref(), Some("Body text"));
    assert_eq!(got.note.as_deref(), Some("check the pinout"));
    assert_eq!(got.share_text.as_deref(), Some("Cool board"));
    assert_eq!(got.tags, vec!["diy"]);
    assert_eq!(got.original_url, "https://t.co/x");
}

#[tokio::test]
async fn reprocess_clears_backoff_and_fetch_error() {
    let db = db().await;
    let s = story(&db, "Story", "news", "unread", 0).await;
    let a = article(&db, "https://ex.com/b", s, &[]).await;
    sqlx::query(
        "UPDATE article SET process_status = 'failed', transient_failures = 5,
                            retry_at = datetime('now', '+1 hour'), fetch_error = 'timed out'
         WHERE id = ?",
    )
    .bind(a)
    .execute(db.pool())
    .await
    .unwrap();

    query::reprocess_article(&db, a).await.unwrap();

    let row =
        sqlx::query("SELECT transient_failures, retry_at, fetch_error FROM article WHERE id = ?")
            .bind(a)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(row.get::<i64, _>(0), 0);
    assert_eq!(row.get::<Option<String>, _>(1), None);
    assert_eq!(row.get::<Option<String>, _>(2), None);
    let claimed = db.claim_next_pending().await.unwrap().map(|r| r.id);
    assert_eq!(
        claimed,
        Some(a),
        "article must be ready now, not after the old retry_at"
    );
}

#[tokio::test]
async fn fetch_error_is_reported() {
    let db = db().await;
    let s = story(&db, "Story", "news", "unread", 0).await;
    let a = article(&db, "https://ex.com/404", s, &[]).await;
    db.store_fetch_failure(a, "HTTP 404").await.unwrap();

    let failed = query::list_failed(&db).await.unwrap();
    assert_eq!(failed[0].fetch_error.as_deref(), Some("HTTP 404"));
    let got = query::get_article(&db, a).await.unwrap();
    assert_eq!(got.fetch_error.as_deref(), Some("HTTP 404"));
}

#[tokio::test]
async fn since_days_zero_means_today() {
    let db = db().await;
    let s = story(&db, "Earlier today", "news", "unread", 0).await;
    sqlx::query("UPDATE story SET updated_at = datetime('now', '-1 hours') WHERE id = ?")
        .bind(s)
        .execute(db.pool())
        .await
        .unwrap();
    let filter = ListFilter {
        since_days: 0,
        ..ListFilter::default()
    };
    assert_eq!(
        ids(&query::list_stories(&db, &filter).await.unwrap()),
        vec![s]
    );
}

#[tokio::test]
async fn search_matches_words_in_any_order() {
    let db = db().await;
    let s = story(&db, "Boards", "news", "unread", 0).await;
    let a = article(&db, "https://ex.com/w", s, &[]).await;
    db.store_fetched(a, None, "The quick zephyr board ships in May.")
        .await
        .unwrap();

    let got = query::search(&db, "ships zephyr", 20).await.unwrap();
    assert_eq!(
        got.articles.iter().map(|h| h.id).collect::<Vec<_>>(),
        vec![a]
    );
    assert!(
        query::search(&db, "zephyr missing", 20)
            .await
            .unwrap()
            .articles
            .is_empty()
    );
}
