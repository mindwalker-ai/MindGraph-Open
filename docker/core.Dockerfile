# syntax=docker/dockerfile:1
# Builds the `mindgraph-core` server (crates/mindgraph-api) and the `mindgraph`
# CLI (crates/mindgraph-cli), whose `mindgraph demo` loads the demo scenarios
# through the REST API (compose service `demo`). Build context: repo root.

FROM rust:1.99-slim-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
        -p mindgraph-api --bin mindgraph-core \
        -p mindgraph-cli --bin mindgraph \
    && cp target/release/mindgraph-core target/release/mindgraph /usr/local/bin/

FROM debian:bookworm-slim AS runtime
# curl is only used by the container health check.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home mindgraph \
    && mkdir /data && chown mindgraph:mindgraph /data
COPY --from=builder /usr/local/bin/mindgraph-core /usr/local/bin/mindgraph /usr/local/bin/
USER mindgraph
# Uploaded source files (M4). Mount a volume here to keep them.
ENV MINDGRAPH_CORE_BIND_ADDR=0.0.0.0:8080 \
    MINDGRAPH_DATA_DIR=/data
VOLUME ["/data"]
EXPOSE 8080
CMD ["mindgraph-core"]
