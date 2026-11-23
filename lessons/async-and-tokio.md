# Async and Tokio

## Do not block the runtime

Tokio runs many tasks on a few threads and only switches between them at
`.await`. Code that computes for a long time without an `.await` stops every
other task on that thread: Telegram polling, MCP requests, the worker.

Move CPU-heavy or blocking work to the blocking pool:

```rust
tokio::task::spawn_blocking(move || extract(&html, &base)).await
```

Rough rule: more than about 100 µs without an `.await` belongs in
`spawn_blocking`. Seen in: `fetch_article` in `src/fetch.rs` (parsing a page
of up to 5 MB).

## `Send`: what may cross threads

A future given to `tokio::spawn` may move to another thread at every
`.await`, so everything it holds across an `.await` must be `Send`. Types
built on `Rc` or similar single-threaded reference counting are not.

When a library type is not `Send` (dom_smoothie's `Readability`), create it
on the thread that uses it. The `spawn_blocking` closure above captures only
two `String`s, and builds `Readability` inside.

If you see `future cannot be sent between threads safely`, look for a
non-`Send` value held across an `.await`, and create it later or drop it
earlier.

## Async functions in traits

`async fn` in a trait works, but callers cannot require the future to be
`Send`. Spell the bound out in the trait, and keep `async fn` in the impl:

```rust
pub trait Notifier: Send + Sync {
    fn notify(&self, text: &str) -> impl Future<Output = ()> + Send;
}
impl Notifier for TelegramNotifier {
    async fn notify(&self, text: &str) { /* … */ }
}
```

The compiler checks the impl against the bound, so a non-`Send` value inside
`notify` fails right there. Such traits are not `dyn`-compatible: consumers
take a generic `N: Notifier`. Seen in: `src/bot.rs`.

## Sharing state between tasks

Pass handles in, never use globals. Handles must be
`Clone + Send + Sync + 'static`, and cloning should be cheap:

| Value | Why cloning is cheap |
| ----- | -------------------- |
| `Db` (wraps `SqlitePool`) | The pool is reference-counted inside |
| `reqwest::Client` | Reference-counted inside; reuse one per process |
| `Arc<Config>` | Explicit `Arc`, so the `String`s are not copied |

`teloxide`'s `dptree::deps![db, http, cfg]` injects these into handlers by
type. Seen in: `bot::run`.

## Let the framework handle concurrency

`teloxide` already runs handlers concurrently, in order within one chat. With
a single allowed user, messages are processed one at a time, which also
avoids two saves of the same URL racing. No `tokio::spawn` per message.

## Stream bodies with a cap

`resp.bytes()` and `resp.text()` buffer whatever the server sends.
`resp.chunk().await` returns the body piece by piece, so you can stop at a
limit, whatever `Content-Length` says. Seen in: `fetch_article`.

## `async move` takes the whole variable

`async move { worker::run(db.clone(), …) }` moves `db` itself into the
block, even though only a clone is used inside. A later `mcp::serve(db, …)`
then fails with "use of moved value". Clone first, outside the block, and
move the clone in: `let worker_db = db.clone();`. Seen in: `main`.

## Shutdown with one `CancellationToken`

`main` creates one token and gives each service a clone. On SIGTERM,
SIGINT, or the first service to stop, it calls `cancel()`, then waits a
bounded time (`tokio::time::timeout`) for the rest. Each service turns
the token into its own stop signal: `select!` in the worker,
`with_graceful_shutdown` in axum, a `ShutdownToken` in teloxide. A
`JoinSet` holds the tasks; dropping it aborts any that did not stop in
time. Seen in: `main`, `bot::run`.
