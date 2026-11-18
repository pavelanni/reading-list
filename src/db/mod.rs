//! Database access. All SQL lives under this module.

mod articles;
mod calls;

pub use articles::{NewArticle, SaveOutcome};

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

/// Whether a story tracks news or a project idea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize)]
#[sqlx(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum StoryKind {
    News,
    Idea,
}

/// Reading status of a story.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize)]
#[sqlx(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum StoryStatus {
    Unread,
    Read,
    Dropped,
}

impl StoryKind {
    /// The lowercase name used in the database, JSON and prompts.
    pub fn as_str(self) -> &'static str {
        match self {
            StoryKind::News => "news",
            StoryKind::Idea => "idea",
        }
    }
}

impl StoryStatus {
    /// The lowercase name used in the database, JSON and prompts.
    pub fn as_str(self) -> &'static str {
        match self {
            StoryStatus::Unread => "unread",
            StoryStatus::Read => "read",
            StoryStatus::Dropped => "dropped",
        }
    }
}

/// Handle to the database. Cheap to clone: the pool is reference-counted.
#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// Opens (creating if needed) the SQLite file at `path` in WAL mode and
    /// applies pending migrations.
    pub async fn connect(path: &Path) -> Result<Db> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .connect_with(opts)
            .await
            .with_context(|| format!("opening database {}", path.display()))?;
        Self::migrate(pool).await
    }

    /// A private in-memory database for tests.
    pub async fn in_memory() -> Result<Db> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")?;
        // Every connection to `:memory:` is a separate empty database, so the
        // pool must hold exactly one connection and never recycle it.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(opts)
            .await?;
        Self::migrate(pool).await
    }

    async fn migrate(pool: SqlitePool) -> Result<Db> {
        sqlx::migrate!()
            .run(&pool)
            .await
            .context("running migrations")?;
        Ok(Db { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
