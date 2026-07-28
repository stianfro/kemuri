# Kemuri

Latency monitoring with smoke-style graphs.

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

## License

MIT
