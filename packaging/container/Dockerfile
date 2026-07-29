FROM rust:1-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p kemuri

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates curl && rm -rf /var/lib/apt/lists/*
RUN useradd -r -s /bin/false kemuri \
    && install -d -o kemuri -g kemuri /var/lib/kemuri /etc/kemuri
COPY --from=builder /app/target/release/kemuri /usr/local/bin/kemuri
WORKDIR /var/lib/kemuri
USER kemuri
EXPOSE 8080
VOLUME ["/etc/kemuri", "/var/lib/kemuri"]
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:8080/healthz >/dev/null || exit 1
ENTRYPOINT ["kemuri"]
CMD ["serve", "--config", "/etc/kemuri/kemuri.yaml"]
