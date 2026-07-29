# Kemuri

Latency monitoring with smoke-style graphs.

Documentation: [stianfro.github.io/kemuri](https://stianfro.github.io/kemuri/)

## Quick Start

Build:

```sh
cargo build --release
```

Configure (`kemuri.yaml`):

```yaml
version: 1

profiles:
  - kind: http
    id: http-default
    url: http://example.com/health
    interval: 30s
    timeout: 5s

notifiers:
  - kind: webhook
    id: slack
    url:
      from_env: SLACK_WEBHOOK_URL

rules:
  - id: high-loss
    profile: http-default
    metric: measurement_loss_ratio
    operator: gte
    threshold: "10%"
    window: 5m
    notifier: slack

targets:
  - id: web-1
    address: web1.example.com
    checks:
      - id: health
        profile: http-default
```

Run:

```sh
kemuri serve --config ./kemuri.yaml
```

Open `http://localhost:8080` for the web UI.

## Install a release

Release archives are available for Linux, macOS, and Windows. The shell and
PowerShell installers select the correct archive and verify its SHA-256
checksum.

Linux and macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/stianfro/kemuri/releases/latest/download/kemuri-installer.sh | sh
```

Windows PowerShell:

```powershell
irm https://github.com/stianfro/kemuri/releases/latest/download/kemuri-installer.ps1 | iex
```

Version tags also publish a signed-provenance multi-platform OCI image at
`ghcr.io/stianfro/kemuri`. The release archives contain the sample
configuration and systemd unit. GitHub Releases also contains a source archive
and SHA-256 checksum files.

## CLI Commands

| Command | Description |
|---------|-------------|
| `kemuri serve --config <path>` | Start the server |
| `kemuri version` | Print version info |
| `kemuri config validate --config <path>` | Validate configuration |
| `kemuri doctor --config <path>` | Check subsystems |
| `kemuri check <target_id>/<check_id> --config <path>` | Run one check immediately |
| `kemuri database backup --output <path>` | Create database backup |
| `kemuri notify test <notifier_id> --config <path>` | Send test notification |

## Probe Types

- **ICMP**: Ping checks (requires CAP_NET_RAW or ping group)
- **HTTP**: HTTP/HTTPS health checks
- **TCP**: TCP connection checks
- **DNS**: DNS resolution checks

## Configuration

See the `version: 1` schema. All probe profiles, notifiers, rules, and targets are defined in a single YAML file.

Secret values support `from_env` and `from_file` references to avoid storing credentials in the config file.

Targets and checks can set `enabled: false`. Disabled entries remain in SQLite history, but Kemuri does not schedule them. Check values override profile values. HTTP headers merge by name. List values replace the profile list.

The scheduler starts each enabled check at process startup by default, then uses interval-aligned slots with deterministic jitter. Set `startup_mode: aligned` to omit the startup round. `max_concurrent` is the global round limit. Optional limits under `max_concurrent_by_probe` apply to `icmp`, `http`, `tcp`, and `dns`.

Disk pressure uses hysteresis. Scheduling pauses when free space reaches `storage.disk_pressure.critical_free`. It resumes only after free space exceeds `warning_free`. The defaults are 5 percent and 10 percent.

HTTP checks accept one `expected_status` integer or a range such as `"200-399"`. ICMP checks require Linux ping socket permission or `CAP_NET_RAW`.

Alert states are evaluated only for checks that use the rule profile. A rule can require `minimum_rounds` and `minimum_latency_samples`. Without `clear_threshold`, the clear condition is the logical inverse of the firing condition.

Send `SIGHUP`, or make a same-origin JSON `POST` to `/api/v1/config/reload`, to reload the file. Kemuri validates the new state before it replaces the active state. A failed reload leaves the active configuration in place. Cross-origin reload requests are rejected.

For the complete field reference, see the
[configuration documentation](https://stianfro.github.io/kemuri/reference/configuration).

## API

All endpoints are under `/api/v1/`. Key endpoints:

- `GET /api/v1/targets` - list targets
- `GET /api/v1/targets/{id}` - target detail
- `GET /api/v1/targets/{id}/checks/{id}/series` - time series
- `GET /api/v1/alerts` - alert states
- `GET /api/v1/info` - version info
- `GET /api/v1/system/status` - system status
- `GET /healthz` - liveness
- `GET /readyz` - readiness
- `GET /metrics` - Prometheus metrics
- `GET /api/openapi.json` - OpenAPI document

API timestamps use Unix milliseconds and latency values use integer microseconds. Range queries use `from_ms` and `to_ms`. Collection limits are from 1 through 200. Cursor values are opaque.

API errors contain a request ID. The same value is returned in `X-Request-ID`. Unknown API routes return JSON. Missing static assets return a normal HTTP 404.

For the full contract and unit rules, see the
[HTTP API documentation](https://stianfro.github.io/kemuri/reference/api).

## Backups

Use `kemuri database backup --config <path> --output <file>` while the service is running. Use `--output -` to write a complete SQLite database image to standard output. Store backups outside the active data directory and test them with `PRAGMA integrity_check`.

## Reverse proxy

Kemuri has no built-in authentication. Run it on a trusted host or behind a trusted reverse proxy. Keep CORS disabled unless another origin must read the API. Configure `server.public_url` when notification links must use the proxy URL. The proxy must support SSE without response buffering for `/api/v1/events`.

## Containers

The image runs as the non-root `kemuri` user and stores data in `/var/lib/kemuri`. Grant ICMP capability only when ICMP profiles are configured:

```sh
docker run --cap-add NET_RAW ...
```

## License

MIT
