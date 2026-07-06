# ---- production builder ----
# Build context must be the repo root (workspace: back/ + common/).
FROM rust:1.96-slim-trixie AS builder
WORKDIR /workspace
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY common/Cargo.toml ./common/Cargo.toml
COPY back/Cargo.toml ./back/Cargo.toml
COPY resolver/Cargo.toml ./resolver/Cargo.toml
COPY worker/Cargo.toml ./worker/Cargo.toml

ENV SQLX_OFFLINE=true
RUN for pkg in common back resolver worker; do \
      mkdir -p $pkg/src && echo 'fn main(){}' > $pkg/src/main.rs; \
    done && \
    touch common/src/lib.rs worker/src/lib.rs resolver/src/lib.rs && \
    cargo build --release -p archypix-back && \
    rm -rf common/src back/src

COPY back/.sqlx ./back/.sqlx
COPY back/migrations ./back/migrations
COPY back/src ./back/src
COPY common/src ./common/src
RUN find common/src back/src -type f -exec touch {} + && cargo build --release -p archypix-back

# ---- production runtime ----
FROM debian:trixie-slim AS prod
WORKDIR /app
RUN apt-get update && apt-get install -y ca-certificates libssl3 curl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /workspace/target/release/archypix-back /usr/local/bin/archypix-back
RUN useradd -m -u 1000 archypix && chown -R archypix:archypix /app
USER archypix
EXPOSE 80
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD addr="${LISTEN_ADDR:-0.0.0.0:80}"; curl -f http://localhost:${addr##*:}/health || exit 1
CMD ["archypix-back"]

# ---- dev (source mounted at runtime, cargo run on start) ----
# Build context: repo root. Mount back/src, common/src, and Cargo files as volumes.
# Named volume on /workspace/target persists compiled deps across restarts.
FROM rust:1.96-slim-trixie AS dev
WORKDIR /workspace
RUN apt-get update && apt-get install -y pkg-config libssl-dev curl && rm -rf /var/lib/apt/lists/*
EXPOSE 80
HEALTHCHECK --interval=10s --timeout=3s --start-period=120s --retries=3 \
    CMD addr="${LISTEN_ADDR:-0.0.0.0:80}"; curl -f http://localhost:${addr##*:}/health || exit 1
CMD ["cargo", "run", "-p", "archypix-back"]
