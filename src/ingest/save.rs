//! What the bot does with one message, independent of Telegram.
//!
//! `bot.rs` converts a Telegram update into [`Incoming`], calls
//! [`handle_message`], and sends the [`Action`]. Everything testable lives
//! here.

use anyhow::Result;

use super::normalize::canonicalize;
use super::parse::{ParsedMessage, parse_message};
use crate::db::{Db, NewArticle, SaveOutcome};

pub const HELP: &str =
    "Send a link. Add #tags, and a line starting with >> for instructions to the agent.";

/// A message as the bot received it, stripped to what the logic needs.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub user_id: u64,
    /// Message text, or the caption of a photo/document.
    pub text: String,
    /// Set when this message replies to another (possibly a bot confirmation).
    pub reply_to_message_id: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Say nothing (messages from anyone but Pavel).
    Ignore,
    Reply(String),
}

/// Handles one message. Returns the action plus the article IDs the reply
/// refers to, so the caller can record them once the reply is sent and later
/// map a reply-to-the-reply back to these articles.
pub async fn handle_message(
    db: &Db,
    http: &reqwest::Client,
    allowed_user_id: u64,
    msg: Incoming,
) -> Result<(Action, Vec<i64>)> {
    if msg.user_id != allowed_user_id {
        return Ok((Action::Ignore, Vec::new()));
    }
    let parsed = parse_message(&msg.text);

    if let Some(reply_to) = msg.reply_to_message_id {
        let ids = db.articles_for_bot_reply(reply_to).await?;
        if !ids.is_empty() {
            let note = reply_note(&parsed);
            for &id in &ids {
                db.add_note(id, &note, &parsed.tags).await?;
            }
            return Ok((Action::Reply("Note added".into()), ids));
        }
        // A reply to something we don't know: treat it as a fresh message.
    }

    if parsed.urls.is_empty() {
        return Ok((Action::Reply(HELP.into()), Vec::new()));
    }

    let several = parsed.urls.len() > 1;
    let mut lines = Vec::with_capacity(parsed.urls.len());
    let mut ids = Vec::with_capacity(parsed.urls.len());
    for original in &parsed.urls {
        let url = canonicalize(http, original).await;
        let outcome = db
            .save_article(NewArticle {
                url: url.to_string(),
                original_url: original.to_string(),
                tags: parsed.tags.clone(),
                note: parsed.note.clone(),
                share_text: parsed.share_text.clone(),
            })
            .await?;
        let line = confirmation(&outcome, &parsed.tags);
        lines.push(if several {
            format!("{url}: {line}")
        } else {
            line
        });
        ids.push(outcome.article_id);
    }
    Ok((Action::Reply(lines.join("\n")), ids))
}

/// In a reply, all text is the note: there is no share text to keep apart.
/// URLs in a reply are ignored.
fn reply_note(parsed: &ParsedMessage) -> String {
    [parsed.share_text.as_deref(), parsed.note.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
}

/// `Saved, tagged #a #b` / `Already saved (added tag #x)`.
fn confirmation(outcome: &SaveOutcome, tags: &[String]) -> String {
    let hashtags = |tags: &[String]| {
        tags.iter()
            .map(|t| format!("#{t}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    match (outcome.created, outcome.added_tags.as_slice()) {
        (true, _) if tags.is_empty() => "Saved".into(),
        (true, _) => format!("Saved, tagged {}", hashtags(tags)),
        (false, []) => "Already saved".into(),
        (false, [_]) => format!(
            "Already saved (added tag {})",
            hashtags(&outcome.added_tags)
        ),
        (false, _) => format!(
            "Already saved (added tags {})",
            hashtags(&outcome.added_tags)
        ),
    }
}
