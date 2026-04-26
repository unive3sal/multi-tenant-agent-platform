FROM rust:latest AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin platform

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/platform /usr/local/bin/platform
COPY --from=builder /app/server/migrations /app/server/migrations
ENV BIND_ADDR=0.0.0.0:3000
EXPOSE 3000
CMD ["platform"]
