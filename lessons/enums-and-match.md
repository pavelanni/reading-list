# Enums and match

## `if` or `match` on an enum

Example: `story_status_after_merge` in `src/status.rs`.

Both are correct. `if current == StoryStatus::Read` is short and reads
well when there is one exception and everything else passes through.

`match` adds **exhaustiveness checking**: when every variant is named, a
new variant (say `Snoozed`) makes the `match` fail to compile with
`E0004 non-exhaustive patterns`, pointing at the decision you still have
to make. An `if/else` silently sends the new variant down `else`.

| Use `if` for | Use `match` for |
| ------------ | --------------- |
| Thresholds, ranges (`attempts < MAX_ATTEMPTS`) | Branching on which variant a value is |
| Boolean logic | Enums likely to grow |
| One exception, rest unchanged | Rules that read as a table |

Tips:

- Match a tuple, `(current, added_info)`, to decide over two inputs.
- Combine arms with `|`: `(Read, false) | (Unread, _) => current`.
- A trailing `_ =>` arm gives up exhaustiveness: it swallows future
  variants just like `else`. Name the variants when you want the check.
- `==` needs `#[derive(PartialEq)]`; pattern matching needs nothing.

## Patterns instead of guards

Example: `classify_llm_error`, `is_transient_fetch_error` in
`src/status.rs`.

The compiler checks exhaustiveness from patterns only. An arm with an
`if` guard counts as "might not match", so a `match` whose `Http` arms
all have guards (`if codes.contains(code)`) needs a `_` fallback, and
that `_` silently swallows codes and variants nobody decided on (a 502
from a website became a permanent failure this way).

| Instead of | Write |
| ---------- | ----- |
| `Http(c, _) if [401, 403].contains(c)` | `Http(c @ (401 \| 403), _)` |
| `Http(c, _) if *c >= 500` | `Http(500..=599, _)` |
| `_ => …` after the listed codes | `Http(..) => …`, one last arm per variant |

- `@` binds the matched value: `code @ (401 | 403)`.
- Patterns also let the compiler warn about overlapping arms
  (`unreachable pattern`), which guards hide.
- `contains` is for sets known only at runtime (config, files).
- A guard covers a whole or-pattern: `A(x) | B(x) if x > 0`. Arms that
  need *different* guards cannot be joined.
- Check a test after the fact: put the old code back and see it fail.

## `|` in patterns, `||` in expressions

`||` is boolean OR on values: it evaluates the left side and maybe the
right. `|` in a pattern is not an operator at all; it lists alternative
shapes the value may have, like `|` in a regex or grammar. Rust needed a
separate token because a pattern is not evaluated: `Some(1 | 2)` makes
sense, `Some(1 || 2)` would be "`Some` of a bool". In expressions, `|`
is bitwise OR (`flags | MASK`), so `||` in a pattern is a syntax error
and `|` in an `if` silently does the bitwise thing on integers (`bool |
bool` works too but does not short-circuit).

## Guards do not count for exhaustiveness

The compiler ignores `if` guards when checking that a `match` covers
every case. `Ok(x) if flag` plus `Ok(x) if !flag` looks complete but is
rejected, which tempts you to add `_ => …`. Instead, leave the last arm
for that pattern unguarded: it only sees what the guarded arms above
rejected. A wildcard switches the exhaustiveness check off, so a case
added later lands there silently. Seen in: `exit_is_failure`.

## Bind errors, do not discard them

`Ok(Err(_))` throws away the one value that explains the failure. Bind
it (`Ok(Err(err))`) and log it with `error = ?err`; for `anyhow::Error`,
Debug prints the whole context chain. Seen in: `exit_is_failure`.
