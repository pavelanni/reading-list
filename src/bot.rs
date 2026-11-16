//! Telegram adapter: long polling in, replies and notifications out.
//!
//! Deliberately thin. All decisions are in [`crate::ingest::save`]; this file
//! only converts between `teloxide` types and ours. Verified manually.

use std::future::Future;
use std::sync::Arc;

use anyhow::Result;
use teloxide::prelude::*;
use teloxide::types::ChatId;

use crate::config::Config;
use crate::db::Db;
use crate::ingest::save::{Action, Incoming, handle_message};

/// Sends Pavel an unprompted message (e.g. "Summaries paused").
///
/// A trait so the worker can be tested with a fake. Async trait methods are
/// not `dyn`-compatible, so consumers take `N: Notifier` as a generic; the
/// `+ Send` bound lets the returned future run inside `tokio::spawn`.
pub trait Notifier: Send + Sync {
    fn notify(&self, text: &str) -> impl Future<Output = ()> + Send;
}

/// [`Notifier`] that writes to Pavel's private chat with the bot.
#[derive(Clone)]
pub struct TelegramNotifier {
    bot: Bot,
    chat: ChatId,
}

impl TelegramNotifier {
    pub fn new(bot: Bot, allowed_user_id: u64) -> Self {
        // In a private chat, the chat ID is the user's ID.
        let chat = ChatId::from(UserId(allowed_user_id));
        Self { bot, chat }
    }
}

impl Notifier for TelegramNotifier {
    async fn notify(&self, text: &str) {
        // Best effort: a lost notification must never stop processing.
        if let Err(err) = self.bot.send_message(self.chat, text).await {
            tracing::warn!(%err, "telegram notification failed");
        }
    }
}

/// Runs the long-polling dispatcher until it is shut down.
pub async fn run(bot: Bot, db: Db, http: reqwest::Client, cfg: Arc<Config>) -> Result<()> {
    let handler = Update::filter_message().endpoint(on_message);
    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![db, http, cfg])
        // Edited messages, reactions etc.: ignore without logging noise.
        .default_handler(|_| async {})
        .build()
        .dispatch()
        .await;
    Ok(())
}

/// One incoming message. Arguments are injected by `dptree` by type.
async fn on_message(
    bot: Bot,
    msg: Message,
    db: Db,
    http: reqwest::Client,
    cfg: Arc<Config>,
) -> ResponseResult<()> {
    let Some(user) = msg.from.as_ref() else {
        return Ok(()); // channel posts have no sender
    };
    let incoming = Incoming {
        user_id: user.id.0,
        text: msg.text().or(msg.caption()).unwrap_or_default().to_owned(),
        reply_to_message_id: msg.reply_to_message().map(|m| m.id.0),
    };

    let (text, ids) = match handle_message(&db, &http, cfg.allowed_user_id, incoming).await {
        Ok((Action::Ignore, _)) => return Ok(()),
        Ok((Action::Reply(text), ids)) => (text, ids),
        Err(err) => {
            // Tell Pavel, so a link is never lost silently.
            tracing::error!(error = %format!("{err:#}"), "saving message failed");
            (
                "Could not save this, please send it again.".to_owned(),
                Vec::new(),
            )
        }
    };

    let sent = bot.send_message(msg.chat.id, text).await?;
    if !ids.is_empty()
        && let Err(err) = db.record_bot_reply(sent.id.0, &ids).await
    {
        tracing::warn!(error = %format!("{err:#}"), "recording bot reply failed");
    }
    Ok(())
}
