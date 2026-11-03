# syntax=docker/dockerfile:1
# Builds the `mindgraph-core` server (crates/mindgraph-api). Build context: repo root.

FROM rust:1.99-slim-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p mindgraph-api --bin mindgraph-core \
    && cp target/release/mindgraph-core /usr/local/bin/mindgraph-core

FROM debian:bookworm-slim AS runtime
# curl is only used by the container health check.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home mindgraph
COPY --from=builder /usr/local/bin/mindgraph-core /usr/local/bin/mindgraph-core
USER mindgraph
ENV MINDGRAPH_CORE_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
CMD ["mindgraph-core"]
