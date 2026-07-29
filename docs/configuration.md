# Configuration version 1

Kemuri rejects unknown fields. Run `kemuri config validate --config PATH` before a restart or reload. Durations use units such as `ms`, `s`, `m`, `h`, and `d`. Percentages use a quoted value such as `"10%"`.

## Top-level fields

| Field | Type | Default | Purpose |
|---|---|---|---|
| `version` | integer | required, must be `1` | Configuration format |
| `server` | object | defaults below | HTTP service |
| `logging` | object | defaults below | Process logging |
| `storage` | object | defaults below | SQLite, retention, and disk limits |
| `scheduler` | object | defaults below | Check dispatch |
| `profiles` | list | empty | Reusable probe settings |
| `notifiers` | list | empty | Alert delivery |
| `rules` | list | empty | Alert rules |
| `targets` | list | empty | Monitored targets and checks |

## Server and logging

| Field | Type | Default or values |
|---|---|---|
| `server.bind` | IP address | `127.0.0.1` |
| `server.port` | integer 1 to 65535 | `8080` |
| `server.cors` | boolean | `false` |
| `server.public_url` | URL | unset |
| `server.shutdown_timeout` | positive duration | `30s` |
| `logging.level` | filter string | `info` |
| `logging.format` | `plain` or `json` | `plain` |

CORS permits cross-origin `GET` and `HEAD` only. Reload is always same-origin and requires JSON. Kemuri has no login. Bind to a private address or use a trusted reverse proxy.

## Storage

| Field | Default |
|---|---|
| `storage.path` | `kemuri.db` |
| `storage.retention.raw_rounds` | `7d` |
| `storage.retention.rollup_5m` | `90d` |
| `storage.retention.rollup_1h` | `forever` |
| `storage.retention.alert_events` | `30d` |
| `storage.retention.notification_records` | `30d` |
| `storage.disk_pressure.warning_free` | `"10%"` |
| `storage.disk_pressure.critical_free` | `"5%"` |

A retention value is a positive duration or `forever`. Kemuri pauses scheduling at or below `critical_free`. It resumes only above `warning_free`. The UI, health endpoints, metrics, retention, and reload remain available while scheduling is paused.

## Scheduler

| Field | Default or values |
|---|---|
| `scheduler.tick_interval` | `1s` |
| `scheduler.max_concurrent` | `64` |
| `scheduler.startup_mode` | `immediate_then_aligned` or `aligned` |
| `scheduler.default_jitter` | `"10%"` |
| `scheduler.max_concurrent_by_probe.icmp` | unset |
| `scheduler.max_concurrent_by_probe.http` | unset |
| `scheduler.max_concurrent_by_probe.tcp` | unset |
| `scheduler.max_concurrent_by_probe.dns` | unset |

All concurrency limits must be greater than zero. An immediate startup round is followed by interval-aligned slots. Jitter is stable for a target and check. Overlap and backpressure slots become no-data rounds rather than delayed work.

## Profiles

Every profile has `kind`, `id`, `interval` (default `30s`), and `timeout` (default `5s`).

### ICMP

Fields: `count` (default `3`), `address_family` (`auto`, `ipv4`, or `ipv6`), `payload_size`, and `source_address`. The source address must match the selected family. Linux needs ping socket permission or `CAP_NET_RAW`.

### HTTP

Fields: `url`, `method`, `headers`, `body`, `expected_status`, `follow_redirects`, `max_redirect_count`, `connection_mode`, `measure_until`, `user_agent`, `tls_validate`, and `root_certificates`.

`expected_status` is one integer from 100 through 599 or a quoted range such as `"200-399"`. `connection_mode` is `pooled`, `per_round`, or `fresh`. `measure_until` is `headers` or `body`. `root_certificates` replaces the inherited list and contains PEM file paths.

### TCP

Fields: `host`, `port`, `address_family`, `source_address`, and `tls`. TLS fields are `enabled`, `server_name`, `tls_validate`, and `root_certificates`. A successful TLS check includes the handshake in measured latency.

### DNS

Fields: `name`, `server`, `record_type`, `protocol`, `expected_rcode`, and `require_answer`. `protocol` is `udp` or `tcp`. Supported response codes are `noerror`, `formerr`, `servfail`, `nxdomain`, `notimp`, and `refused`. `domain` and `resolver` remain accepted aliases for `name` and `server`.

## Targets and checks

A target has `id`, `address`, optional `name`, optional nested `group_path`, optional string `labels`, `enabled` (default `true`), and `checks`.

A check has `id`, `profile`, and `enabled` (default `true`). It can override `interval`, `timeout`, and every field of its profile kind. It can also specify `kind`, but the kind must match the profile. Scalar values replace profile values. HTTP headers merge by header name. Lists replace profile lists. Disabled targets and checks keep history but are not resolved or scheduled.

## Secrets

HTTP bodies, webhook URLs, webhook header values, and SMTP passwords accept a literal string or one of these objects:

```yaml
from_env: VARIABLE_NAME
```

```yaml
from_file: /run/secrets/name
```

Kemuri hashes effective secret values for revision identity. It does not store or log those values. Prefer environment or file references.

## Notifiers

Webhook fields: `kind: webhook`, `id`, secret `url`, optional secret `headers`, and `timeout` (default `10s`).

SMTP fields: `kind: smtp`, `id`, `host`, `port`, `from`, `to`, optional `username`, optional secret `password`, `tls_mode` (default `required`), and `timeout` (default `30s`).

## Alert rules

Fields: `id`, `profile`, `metric`, `operator`, `threshold`, `window`, `notifier`, `duration`, `clear_threshold`, `clear_operator`, `repeat_every`, `minimum_rounds`, `minimum_latency_samples`, and `no_data_period`.

A rule applies only to checks that use its profile. Supported operators are `gt`, `gte`, `lt`, and `lte`. Metrics include response and measurement-loss ratios, latency values, and consecutive failure counts accepted by validation. `minimum_rounds` and `minimum_latency_samples` prevent evaluation with too little data. Without `clear_threshold`, clear is the exact inverse of fire at the firing threshold. Hysteresis applies only when `clear_threshold` is set. Removing a rule or check resolves its active state with reason `config_removed`, without a removed notifier.

## Reload

Send `SIGHUP` or make an `application/json` `POST` to `/api/v1/config/reload`. Reload requests are serialized. Kemuri parses and resolves the full new configuration, initializes notifiers, and reconciles SQLite before it switches active runtime state. A failed reload keeps the prior active configuration. The HTTP server remains active across repeated reloads.
