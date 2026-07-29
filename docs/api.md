# HTTP API version 1

The OpenAPI document is at `/api/openapi.json`. Generated TypeScript types are tracked in `web/src/generated/api.ts`. `just api-check` fails when either artifact is stale.

All timestamps are Unix milliseconds and use an `_ms` suffix. Latencies are integer microseconds and use a `_us` suffix. Range queries use `from_ms` and `to_ms`. Collection endpoints accept `limit` from 1 through 200 and an opaque `cursor` returned as `next_cursor`.

The series endpoint returns fixed time buckets. Each bucket is `observed`, `skipped`, or `missing`. The response also contains alert events and check revision markers for graph overlays. Raw rounds are used where rollup coverage is not complete.

Errors have `code`, a safe `message`, and `request_id`. The same ID is in `X-Request-ID`. SQL and internal error text are not returned. Unknown API routes return JSON 404 responses. Missing UI assets return normal 404 responses.

Liveness is `/healthz`. Readiness is `/readyz` and checks SQLite, required runtime tasks, writer/probe capability, and disk pressure. Prometheus metrics are at `/metrics`. The SSE stream at `/api/v1/events` reports state changes. Clients must refetch after reconnect because the stream is not a durable event log.
