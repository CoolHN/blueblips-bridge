# ── Build stage ──────────────────────────────────────────────
FROM rust:1.75-slim as builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libasound2-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy and build
COPY Cargo.toml .
COPY src/ src/

RUN cargo build --release

# ── Runtime stage ─────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libssl3 \
    libasound2 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/blueblips-bridge .

# HTTP port
EXPOSE 8080
# Spotify AP protocol port
EXPOSE 4070

ENV RUST_LOG=info

CMD ["./blueblips-bridge"]
