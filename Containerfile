# Build and runtime both use UBI 10, so the binary links against the
# same glibc it runs with. Rust comes from rustup, pinned to the version
# in CLAUDE.md and CI.
FROM registry.access.redhat.com/ubi10/ubi AS build
ARG RUST_VERSION=1.100.0
# gcc and make build bundled SQLite and aws-lc (rustls crypto).
RUN dnf install -y --setopt=install_weak_deps=0 gcc make && dnf clean all
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain "${RUST_VERSION}"
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
COPY --from=build /usr/local/bin/reading-list /usr/local/bin/reading-list
# A named volume copies this directory's ownership on first use.
RUN mkdir /data && chown 1001:0 /data
ENV DATABASE_PATH=/data/reading-list.db \
    MCP_LISTEN_ADDR=0.0.0.0:8787
USER 1001
VOLUME /data
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/reading-list"]
