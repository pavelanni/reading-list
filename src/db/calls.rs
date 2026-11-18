//! Model call log: which model ran for which article, and what it cost.

use anyhow::Result;

use super::Db;
use crate::llm::Usage;

impl Db {
    /// Records one model call so cost per article is visible.
    pub async fn record_model_call(
        &self,
        article_id: i64,
        purpose: &str,
        model: &str,
        usage: &Usage,
    ) -> Result<()> {
        sqlx::query!(
            "INSERT INTO model_call (article_id, purpose, model, prompt_tokens, completion_tokens, cost)
             VALUES (?, ?, ?, ?, ?, ?)",
            article_id,
            purpose,
            model,
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.cost
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
