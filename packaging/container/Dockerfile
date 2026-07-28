FROM rust:1-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p kemuri-cli

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
RUN useradd -r -s /bin/false kemuri
COPY --from=builder /app/target/release/kemuri /usr/local/bin/kemuri
USER kemuri
EXPOSE 8080
VOLUME ["/etc/kemuri", "/var/lib/kemuri"]
ENTRYPOINT ["kemuri"]
CMD ["serve", "--config", "/etc/kemuri/kemuri.yaml"]
