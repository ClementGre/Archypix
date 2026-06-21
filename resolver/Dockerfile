# Multi-stage build for Archypix Resolver
FROM rust:1.96-slim AS builder

WORKDIR /workspace

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Create a dummy src to cache dependencies
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Copy actual source code
COPY src ./src

# Build the real application
RUN touch src/main.rs && cargo build --release

# Runtime stage
FROM debian:bookworm-slim AS prod

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy binary from builder
COPY --from=builder /workspace/target/release/archypix-resolver /usr/local/bin/archypix-resolver

# Create non-root user
RUN useradd -m -u 1000 archypix && \
    chown -R archypix:archypix /app

USER archypix

# Expose port
EXPOSE 80
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD addr="${LISTEN_ADDR:-0.0.0.0:80}"; curl -f http://localhost:${addr##*:}/health || exit 1
CMD ["archypix-resolver"]

# ---- dev (source mounted at runtime, cargo run on start) ----
# Usage: build with --target dev, mount ./src, Cargo.toml, Cargo.lock as volumes.
# Named volume on /workspace/target persists compiled deps across restarts.
FROM rust:1.96-slim AS dev
WORKDIR /workspace
RUN apt-get update && apt-get install -y pkg-config libssl-dev curl && rm -rf /var/lib/apt/lists/*
EXPOSE 80
HEALTHCHECK --interval=10s --timeout=3s --start-period=120s --retries=3 \
    CMD addr="${LISTEN_ADDR:-0.0.0.0:80}"; curl -f http://localhost:${addr##*:}/health || exit 1
CMD ["cargo", "run", "-p", "archypix-resolver"]
