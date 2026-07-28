# Changelog

## 1.0.0

- Add typed probe settings, strict version 1 configuration validation, and stable revisions
- Fix scheduler cadence, startup rounds, concurrency limits, reloads, and graceful shutdown
- Add disk pressure controls, readiness checks, migration-backed runtime state, and safer retention
- Add millisecond and microsecond API units, cursor validation, OpenAPI, group pages, and graph overlays
- Embed the production web bundle and ship the `kemuri` binary
- Add Linux release archives, multi-architecture container publishing, and local usage gates

## 0.1.0

Initial release.

- ICMP, HTTP, TCP, and DNS probe types
- YAML configuration with profile resolution and check overrides
- SQLite storage with forward-only migrations
- Smoke-style latency graphs in the web UI
- Alert evaluation with pending/firing/resolved state machine
- Webhook and SMTP notification delivery with retry
- Prometheus metrics endpoint
- SSE event stream for real-time updates
- SIGHUP configuration reload
- CLI commands: serve, version, config validate, doctor, check, database backup, notify test
