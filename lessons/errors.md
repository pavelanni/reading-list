# Errors

## `?` converts through `From`

When a function returns `Result<T, MyError>` and you write `call()?` on a
`Result<U, OtherError>`, Rust calls `MyError::from(other_error)`. One `From`
impl removes `map_err` from every call site.

Seen in: `impl From<reqwest::Error> for FetchError` in `src/fetch.rs`, which
also decides between `Timeout` and `Network` in one place.

## `thiserror` for libraries, `anyhow` for applications

| | `thiserror` | `anyhow` |
| - | ----------- | -------- |
| What you get | Your own enum with a derived `Display` | One opaque `anyhow::Error` |
| Callers can | `match` on variants | Only print or downcast |
| Use when | Callers react differently per case | You only report the error |

This project uses both: `FetchError` is for the worker to match on (a fetch
failure is stored and is not fatal), while database and startup errors use
`anyhow::Result` with `.context("…")` and are only reported.

## Store messages, not foreign error types

`FetchError::Network(String)` stores a message instead of wrapping
`reqwest::Error`. `reqwest::Error` is neither `Clone` nor `PartialEq`, so a
wrapping enum could not derive them, and tests could not write
`assert_eq!(err, FetchError::TooLarge)`.

Wrap the original error only when a caller needs to inspect it later.

## The useful part is often in the source chain

Many libraries put a generic message at the top ("error sending request")
and the real cause ("connection refused") in `Error::source()`. Walk the
chain to report it:

```rust
std::iter::successors(Some(&err as &dyn Error), |e| (*e).source())
    .map(ToString::to_string)
    .collect::<Vec<_>>()
    .join(": ")
```

With `anyhow`, format with `{err:#}` to get the whole chain on one line;
plain `{err}` prints only the top message.

## Never fail quietly

The rule here is that a saved link is never lost. When saving fails,
`on_message` in `src/bot.rs` logs the error *and* tells the user to resend.
`resolve_shortener` returns its input unchanged on any failure. When code
cannot do its job, it should fall back to something visible, not drop the
data.

## Keep secrets out of `Debug`

`#[derive(Debug)]` prints every field. A struct holding an API key would leak
it in any `tracing::info!(?client)` or `{:?}` panic message. Either skip
`Debug` on such structs (`OpenRouterClient` in `src/llm/client.rs`), or
write it by hand and redact the secret (`Config` in `src/config.rs`).
