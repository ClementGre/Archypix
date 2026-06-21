# Multi-stage build for Archypix Resolver
FROM rust:1.96-slim-trixie AS builder

WORKDIR /workspace

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Workspace skeleton — pre-compile dependencies before copying real source.
COPY Cargo.toml Cargo.lock ./
COPY common/Cargo.toml ./common/Cargo.toml
COPY back/Cargo.toml ./back/Cargo.toml
COPY resolver/Cargo.toml ./resolver/Cargo.toml
COPY worker/Cargo.toml ./worker/Cargo.toml

# Stub out every workspace member so cargo can resolve the dependency graph.
RUN for pkg in common back resolver worker; do \
      mkdir -p $pkg/src && echo 'fn main(){}' > $pkg/src/main.rs; \
    done && \
    touch common/src/lib.rs back/src/lib.rs resolver/src/lib.rs && \
    cargo build --release -p archypix-resolver && \
    rm -rf resolver/src

# Copy real source and rebuild only resolver (common/back/worker are not linked).
COPY resolver/src ./resolver/src
RUN find resolver/src -type f -exec touch {} + && cargo build --release -p archypix-resolver

# Runtime stage
FROM debian:trixie-slim AS prod
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /workspace/target/release/archypix-resolver /usr/local/bin/archypix-resolver

RUN useradd -m -u 1000 archypix
USER archypix
EXPOSE 80
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD addr="${LISTEN_ADDR:-0.0.0.0:80}"; curl -f http://localhost:${addr##*:}/health || exit 1
CMD ["archypix-resolver"]

# ---- dev (source mounted at runtime, cargo run on start) ----
# Usage: build with --target dev, mount ./src, Cargo.toml, Cargo.lock as volumes.
# Named volume on /workspace/target persists compiled deps across restarts.
FROM rust:1.96-slim-trixie AS dev
WORKDIR /workspace
RUN apt-get update && apt-get install -y pkg-config libssl-dev curl && rm -rf /var/lib/apt/lists/*
EXPOSE 80
HEALTHCHECK --interval=10s --timeout=3s --start-period=120s --retries=3 \
    CMD addr="${LISTEN_ADDR:-0.0.0.0:80}"; curl -f http://localhost:${addr##*:}/health || exit 1
CMD ["cargo", "run", "-p", "archypix-resolver"]
