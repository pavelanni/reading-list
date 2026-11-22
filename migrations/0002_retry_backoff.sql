-- Transient failures (timeouts, 5xx, rate limits, database trouble) back
-- off per article instead of blocking the queue, and give up after a cap.
-- NULL retry_at means "ready now".
ALTER TABLE article ADD COLUMN retry_at TEXT;
ALTER TABLE article ADD COLUMN transient_failures INTEGER NOT NULL DEFAULT 0;
-- Why the page could not be fetched. Kept apart from `error`, which holds
-- the latest processing error and would otherwise overwrite it.
ALTER TABLE article ADD COLUMN fetch_error TEXT;

CREATE INDEX article_ready ON article (process_status, retry_at);
