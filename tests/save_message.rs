//! `handle_message`: everything the bot does, without Telegram.
//!
//! URLs here are not shorteners, so `canonicalize` makes no network calls.

use reading_list::db::Db;
use reading_list::ingest::save::{Action, Incoming, handle_message};
use reading_list::net::{Addresses, WebClient};

const PAVEL: u64 = 1001;

fn msg(text: &str) -> Incoming {
    Incoming {
        user_id: PAVEL,
        text: text.to_string(),
        reply_to_message_id: None,
    }
}

async fn handle(db: &Db, m: Incoming) -> (Action, Vec<i64>) {
    handle_message(
        db,
        &WebClient::new(Addresses::AllowLoopback).unwrap(),
        PAVEL,
        m,
    )
    .await
    .unwrap()
}

fn reply_text(action: Action) -> String {
    match action {
        Action::Reply(text) => text,
        Action::Ignore => panic!("expected a reply, got Ignore"),
    }
}

#[tokio::test]
async fn ignores_other_users() {
    let db = Db::in_memory().await.unwrap();
    let m = Incoming {
        user_id: 666,
        ..msg("https://example.com/a")
    };
    let (action, ids) = handle(&db, m).await;
    assert_eq!(action, Action::Ignore);
    assert!(ids.is_empty());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM article")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0, "nothing may be saved for strangers");
}

#[tokio::test]
async fn saves_and_confirms_with_tags() {
    let db = Db::in_memory().await.unwrap();
    let (action, ids) = handle(&db, msg("https://example.com/a #DIY")).await;
    assert_eq!(reply_text(action), "Saved, tagged #diy");
    assert_eq!(ids.len(), 1);
}

#[tokio::test]
async fn saves_without_tags() {
    let db = Db::in_memory().await.unwrap();
    let (action, _) = handle(&db, msg("https://example.com/a")).await;
    assert_eq!(reply_text(action), "Saved");
}

#[tokio::test]
async fn duplicate_reports_added_tag() {
    let db = Db::in_memory().await.unwrap();
    handle(&db, msg("https://example.com/a #ai")).await;
    // Tracking params differ, so this also checks normalization is applied.
    let (action, _) = handle(&db, msg("https://example.com/a?utm_source=x #ai #diy")).await;
    assert_eq!(reply_text(action), "Already saved (added tag #diy)");
}

#[tokio::test]
async fn duplicate_without_new_tags() {
    let db = Db::in_memory().await.unwrap();
    handle(&db, msg("https://example.com/a #ai")).await;
    let (action, _) = handle(&db, msg("https://example.com/a #ai")).await;
    assert_eq!(reply_text(action), "Already saved");
}

#[tokio::test]
async fn several_urls_get_one_line_each() {
    let db = Db::in_memory().await.unwrap();
    handle(&db, msg("https://example.com/a")).await;
    let (action, ids) = handle(&db, msg("https://example.com/a https://example.com/b")).await;
    assert_eq!(
        reply_text(action),
        "https://example.com/a: Already saved\nhttps://example.com/b: Saved"
    );
    assert_eq!(ids.len(), 2);
}

#[tokio::test]
async fn reply_to_confirmation_adds_note() {
    let db = Db::in_memory().await.unwrap();
    let (_, ids) = handle(&db, msg("https://example.com/a")).await;
    let id = ids[0];
    db.record_bot_reply(77, &ids).await.unwrap();
    sqlx::query("UPDATE article SET process_status = 'done' WHERE id = ?")
        .bind(id)
        .execute(db.pool())
        .await
        .unwrap();

    let reply = Incoming {
        reply_to_message_id: Some(77),
        ..msg("focus on the cost numbers #cost")
    };
    let (action, linked) = handle(&db, reply).await;
    assert_eq!(reply_text(action), "Note added");
    assert_eq!(linked, vec![id], "replying to 'Note added' should work too");

    let (note, status): (String, String) =
        sqlx::query_as("SELECT note, process_status FROM article WHERE id = ?")
            .bind(id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(note, "focus on the cost numbers");
    assert_eq!(status, "pending");
}

#[tokio::test]
async fn reply_to_unknown_message_is_a_normal_message() {
    let db = Db::in_memory().await.unwrap();
    let m = Incoming {
        reply_to_message_id: Some(12345),
        ..msg("https://example.com/a")
    };
    let (action, _) = handle(&db, m).await;
    assert_eq!(reply_text(action), "Saved");
}

#[tokio::test]
async fn no_url_gets_help() {
    let db = Db::in_memory().await.unwrap();
    let (action, ids) = handle(&db, msg("hello")).await;
    assert_eq!(
        reply_text(action),
        "Send a link. Add #tags, and a line starting with >> for instructions to the agent."
    );
    assert!(ids.is_empty());
}
