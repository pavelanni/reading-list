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
| Rust | 1.100.0 (edition 2024) | `.github/workflows/ci.yml` and `Containerfile` `RUST_VERSION` |
| GitHub runner | `ubuntu-26.04` (`-arm` for arm64 builds) | CI, release; `.github/actionlint.yaml` until actionlint knows the labels |
| `actions/checkout` | v7 | CI |
| `Swatinem/rust-cache` | v2 | CI |
| `docker/metadata-action` | v6.2.0 | release (tags, labels only) |
| `redhat-actions/buildah-build` | v3.1.0 | release |
| `redhat-actions/push-to-registry` | v3.0.0 | release |
| `sigstore/cosign-installer` | v4.1.4 | release |
| `actions/attest-build-provenance` | v4.2.2 | release |
| rustup | 1.29.3 | `Containerfile` `RUSTUP_VERSION` + sha256 per arch |

When upgrading local Rust, update `RUST_VERSION` in CI and the
`Containerfile` in the same commit.

## Commands

| Task | Command |
| ---- | ------- |
| Format check | `cargo fmt --check` |
| Lint | `cargo clippy --all-targets -- -D warnings` |
| Test | `cargo test` |
| Markdown lint | `markdownlint-cli2 "**/*.md" "#target" "#.superpowers"` |
| Container image | `podman build -t reading-list .` |

Run the first four before every commit.

## Crates

| Crate | Version | Notes |
| ----- | ------- | ----- |
| `sqlx` | 0.9.0 | `sqlite` (bundled, FTS5), `macros`, `migrate`, `chrono` |
| `serde` | 1.0 | `derive` |
| `serde_json` | 1.0 | model responses |
| `chrono` | 0.4 | `clock`, `serde` |
| `url` | 2.5 | |
| `reqwest` | 0.13 | `rustls` (aws-lc-rs backend, needs a C toolchain), `json` |
| `teloxide` | 0.17 | `rustls` only; brings its own reqwest 0.12 |
| `thiserror` | 2.0 | typed errors (`FetchError`) |
| `dom_smoothie` | 0.19 | Readability extraction |
| `rmcp` | 3.11 | `server`, `macros`, `transport-streamable-http-server`; re-exports `schemars` |
| `axum` | 0.8 | HTTP host for MCP, bearer middleware |
| `subtle` | 2.6 | constant-time token compare |
| `tokio` | 1.54 | `macros`, `rt-multi-thread`, `signal`, `time` |
| `tokio-util` | 0.7 | `CancellationToken` for shutdown |
| `anyhow` | 1.0 | application errors |
| `tracing`, `tracing-subscriber` | 0.1, 0.3 | `env-filter`; `RUST_LOG` |
| `wiremock` | 0.6 | dev: HTTP mocks |

## Deployment

`Containerfile` builds on UBI 10 with rustup pinned to `RUST_VERSION`
(keep it in step with CI); runtime is `ubi10/ubi-minimal`, user 1001.
Quadlet unit: `deploy/reading-list.container`. `main` drains services
for 8 s after SIGTERM, then gives blocking tasks 1 s; keep the unit's
`StopTimeout` (10 s) above the sum.

## Releasing

1. PR that bumps `version` in `Cargo.toml` (and `Cargo.lock`); merge.
2. `git tag -s vX.Y.Z && git push origin vX.Y.Z` on the merged `main`.

`release.yml` fails unless the tag equals the `Cargo.toml` version. It
builds each arch on a native runner (no QEMU), pushes `X.Y.Z-amd64` and
`X.Y.Z-arm64`, merges them into the multi-arch tags with `buildah
manifest`, signs with cosign (keyless), attests provenance and creates
the GitHub Release with generated notes. Tags with `-` are pre-releases.

## Database and sqlx

SQL lives only in `src/db/`; schema in `migrations/`. Queries use the
compile-time checked `query!` macros. Without `DATABASE_URL` they read
the committed `.sqlx/` cache (that is how CI builds).

After changing a query or migration:

1. `echo 'DATABASE_URL=sqlite:dev.db' > .env` (gitignored, once)
2. `sqlx database reset -y` (recreate `dev.db` from migrations)
3. `cargo sqlx prepare -- --all-targets` and commit `.sqlx/`

## Conventions

- Secrets only from environment variables; never log them (`Config`
  has a redacting `Debug`).
- Pure logic separated from I/O so it is testable without network.
- When explaining a Rust gotcha or practice, add it to the matching file
  in `lessons/` (index in `lessons/README.md`).
