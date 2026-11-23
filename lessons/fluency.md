# Becoming fluent

Knowing the borrow checker is half of writing Rust. The other half is
vocabulary: knowing that a method or crate already does what you are
about to write by hand. `truncate_chars` in `src/llm/prompts.rs` is one
call to `Iterator::nth`, not a counting loop.

## Rust is not Go here

Go keeps its standard library small and treats the `for` loop as the
idiom. Rust goes the other way: `Iterator` alone has about 75 provided
methods, and reviewers expect `iter().position(…)` over a loop that
tracks an index. Learning the vocabulary is part of writing idiomatic
code, not just a way to type less.

## Where to spend time

1. **Read a few standard-library pages once, top to bottom.** These cover
   most everyday code:
   [`Iterator`](https://doc.rust-lang.org/std/iter/trait.Iterator.html),
   [`Option`](https://doc.rust-lang.org/std/option/enum.Option.html),
   [`Result`](https://doc.rust-lang.org/std/result/enum.Result.html),
   [`str`](https://doc.rust-lang.org/std/primitive.str.html),
   [`Vec`](https://doc.rust-lang.org/std/vec/struct.Vec.html) and
   [slices](https://doc.rust-lang.org/std/primitive.slice.html),
   [`HashMap`](https://doc.rust-lang.org/std/collections/struct.HashMap.html).
   Skim the method list in the sidebar so you remember what exists.
2. **Let clippy teach you.** Run `cargo clippy -- -W clippy::pedantic`
   for a while. Many of its warnings say "there is a method for this".
   Keep the lessons, then switch the group off.
3. **Write the loop, then look for the method.** A working loop is a
   fine first draft. Then ask whether `Iterator` already does it. You
   remember a method better after writing it out by hand once.
4. **Read other people's code.** Exercism's Rust track shows community
   solutions after you solve an exercise.

## Most of the value is in a few traits

`Iterator`, `Option` and `Result` are combinator APIs. With `map`,
`filter`, `and_then`, `ok_or`, `unwrap_or_else` and `collect` you can
read most Rust code; the other methods are variations on those.

## Finding crates

| Resource | What it is |
| -------- | ---------- |
| [blessed.rs](https://blessed.rs/crates) | Curated list of the crates everyone uses, by task |
| [lib.rs](https://lib.rs) | Index of crates.io with better categories and rankings |
| [*Effective Rust*](https://effective-rust.com) | Short free book of idioms, in the style of *Effective Go* |

To see why a crate is in your build, run `cargo tree -i <crate>`.
