# syntax=docker/dockerfile:1
FROM rust:1.85-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations

RUN cargo build --release -p openbrain-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/openbrain /usr/local/bin/openbrain

EXPOSE 8080

CMD ["openbrain", "serve", "--bind", "0.0.0.0", "--port", "8080"]