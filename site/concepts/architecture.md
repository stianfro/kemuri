# Architecture

Kemuri runs as one process on one Linux host. One process owns the SQLite
database.

## Runtime components

```text
Configuration file
        |
        v
Resolver and validator
        |
        v
Scheduler -> Probe workers -> Storage writer -> SQLite
     |                              |
     |                              +-> Rollups and retention
     |
     +-> Alert evaluator -> Notification worker

SQLite -> HTTP API -> Web UI
             |
             +-> SSE and Prometheus metrics
```

The lifecycle controller supervises these components:

- scheduler
- probe workers
- storage writer
- alert evaluator
- notification worker
- rollup worker
- retention worker
- HTTP server

An unexpected component exit is fatal. Kemuri stops new work, drains active
rounds, flushes storage, and returns a nonzero process exit code.

## Configuration state

Kemuri resolves the full configuration before it starts. The resolved state
contains typed probe settings, loaded certificate files, effective secrets,
and notifier clients.

Kemuri computes stable hashes from canonical configuration data. A check
revision changes when its effective settings or secret values change. Kemuri
does not store the effective secret values.

## Storage path

Probe workers send completed rounds to one storage writer. The writer stores
the configuration generation and check revision with each round.

Rollup workers create fixed five-minute and one-hour buckets. API queries use a
raw round when the matching rollup bucket is not complete.

## HTTP path

The HTTP server provides:

- the web UI
- the version 1 JSON API
- an OpenAPI document
- server-sent events
- liveness and readiness endpoints
- Prometheus metrics

The web UI files are in the Kemuri binary.

## Failure behavior

Kemuri marks itself not ready when a required dependency is not available.
Disk pressure can pause only the scheduler. The UI, health endpoints, reload,
metrics, retention, and database access stay available during the pause.
