# Ownership and borrowing

Most borrow-checker errors come down to one question: **who owns this
value, and how long does it live?** A reference must never outlive the
value it points to.

## A clone that is dropped too early

Seen in: `normalize_url` in `src/ingest/normalize.rs`.

```rust
let mut pairs: Vec<(&str, &str)> = Vec::new();
for (key, value) in url.query_pairs() {
    let k = key.clone();            // k lives until the end of this iteration
    pairs.push((k.as_ref(), "…"));  // error: `k` does not live long enough
}
```

Cloning created a new owner, but the code then kept only a *reference* to
it and dropped the owner at the end of the loop body. The vector would hold
references to freed memory.

Fix: store the values themselves, not references.

- `Vec<(String, String)>` and `key.into_owned()`: simple, copies when needed.
- `Vec<(Cow<str>, Cow<str>)>` and push `key` directly: no copy, works because
  each `Cow` borrows from `url`, which outlives the function body.

Before writing `x.as_ref()` or `&x`, ask where `x` is dropped. If the answer
is "inside this block" and the reference escapes the block, it won't compile.

## `Cow`: borrowed or owned

`Cow<str>` ("clone on write") holds either a `&str` or a `String`.
`Url::query_pairs()` returns `Cow` because most query strings need no
decoding, so it borrows from the URL; only `%20`-style escapes force an
allocation. Cloning a borrowed `Cow` gives another borrowed `Cow`, which is
why `key.clone()` did not help above.

## Borrow first, then move

Seen in: `parse_message` in `src/ingest/parse.rs`.

```rust
let tag = word.to_lowercase();   // tag: String (owned)
if !tags.contains(&tag) {        // borrow: only looks at it
    tags.push(tag);              // move: the String goes into the Vec
}
```

`contains` takes a reference, `push` takes ownership. In this order it
compiles. Reversed, `tag` would no longer exist when you try to borrow it.

## Reading from one value while mutating it

Seen in: the trailing-slash rule in `normalize_url`.

```rust
let trimmed = out.path().trim_end_matches('/').to_owned();
out.set_path(&trimmed);
```

`out.path()` borrows `out`, and `trim_end_matches` returns a slice of that
borrow. `set_path` needs `&mut out`. You cannot hold a shared borrow while
taking a mutable one, so `.to_owned()` copies the trimmed path into its own
`String` first.

## The invisible double reference

Seen in: `impl From<reqwest::Error> for FetchError` in `src/fetch.rs`.

```rust
std::iter::successors(Some(&err as &dyn Error), |e| (*e).source())
```

`successors` passes the closure a *reference to* the previous item. The items
are `&dyn Error`, so `e` is `&&dyn Error`. Calling `e.source()` auto-derefs
through the outer, short-lived reference, and the result cannot outlive one
closure call (`lifetime may not live long enough`). `(*e).source()` borrows
from the inner `&dyn Error`, which lives as long as `err`.

When an error message shows `&'1 &T`, look for a double reference and
dereference once.

## `&&str` when iterating a slice of `&str`

Seen in: `is_tracking` in `src/ingest/normalize.rs`.

Iterating `&[&str]` yields `&&str`. Method calls auto-deref, so
`p.strip_suffix('*')` works. Comparison does not: `key == p` fails, and you
write `key == *p`.
