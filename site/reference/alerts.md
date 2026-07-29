# Alerts and notifiers

An alert rule applies to checks that use one named profile. Kemuri evaluates
the same rule for each matching check.

## Alert rule fields

| Field | Purpose |
|---|---|
| `id` | Stable rule ID |
| `profile` | Profile that selects checks |
| `metric` | Measured value |
| `operator` | `gt`, `gte`, `lt`, or `lte` |
| `threshold` | Firing threshold |
| `window` | Evaluation window |
| `notifier` | Notifier ID |
| `duration` | Required firing duration |
| `clear_threshold` | Optional clear threshold |
| `clear_operator` | Optional clear operator |
| `repeat_every` | Repeat notification interval |
| `minimum_rounds` | Required round count |
| `minimum_latency_samples` | Required latency sample count |
| `no_data_period` | Time before a no-data alert |

Kemuri validates the metric, operator, and threshold combination.

## Fire and clear behavior

Without `clear_threshold`, the clear condition is the exact logical inverse of
the firing condition at the firing threshold.

Set `clear_threshold` to use hysteresis. Hysteresis does not apply when this
field is absent.

`minimum_rounds` prevents evaluation with too few rounds.
`minimum_latency_samples` prevents latency evaluation with too few latency
samples.

When a rule or check is removed, Kemuri resolves its active alert with reason
`config_removed`. It does not send the resolution through a notifier that was
removed.

## Rule example

```yaml
rules:
  - id: http-loss
    profile: api-health
    metric: measurement_loss_ratio
    operator: gte
    threshold: "20%"
    clear_threshold: "5%"
    clear_operator: lte
    window: 5m
    duration: 2m
    repeat_every: 1h
    minimum_rounds: 5
    notifier: operations-webhook
```

## Webhook notifier

Webhook fields are:

| Field | Default |
|---|---|
| `kind` | must be `webhook` |
| `id` | required |
| `url` | required secret value |
| `headers` | empty |
| `timeout` | `10s` |

Example:

```yaml
notifiers:
  - kind: webhook
    id: operations-webhook
    url:
      from_env: KEMURI_WEBHOOK_URL
    headers:
      Authorization:
        from_env: KEMURI_WEBHOOK_AUTH
    timeout: 10s
```

## SMTP notifier

SMTP fields are:

| Field | Default |
|---|---|
| `kind` | must be `smtp` |
| `id` | required |
| `host` | required |
| `port` | required |
| `from` | required |
| `to` | required |
| `username` | not set |
| `password` | not set |
| `tls_mode` | `required` |
| `timeout` | `30s` |

Example:

```yaml
notifiers:
  - kind: smtp
    id: operations-mail
    host: smtp.example.com
    port: 587
    from: kemuri@example.com
    to:
      - operations@example.com
    username: kemuri
    password:
      from_file: /run/secrets/smtp-password
    tls_mode: required
```

## Test notifications

Test one notifier:

```sh
kemuri notify test operations-webhook --config ./kemuri.yaml
```

Test all notifiers as part of `doctor`:

```sh
kemuri doctor --config ./kemuri.yaml --test-notifiers
```

These commands send real messages.
