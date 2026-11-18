# sqlx and SQLite

## Compile-time checked queries

`sqlx::query!` connects to the database in `DATABASE_URL` *while compiling*
and asks SQLite what the query returns, so a typo in a column name is a
compile error. Without `DATABASE_URL`, the macro reads cached answers from
`.sqlx/*.json`. That cache is committed, so CI builds without a database.

After changing a query or migration, refresh the cache. The steps are in
`CLAUDE.md` under "Database and sqlx".

## Transactions: `&mut *tx` and `&mut **tx`

Queries run on anything implementing sqlx's `Executor`. A `Transaction` is
not one itself, but dereferences to a connection that is.

| You have | Write | Why |
| -------- | ----- | --- |
| `tx: Transaction` | `&mut *tx` | One deref reaches the connection |
| `tx: &mut Transaction` (in a helper) | `&mut **tx` | One deref through the reference, one to the connection |

Passing `&mut Transaction` into helpers (`append_note`, `attach_tags` in
`src/db/articles.rs`) keeps everything in `save_article` in one transaction:
all of it commits, or none.

## Feature flags

With `default-features = false`, a crate builds only the parts you list.
The `sqlite` feature compiles SQLite from source (`bundled`), so the
container needs no system library and FTS5 is always available. Check
feature names in the crate's `Cargo.toml` after a major upgrade; they
change (sqlx 0.9 made `offline` part of `macros`).

## Triggers that fire only when needed

```sql
CREATE TRIGGER article_fts_update AFTER UPDATE OF title, extracted_text ON article
```

`UPDATE OF <columns>` limits the trigger to changes in those columns. Without
it, every status change would delete and re-insert the article in the
full-text index. Seen in: `migrations/0001_init.sql`.
