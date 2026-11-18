# Strings and options

## String methods that return `Option`

`strip_prefix` and `strip_suffix` answer two questions at once: does the
string start (or end) with this, and what is left? Use them instead of
`starts_with` followed by slicing.

```rust
if let Some(tag) = word.strip_prefix('#') { /* tag has no '#' */ }
let note = line.trim_start().strip_prefix(">>");  // Option<&str>
```

Watch the edge case: `"#".strip_prefix('#')` is `Some("")`. An empty tag was
a real bug in `parse_message`.

## Borrowed pieces are free

`split_whitespace()` and `lines()` yield `&str` slices pointing into the
original text; nothing is copied. Collecting them into a `Vec<&str>` costs
only the vector. Allocate (`to_string`, `join`) only for what goes into the
result.

`join` puts separators *between* items, so it never produces the leading
space you get from `push_str(" "); push_str(word)` in a loop.

## Turning a value into an `Option`

| You have | You want | Write |
| -------- | -------- | ----- |
| `bool` and a value | `Some(value)` if true | `cond.then(\|\| value)` |
| `String` | `None` if empty | `Some(s).filter(\|s\| !s.is_empty())` |
| `Option<&str>` | compare with a literal | `opt == Some("")` |

`then` takes a closure, so the value is only computed when the condition is
true. `then_some(value)` exists too but always evaluates `value`.

## Let chains and `let … else` (edition 2024)

A let chain combines pattern matching and conditions in one `if`, which
removes the nested "pyramid" older Rust code has:

```rust
if let Ok(url) = Url::parse(word) && matches!(url.scheme(), "http" | "https") {
    urls.push(url);
} else {
    words.push(word); // not a URL, or the wrong scheme
}
```

`let … else` destructures or bails out early:

```rust
let Some(user) = msg.from.as_ref() else {
    return Ok(());
};
```

`matches!(x, "a" | "b")` is the idiomatic "does it fit one of these
patterns?" when you only need a `bool`.

## Bytes are not characters

`str::len()` counts **bytes**. UTF-8 uses 2 bytes for a Cyrillic letter and
up to 4 for an emoji. For a minimum text length, as in `extract` in
`src/fetch.rs`, count characters: `text.chars().count()`.

## `dedup` only removes neighbours

`Vec::dedup()` removes *consecutive* duplicates, so it works only after
`sort()`. To deduplicate and keep the original order, check before pushing:
`if !v.contains(&x) { v.push(x) }`. That is fine for a few items; use a
`HashSet` for many.
