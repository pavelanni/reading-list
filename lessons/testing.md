# Testing

## Watch it fail first

A new test should be red before the code exists. If it passes at once, it is
testing something else: existing behaviour, a coincidence, or the wrong
expectation.

Two examples from `parse_message`:

- `duplicate_tags_and_urls_collapse` passed against code using
  `Vec::dedup()`, because the duplicate URLs happened to be adjacent.
- `tags_keep_message_order` first expected `["apple", "zebra"]`: the
  *sorted* order the buggy code produced. A test written to catch the bug
  confirmed it instead.

Write the expected value from the spec, not from the output you just saw.

## `assert_eq!(actual, expected)`

Rust prints the first argument as `left` and the second as `right`. With the
`(actual, expected)` convention, `left` is always what your code produced.

## Pin decisions, not only bugs

When you deliberately pick behaviour the spec does not state, add a test for
it, so nobody "fixes" it back. Example:
`note_marker_after_url_and_tag_on_same_line` records that `>>` after a URL on
the same line still starts a note. This kind of test passes immediately;
its job is to fail later.

## Pure core, thin shell

`handle_message` in `src/ingest/save.rs` takes our own `Incoming` struct,
never a `teloxide::Message`. Every bot decision is tested without Telegram.
`src/bot.rs` only converts types. The same split keeps `normalize_url` pure
and `resolve_shortener` separate.

Push logic toward functions that take plain data; keep I/O at the edges.

## Test setup can lie

`unreachable_server_is_network_error` first dropped a wiremock `MockServer`
to get a dead port. It failed with `Http(404)`: wiremock *pools* servers,
and a dropped one still listens with its mocks cleared. The fix was a port
from `TcpListener::bind("127.0.0.1:0")` that is closed right away.

The worker tests served the fixture with
`.insert_header("Content-Type", "text/html").set_body_string(…)`, and
every fetch failed with `not an HTML page (text/plain)`. `set_body_string`
also sets the MIME type to `text/plain`, which overrides the header. Use
`set_body_raw(body, "text/html")`, or `set_body_bytes` plus the header.

When a test fails, print what you actually got before changing the code.
The bug may be in the test.

## In-memory SQLite needs one connection

Every connection to `sqlite::memory:` opens a separate, empty database. A
pool with several connections, or one that recycles idle connections, loses
data between queries. `Db::in_memory` sets `max_connections(1)` and disables
idle timeout and max lifetime.
