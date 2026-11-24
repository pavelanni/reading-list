# Build and runtime both use UBI 10, so the binary links against the
# same glibc it runs with. Rust comes from rustup, pinned to the version
# in CLAUDE.md and CI.
FROM registry.access.redhat.com/ubi10/ubi AS build
ARG RUST_VERSION=1.100.0
# rustup-init is pinned and checked against hashes recorded here, not
# fetched at build time. Bumping the version means updating both hashes
# (from …/rustup-init.sha256 next to each binary).
ARG RUSTUP_VERSION=1.29.3
# gcc and make build bundled SQLite and aws-lc (rustls crypto).
RUN dnf install -y --setopt=install_weak_deps=0 gcc make && dnf clean all
RUN arch="$(uname -m)" \
    && case "$arch" in \
         x86_64)  sha=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55 ;; \
         aarch64) sha=9f608c142e0e38a78957d0661ba1dfaf0b6fb37184cfaa4b9a0aa57af6184282 ;; \
         *) echo "unsupported architecture: $arch" >&2; exit 1 ;; \
       esac \
    && curl --proto '=https' --tlsv1.2 -sSfo /tmp/rustup-init \
       "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${arch}-unknown-linux-gnu/rustup-init" \
    && echo "${sha}  /tmp/rustup-init" | sha256sum -c - \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --no-modify-path --profile minimal \
       --default-toolchain "${RUST_VERSION}" \
    && rm /tmp/rustup-init
ENV PATH=/root/.cargo/bin:$PATH
WORKDIR /src
COPY . .
# Queries are checked against the committed .sqlx/ cache, not a database.
ENV SQLX_OFFLINE=true
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
    && cp target/release/reading-list /usr/local/bin/reading-list

FROM registry.access.redhat.com/ubi10/ubi-minimal
# Labels are inherited from the base image, so UBI's name, vendor,
# version and revision would describe this image unless overwritten.
# CI passes VERSION and REVISION; the release workflow also sets
# org.opencontainers.image.created.
ARG VERSION=dev
ARG REVISION=unknown
LABEL name="reading-list" \
      summary="Telegram reading inbox with story summaries and an MCP server" \
      description="Saves links sent to a Telegram bot, summarizes them into stories via OpenRouter, and serves them to agents over MCP." \
      vendor="Pavel Anni" \
      maintainer="Pavel Anni" \
      url="https://github.com/pavelanni/reading-list" \
      version="${VERSION}" \
      release="" \
      vcs-ref="${REVISION}" \
      io.k8s.display-name="reading-list" \
      io.k8s.description="Telegram reading inbox with story summaries and an MCP server" \
      io.openshift.tags="reading-list,mcp,telegram" \
      org.opencontainers.image.title="reading-list" \
      org.opencontainers.image.description="Telegram reading inbox with story summaries and an MCP server" \
      org.opencontainers.image.source="https://github.com/pavelanni/reading-list" \
      org.opencontainers.image.url="https://github.com/pavelanni/reading-list" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"
COPY --from=build /usr/local/bin/reading-list /usr/local/bin/reading-list
# A named volume copies this directory's ownership on first use.
RUN mkdir /data && chown 1001:0 /data
ENV DATABASE_PATH=/data/reading-list.db \
    MCP_LISTEN_ADDR=0.0.0.0:8787
USER 1001
VOLUME /data
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/reading-list"]
