FROM rust:1.75-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/sase-payments /app/sase-payments
COPY --from=builder /app/migrations /app/migrations
RUN useradd -r -s /bin/false appuser
USER appuser
ENV RUST_LOG=info PORT=8084
EXPOSE 8084
ENTRYPOINT ["/app/sase-payments"]
