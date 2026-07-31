# Grafana

Prometheus can scrape the Kemuri `/metrics` endpoint. The
`integrations/grafana` directory contains a service-health dashboard and
provisioning examples.

The same directory contains an optional detailed dashboard for the Grafana
Infinity data source. It reads `/api/v1` and shows a smoke-style latency
heatmap, latency percentiles, loss, bucket state, alert events, and revision
markers.

The full setup procedure is in the
[Grafana operations guide](https://stianfro.github.io/kemuri/operations/grafana).
