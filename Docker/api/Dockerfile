# syntax=docker/dockerfile:1
# Buduje tylko binarkę clmm-lp-api z workspace (wolny pierwszy build — normalne w Rust).
FROM rust:1-bookworm AS builder
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml ./
COPY crates ./crates

# Pierwszy build może trwać długo (pobranie zależności Solana/Orca). Kolejne warstwy cacheują cargo.
RUN cargo build --release -p clmm-lp-api

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/clmm-lp-api /usr/local/bin/clmm-lp-api

ENV RUST_LOG=info
EXPOSE 8080
CMD ["clmm-lp-api"]
