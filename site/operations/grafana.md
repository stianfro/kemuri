# Grafana

Kemuri includes two Grafana dashboards.

| Dashboard | Data source | Purpose |
|---|---|---|
| Kemuri service health | Prometheus | Monitor the Kemuri process and its runtime tasks |
| Kemuri check analysis | Infinity | Inspect one target and check |

The Infinity dashboard is optional. Kemuri does not require Grafana or the
Infinity plugin.

The dashboard and provisioning files are in
[`integrations/grafana`](https://github.com/stianfro/kemuri/tree/main/integrations/grafana).

## Prometheus dashboard

Configure Prometheus to scrape the Kemuri metrics endpoint:

```yaml
scrape_configs:
  - job_name: kemuri
    static_configs:
      - targets:
          - kemuri.example.com:8080
```

Import `kemuri-service-health.json`. Select the Prometheus data source.

This dashboard shows:

- Active, queued, and running checks.
- Scheduled round and probe result rates.
- Scheduler dispatch delay.
- Probe and storage write duration.
- Writer and notification queue depth.
- Disk capacity.
- Runtime errors.
- Configuration reload results.

Kemuri does not export target IDs or check IDs as Prometheus labels. This rule
keeps the number of time series bounded.

## Detailed check dashboard

Install the `yesoreyeram-infinity-datasource` plugin in Grafana. Create an
Infinity data source with these values:

| Setting | Value |
|---|---|
| Base URL | The Kemuri base URL |
| Allowed hosts | The same Kemuri base URL |
| Authentication | None, unless a reverse proxy requires it |
| Health check URL | `/healthz` |

The Grafana server sends the API requests. The Grafana server must be able to
connect to Kemuri.

Import `kemuri-check-analysis.json`. Select the Infinity data source. Then
select a target and a check.

The dashboard shows:

- A smoke-style latency heatmap.
- Minimum, p50, p95, and maximum latency.
- Measurement loss and health failure.
- Attempted, healthy, unhealthy, and lost sample counts.
- Observed, skipped, and missing time buckets.
- Alert events and check revision markers.

### Smoke-style heatmap

The heatmap uses the histogram in each fixed time bucket. The vertical axis is
latency in microseconds. The horizontal axis is time. A darker cell contains
more samples.

Timeouts and network errors do not enter the latency histogram. Use the loss
panel and the bucket-status panel to inspect these results.

The dashboard requests at most 300 time buckets for the heatmap. Kemuri uses
rollups when the selected range contains too many raw rounds.

## Authentication

Kemuri does not include authentication. Keep Grafana and Kemuri on a trusted
network, or put Kemuri behind a trusted reverse proxy.

If the proxy requires a header, store the header in the Infinity data source.
Do not store secrets in dashboard JSON.

## Local test stack

Start Kemuri on port 8080. Then start the example Prometheus and Grafana
services:

```sh
just grafana-up
```

Open `http://localhost:3000`. Use `admin` for the user name and password.

Stop the services:

```sh
just grafana-down
```

Set `KEMURI_URL` when the Infinity data source must use another address:

```sh
KEMURI_URL=http://192.0.2.10:8080 just grafana-up
```

Change `integrations/grafana/prometheus.yaml` when Prometheus must use another
address.
