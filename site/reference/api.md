# HTTP API version 1

The JSON API is under `/api/v1`. The OpenAPI document is at
`/api/openapi.json`.

The repository also contains the tracked
[OpenAPI file](https://github.com/stianfro/kemuri/blob/main/openapi/openapi.json).

## Units

All API timestamps are Unix milliseconds. Timestamp field names use the `_ms`
suffix.

All latency values are integer microseconds. Latency field names use the `_us`
suffix.

Range queries use `from_ms` and `to_ms`.

## Pagination

Collection endpoints accept:

| Parameter | Rule |
|---|---|
| `limit` | Integer from 1 through 200 |
| `cursor` | Opaque value from the prior response |

Do not parse or change a cursor. Pass `next_cursor` to the next request.

## Main endpoints

| Method and path | Purpose |
|---|---|
| `GET /api/v1/info` | Version and build information |
| `GET /api/v1/system/status` | Runtime and dependency status |
| `GET /api/v1/groups` | Group list |
| `GET /api/v1/groups/{group_path}` | Nested group detail |
| `GET /api/v1/targets` | Target list |
| `GET /api/v1/targets/{target_id}` | Target detail |
| `GET /api/v1/targets/{target_id}/checks` | Check list |
| `GET /api/v1/targets/{target_id}/checks/{check_id}` | Check detail |
| `GET /api/v1/targets/{target_id}/checks/{check_id}/series` | Fixed time buckets |
| `GET /api/v1/targets/{target_id}/checks/{check_id}/rounds` | Round list |
| `GET /api/v1/alerts` | Alert state list |
| `GET /api/v1/alerts/{alert_id}` | Alert state detail |
| `GET /api/v1/alert-events` | Alert event list |
| `GET /api/v1/events` | Server-sent event stream |
| `POST /api/v1/config/reload` | Reload configuration |

## Series response

The series endpoint returns fixed time buckets. Each bucket has one state:

- `observed`
- `skipped`
- `missing`

The response also contains alert events and check revision markers. Clients can
use these objects as graph overlays.

Kemuri uses raw rounds where rollup coverage is not complete. It limits
concurrent series work so dashboard refreshes cannot use every SQLite
connection. Kemuri returns `503 series_busy` with `Retry-After` when a request
waits too long or needs too many raw rounds to fill a rollup gap. Retry the
request, or select a shorter range.

## Reload request

The reload endpoint accepts a same-origin JSON request:

```sh
curl --fail \
  -X POST \
  -H 'Content-Type: application/json' \
  --data '{}' \
  http://127.0.0.1:8080/api/v1/config/reload
```

Kemuri rejects a cross-origin reload request.

## Error format

API errors have this structure:

```json
{
  "code": "bad_request",
  "message": "limit must be from 1 through 200",
  "request_id": "opaque-request-id"
}
```

The response includes the same value in the `X-Request-ID` header.

Kemuri does not return SQL text or an internal error value. Unknown API routes
return a JSON 404 response.

## Health and metrics

| Path | Purpose |
|---|---|
| `/healthz` | Process liveness |
| `/readyz` | Dependency readiness |
| `/metrics` | Prometheus metrics |
| `/api/openapi.json` | OpenAPI document |

Readiness checks SQLite, required runtime tasks, writer availability, probe
capability, and disk state.

## CORS

CORS is off by default. When it is active, another origin can make read
requests with `GET` or `HEAD`.

Cross-origin reload is not permitted.

## Server-sent events

`GET /api/v1/events` reports current state changes. The stream is not a durable
event log. Read current resources again after a reconnect.
