//! Integration tests for article storage. Each test gets a fresh in-memory DB.

use reading_list::db::{Db, NewArticle};

fn article(url: &str, tags: &[&str], note: Option<&str>) -> NewArticle {
    NewArticle {
        url: url.to_string(),
        original_url: url.to_string(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        note: note.map(str::to_string),
        share_text: None,
    }
}

async fn process_status(db: &Db, id: i64) -> String {
    sqlx::query_scalar("SELECT process_status FROM article WHERE id = ?")
        .bind(id)
        .fetch_one(db.pool())
        .await
        .unwrap()
}

async fn mark_done(db: &Db, id: i64) {
    sqlx::query("UPDATE article SET process_status = 'done' WHERE id = ?")
        .bind(id)
        .execute(db.pool())
        .await
        .unwrap();
}

#[tokio::test]
async fn save_new_article_creates() {
    let db = Db::in_memory().await.unwrap();
    let out = db
        .save_article(article("https://example.com/a", &["ai"], None))
        .await
        .unwrap();
    assert!(out.created);
    assert_eq!(out.added_tags, vec!["ai".to_string()]);
    assert_eq!(process_status(&db, out.article_id).await, "pending");
}

#[tokio::test]
async fn save_duplicate_merges_tags() {
    let db = Db::in_memory().await.unwrap();
    let first = db
        .save_article(article("https://example.com/a", &["ai"], None))
        .await
        .unwrap();
    let second = db
        .save_article(article("https://example.com/a", &["ai", "diy"], None))
        .await
        .unwrap();
    assert!(!second.created);
    assert_eq!(second.article_id, first.article_id);
    assert_eq!(second.added_tags, vec!["diy".to_string()]);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM article")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn duplicate_with_note_requeues_done_article() {
    let db = Db::in_memory().await.unwrap();
    let first = db
        .save_article(article("https://example.com/a", &[], Some("first")))
        .await
        .unwrap();
    mark_done(&db, first.article_id).await;
    db.save_article(article("https://example.com/a", &[], Some("second")))
        .await
        .unwrap();
    assert_eq!(process_status(&db, first.article_id).await, "pending");
    let note: String = sqlx::query_scalar("SELECT note FROM article WHERE id = ?")
        .bind(first.article_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(note, "first\nsecond");
}

#[tokio::test]
async fn duplicate_without_note_keeps_done() {
    let db = Db::in_memory().await.unwrap();
    let first = db
        .save_article(article("https://example.com/a", &[], None))
        .await
        .unwrap();
    mark_done(&db, first.article_id).await;
    db.save_article(article("https://example.com/a", &["diy"], None))
        .await
        .unwrap();
    assert_eq!(process_status(&db, first.article_id).await, "done");
}

#[tokio::test]
async fn add_note_requeues_done_article() {
    let db = Db::in_memory().await.unwrap();
    let a = db
        .save_article(article("https://example.com/a", &[], None))
        .await
        .unwrap();
    mark_done(&db, a.article_id).await;
    db.add_note(a.article_id, "look at costs", &["cost".to_string()])
        .await
        .unwrap();
    assert_eq!(process_status(&db, a.article_id).await, "pending");
}

#[tokio::test]
async fn requeued_article_gets_a_fresh_retry_budget() {
    let db = Db::in_memory().await.unwrap();
    let a = db
        .save_article(article("https://example.com/a", &[], None))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE article SET process_status = 'done', transient_failures = 5,
                            retry_at = datetime('now', '+1 hour') WHERE id = ?",
    )
    .bind(a.article_id)
    .execute(db.pool())
    .await
    .unwrap();
    db.add_note(a.article_id, "again", &[]).await.unwrap();
    let (failures, retry_at): (i64, Option<String>) =
        sqlx::query_as("SELECT transient_failures, retry_at FROM article WHERE id = ?")
            .bind(a.article_id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!((failures, retry_at), (0, None));
}

#[tokio::test]
async fn bot_reply_roundtrip() {
    let db = Db::in_memory().await.unwrap();
    let a = db
        .save_article(article("https://example.com/a", &[], None))
        .await
        .unwrap();
    let b = db
        .save_article(article("https://example.com/b", &[], None))
        .await
        .unwrap();
    db.record_bot_reply(42, &[a.article_id, b.article_id])
        .await
        .unwrap();
    let mut ids = db.articles_for_bot_reply(42).await.unwrap();
    ids.sort();
    assert_eq!(ids, vec![a.article_id, b.article_id]);
    assert!(db.articles_for_bot_reply(7).await.unwrap().is_empty());
}
