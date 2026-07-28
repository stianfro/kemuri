# Changelog

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
