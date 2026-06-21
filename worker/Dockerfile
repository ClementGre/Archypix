# ── Production builder ─────────────────────────────────────────────────────────
# Build context must be the repo root (contains both worker/ and common/).
FROM rust:1.96-slim-trixie AS builder
WORKDIR /workspace
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libclang-dev \
    libmagickwand-dev \
    libgexiv2-dev \
    && rm -rf /var/lib/apt/lists/*
ENV MAGICKCORE_QUANTUM_DEPTH=16 \
    MAGICKCORE_HDRI_ENABLE=1

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
    cargo build --release -p archypix-worker && \
    rm -rf common/src worker/src

# Copy real source and rebuild only worker + common (back/resolver are not linked).
COPY common/src ./common/src
COPY worker/src ./worker/src
RUN find common/src worker/src -type f -exec touch {} + && cargo build --release -p archypix-worker

# ── Production runtime ─────────────────────────────────────────────────────────
FROM debian:trixie-slim AS prod
RUN apt-get update && \
    MAGICKWAND_PKG=$(apt-cache pkgnames | grep -E '^libmagickwand-7\.' | sort -V | tail -n1) && \
    apt-get install -y \
    "$MAGICKWAND_PKG" \
    libgexiv2-2 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /workspace/target/release/archypix-worker /usr/local/bin/archypix-worker
RUN useradd -m -u 1000 archypix
USER archypix
EXPOSE 80
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD addr="${LISTEN_ADDR:-0.0.0.0:80}"; curl -f http://localhost:${addr##*:}/health || exit 1
CMD ["archypix-worker"]

# ── Development (source mounted at runtime, cargo run on start) ────────────────
# Build context: repo root. Mount worker/src, common/src, and Cargo files as volumes.
# Named volume on /workspace/target persists compiled deps across restarts.
FROM rust:1.96-slim-trixie AS dev
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libclang-dev \
    libmagickwand-dev \
    libgexiv2-dev \
    curl \
    && rm -rf /var/lib/apt/lists/*
ENV MAGICKCORE_QUANTUM_DEPTH=16 \
    MAGICKCORE_HDRI_ENABLE=1
WORKDIR /workspace
EXPOSE 80
HEALTHCHECK --interval=10s --timeout=3s --start-period=120s --retries=3 \
    CMD addr="${LISTEN_ADDR:-0.0.0.0:80}"; curl -f http://localhost:${addr##*:}/health || exit 1
CMD ["cargo", "run", "-p", "archypix-worker"]
