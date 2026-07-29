# Configuration reference

Kemuri reads one YAML file. Set `version: 1` at the top of the file.

Kemuri rejects unknown fields. Use this command before a restart or reload:

```sh
kemuri config validate --config PATH
```

## Value formats

Durations use a number and a unit. Valid units include `ms`, `s`, `m`, `h`,
and `d`.

```yaml
timeout: 1500ms
interval: 30s
window: 5m
```

Percentages use a quoted percentage string:

```yaml
default_jitter: "10%"
warning_free: "10%"
```

## Top-level fields

| Field | Type | Default | Purpose |
|---|---|---|---|
| `version` | integer | required | Configuration format. The value must be `1`. |
| `server` | object | defaults apply | HTTP listener and shutdown settings |
| `logging` | object | defaults apply | Log level and format |
| `storage` | object | defaults apply | SQLite, retention, and disk pressure |
| `scheduler` | object | defaults apply | Scheduling and concurrency |
| `profiles` | list | empty | Reusable probe settings |
| `notifiers` | list | empty | Notification delivery |
| `rules` | list | empty | Alert rules |
| `targets` | list | empty | Targets and checks |

## Server

| Field | Type | Default |
|---|---|---|
| `server.bind` | IP address | `127.0.0.1` |
| `server.port` | integer from 1 through 65535 | `8080` |
| `server.cors` | Boolean | `false` |
| `server.public_url` | URL | not set |
| `server.shutdown_timeout` | positive duration | `30s` |

Kemuri has no built-in login. Bind it to a private address or put it behind a
trusted reverse proxy.

When CORS is active, Kemuri permits cross-origin `GET` and `HEAD` requests.
Cross-origin configuration reload is always blocked.

Set `public_url` when notification links must use a reverse-proxy URL.

## Logging

| Field | Values | Default |
|---|---|---|
| `logging.level` | tracing filter | `info` |
| `logging.format` | `plain` or `json` | `plain` |

Kemuri redacts secret values, sensitive HTTP headers, URL credentials, and URL
query values from its own stored errors and configuration snapshots.

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

A retention value is a positive duration or `forever`.

Kemuri pauses scheduling when free space is at or below `critical_free`. It
resumes scheduling only when free space is above `warning_free`.

## Scheduler

| Field | Values or type | Default |
|---|---|---|
| `scheduler.tick_interval` | positive duration | `1s` |
| `scheduler.max_concurrent` | positive integer | `64` |
| `scheduler.startup_mode` | `immediate_then_aligned` or `aligned` | `immediate_then_aligned` |
| `scheduler.default_jitter` | percentage | `"10%"` |
| `scheduler.max_concurrent_by_probe.icmp` | positive integer | not set |
| `scheduler.max_concurrent_by_probe.http` | positive integer | not set |
| `scheduler.max_concurrent_by_probe.tcp` | positive integer | not set |
| `scheduler.max_concurrent_by_probe.dns` | positive integer | not set |

The global concurrency limit applies to all checks. A probe-specific limit
also applies when it is set.

## Profiles

Each profile has these common fields:

| Field | Type | Default |
|---|---|---|
| `kind` | `icmp`, `http`, `tcp`, or `dns` | required |
| `id` | string | required |
| `interval` | positive duration | `30s` |
| `timeout` | positive duration | `5s` |

See [Probe settings](./probes) for fields that apply to each probe kind.

## Targets

A target has these fields:

| Field | Type | Default |
|---|---|---|
| `id` | string | required |
| `address` | string | required |
| `name` | string | target ID |
| `group_path` | string | not set |
| `labels` | map of strings | empty |
| `enabled` | Boolean | `true` |
| `checks` | list | empty |

A group path can contain nested segments. Use `/` between segments.

```yaml
group_path: production/europe
```

## Checks

A check has these fields:

| Field | Type | Default |
|---|---|---|
| `id` | string | required |
| `profile` | profile ID | required |
| `enabled` | Boolean | `true` |
| `kind` | probe kind | profile kind |
| `interval` | duration | profile value |
| `timeout` | duration | profile value |

A check can override each field for its probe kind.

Kemuri uses these merge rules:

- A scalar value replaces the profile value.
- HTTP headers merge by header name.
- A list replaces the profile list.

The check kind must match the profile kind.

## Secrets

HTTP bodies, webhook URLs, webhook header values, and SMTP passwords can use a
literal value, an environment variable, or a file.

Read a value from an environment variable:

```yaml
url:
  from_env: KEMURI_WEBHOOK_URL
```

Read a value from a file:

```yaml
password:
  from_file: /run/secrets/smtp-password
```

Use environment or file references for credentials. Restrict access to secret
files and process environment data.

## Disabled configuration

Set `enabled: false` on a target or check to stop scheduling it. Kemuri keeps
its stored history.

Kemuri resolves only enabled targets and checks. Invalid values in an enabled
entry stop startup or reload.

## Complete example

The repository contains a
[sample configuration](https://github.com/stianfro/kemuri/blob/main/packaging/kemuri.yaml).
