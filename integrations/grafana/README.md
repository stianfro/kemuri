# Grafana integration

Kemuri supplies two Grafana dashboards:

- `Kemuri service health` uses the built-in Prometheus data source. It shows
  scheduler, worker, storage, disk, and process metrics.
- `Kemuri check analysis` uses the optional Grafana Infinity data source. It
  reads detailed check data from the Kemuri HTTP API.

The Infinity dashboard does not change the metrics that Kemuri exports. Target
and check identifiers do not become Prometheus labels.

## Test the dashboards with Docker

Start Kemuri on port 8080. Then run:

```sh
just grafana-up
```

Open <http://localhost:3000>. Use `admin` for the user name and password. The
provisioned dashboards are in the `Kemuri` folder.

Stop the test stack with:

```sh
just grafana-down
```

The example stack expects Kemuri at `http://host.docker.internal:8080`. Change
`prometheus.yaml` when Kemuri uses a different address. Set `KEMURI_URL` to
change the address that the Infinity data source uses:

```sh
KEMURI_URL=http://192.0.2.10:8080 just grafana-up
```

The check analysis dashboard starts with a smoke-style latency distribution.
Each heatmap cell contains the sample count for one latency and time bucket.
Timeouts and network errors do not enter the latency distribution. The loss
panel shows these outcomes separately.

## Install in an existing Grafana system

1. Configure Prometheus to scrape the Kemuri `/metrics` endpoint.
2. Import `dashboards/kemuri-service-health.json`.
3. Select the Prometheus data source in the dashboard.
4. Install `yesoreyeram-infinity-datasource` if detailed check graphs are
   required.
5. Create an Infinity data source. Set its URL to the Kemuri base URL.
6. Add the Kemuri base URL to the Infinity allowed hosts list.
7. Import `dashboards/kemuri-check-analysis.json`.
8. Select the Infinity data source in the dashboard.

Infinity sends API requests from the Grafana server. The Grafana server must be
able to connect to Kemuri.

Kemuri does not include authentication. Keep both services on a trusted network,
or put Kemuri behind a trusted reverse proxy. Configure authentication headers
in the Grafana data source, not in dashboard JSON.
