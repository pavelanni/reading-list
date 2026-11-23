# Rust lessons

Notes collected while building this project, one file per theme. Each
lesson points at the real code where the issue came up, so you can read
the example in context.

| Lesson | Topics |
| ------ | ------ |
| [Ownership and borrowing](ownership-and-borrowing.md) | References that outlive their owner, `Cow`, borrow-then-move, `&&T` |
| [Strings and options](strings-and-options.md) | `Option`-returning string methods, `then`, let chains, `let … else`, bytes vs chars, escaping untrusted text |
| [Errors](errors.md) | `?` and `From`, `thiserror` vs `anyhow`, error source chains, `downcast_ref` |
| [Async and Tokio](async-and-tokio.md) | Blocking work, `Send`, async traits, shared state, streaming bodies |
| [Testing](testing.md) | Tests that pass too early, pure core and thin shell, mock server traps |
| [sqlx and SQLite](sqlx-and-sqlite.md) | Compile-time checked queries, offline cache, transactions, feature flags |
| [Enums and match](enums-and-match.md) | `if` vs `match`, exhaustiveness, tuple patterns, `_` arms, patterns vs guards, `\|` vs `\|\|` |
| [Newtypes and IP addresses](newtypes-and-ip-addresses.md) | Newtype as a guard, SSRF checks, `is_global` alternatives, IPv4-mapped IPv6 |
| [Becoming fluent](fluency.md) | Learning the standard library and crates, clippy pedantic, `cargo tree -i` |

## How to use these

- Read the lesson, then open the linked function and find the pattern.
- When the compiler rejects something, search these files for the error
  text (`does not live long enough`, `cannot be sent between threads`).
- Add a lesson when a mistake costs you more than a few minutes.
