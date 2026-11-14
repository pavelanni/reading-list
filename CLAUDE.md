# Reading list

Telegram bot inbox, story-based summaries via OpenRouter, MCP server for
reading and triage. Design: `docs/dev/2026-11-01-reading-list-design.md`.
Plan: `docs/dev/2026-11-01-reading-list-plan.md`.

## Owner context

Pavel is learning Rust with this project. Explain Rust-specific choices
briefly when writing code, and leave marked learning hooks to him.

## Toolchain

| Tool | Version | Where pinned |
| ---- | ------- | ------------ |
| Rust | 1.100.0 (edition 2024) | `.github/workflows/ci.yml` `RUST_VERSION` |
| `actions/checkout` | v7 | CI |
| `Swatinem/rust-cache` | v2 | CI |

When upgrading local Rust, update `RUST_VERSION` in CI in the same commit.

## Commands

| Task | Command |
| ---- | ------- |
| Format check | `cargo fmt --check` |
| Lint | `cargo clippy --all-targets -- -D warnings` |
| Test | `cargo test` |
| Markdown lint | `markdownlint-cli2 "**/*.md" "#target" "#.superpowers"` |

Run all four before every commit.

## Conventions

- Secrets only from environment variables; never log them (`Config`
  has a redacting `Debug`).
- Pure logic separated from I/O so it is testable without network.
