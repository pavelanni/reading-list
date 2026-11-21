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
