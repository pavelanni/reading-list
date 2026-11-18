//! Recording model calls for cost tracking.

use reading_list::db::{Db, NewArticle};
use reading_list::llm::Usage;

#[tokio::test]
async fn records_model_call_with_usage() {
    let db = Db::in_memory().await.unwrap();
    let saved = db
        .save_article(NewArticle {
            url: "https://example.com/a".into(),
            original_url: "https://example.com/a".into(),
            tags: vec![],
            note: None,
            share_text: None,
        })
        .await
        .unwrap();
    let usage = Usage {
        prompt_tokens: 120,
        completion_tokens: 30,
        cost: Some(0.00042),
    };
    db.record_model_call(saved.article_id, "assign", "test/model", &usage)
        .await
        .unwrap();

    let row: (i64, String, String, i64, i64, Option<f64>) = sqlx::query_as(
        "SELECT article_id, purpose, model, prompt_tokens, completion_tokens, cost FROM model_call",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            saved.article_id,
            "assign".into(),
            "test/model".into(),
            120,
            30,
            Some(0.00042)
        )
    );
}
