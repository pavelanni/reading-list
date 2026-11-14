-- Initial schema. Timestamps are UTC text ('YYYY-MM-DD HH:MM:SS') so they
-- compare correctly as strings and against datetime('now', ...).

CREATE TABLE story (
    id               INTEGER PRIMARY KEY,
    title            TEXT NOT NULL,
    kind             TEXT NOT NULL CHECK (kind IN ('news', 'idea')),
    summary          TEXT NOT NULL DEFAULT '',
    status           TEXT NOT NULL DEFAULT 'unread'
                     CHECK (status IN ('unread', 'read', 'dropped')),
    created_at       TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at       TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_new_info_at TEXT,
    last_read_at     TEXT
);

CREATE TABLE article (
    id               INTEGER PRIMARY KEY,
    url              TEXT NOT NULL UNIQUE,
    original_url     TEXT NOT NULL,
    title            TEXT,
    extracted_text   TEXT,
    note             TEXT,
    share_text       TEXT,
    saved_at         TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    fetch_status     TEXT NOT NULL DEFAULT 'pending'
                     CHECK (fetch_status IN ('pending', 'ok', 'failed')),
    process_status   TEXT NOT NULL DEFAULT 'pending'
                     CHECK (process_status IN ('pending', 'processing', 'done', 'failed')),
    story_id         INTEGER REFERENCES story (id),
    one_line_summary TEXT,
    error            TEXT,
    attempts         INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX article_process_status ON article (process_status);
CREATE INDEX article_story_id ON article (story_id);

CREATE TABLE tag (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE article_tag (
    article_id INTEGER NOT NULL REFERENCES article (id) ON DELETE CASCADE,
    tag_id     INTEGER NOT NULL REFERENCES tag (id),
    PRIMARY KEY (article_id, tag_id)
);

CREATE TABLE story_addition (
    id         INTEGER PRIMARY KEY,
    story_id   INTEGER NOT NULL REFERENCES story (id),
    article_id INTEGER NOT NULL REFERENCES article (id) ON DELETE CASCADE,
    added_text TEXT NOT NULL DEFAULT '', -- empty = nothing new
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX story_addition_story_id ON story_addition (story_id);

CREATE TABLE bot_reply (
    telegram_message_id INTEGER NOT NULL,
    article_id          INTEGER NOT NULL REFERENCES article (id) ON DELETE CASCADE,
    PRIMARY KEY (telegram_message_id, article_id)
);

CREATE TABLE model_call (
    id                INTEGER PRIMARY KEY,
    article_id        INTEGER REFERENCES article (id) ON DELETE SET NULL,
    purpose           TEXT NOT NULL,
    model             TEXT NOT NULL,
    prompt_tokens     INTEGER,
    completion_tokens INTEGER,
    cost              REAL,
    created_at        TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Full-text search. External-content tables index the source rows without
-- storing a second copy of the text; triggers keep them in sync.

CREATE VIRTUAL TABLE article_fts USING fts5 (
    title, extracted_text, content = 'article', content_rowid = 'id'
);

CREATE TRIGGER article_fts_insert AFTER INSERT ON article BEGIN
    INSERT INTO article_fts (rowid, title, extracted_text)
    VALUES (new.id, new.title, new.extracted_text);
END;

CREATE TRIGGER article_fts_delete AFTER DELETE ON article BEGIN
    INSERT INTO article_fts (article_fts, rowid, title, extracted_text)
    VALUES ('delete', old.id, old.title, old.extracted_text);
END;

-- Only re-index when indexed columns change, not on every status update.
CREATE TRIGGER article_fts_update AFTER UPDATE OF title, extracted_text ON article BEGIN
    INSERT INTO article_fts (article_fts, rowid, title, extracted_text)
    VALUES ('delete', old.id, old.title, old.extracted_text);
    INSERT INTO article_fts (rowid, title, extracted_text)
    VALUES (new.id, new.title, new.extracted_text);
END;

CREATE VIRTUAL TABLE story_fts USING fts5 (
    title, summary, content = 'story', content_rowid = 'id'
);

CREATE TRIGGER story_fts_insert AFTER INSERT ON story BEGIN
    INSERT INTO story_fts (rowid, title, summary)
    VALUES (new.id, new.title, new.summary);
END;

CREATE TRIGGER story_fts_delete AFTER DELETE ON story BEGIN
    INSERT INTO story_fts (story_fts, rowid, title, summary)
    VALUES ('delete', old.id, old.title, old.summary);
END;

CREATE TRIGGER story_fts_update AFTER UPDATE OF title, summary ON story BEGIN
    INSERT INTO story_fts (story_fts, rowid, title, summary)
    VALUES ('delete', old.id, old.title, old.summary);
    INSERT INTO story_fts (rowid, title, summary)
    VALUES (new.id, new.title, new.summary);
END;
