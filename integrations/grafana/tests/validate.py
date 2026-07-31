#!/usr/bin/env python3
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DASHBOARDS = ROOT / "dashboards"
EXPECTED = {
    "kemuri-service-health.json": {
        "uid": "kemuri-service-health",
        "datasource": "prometheus",
        "required_metrics": {
            "kemuri_build_info",
            "kemuri_scheduler_active_checks",
            "kemuri_scheduler_rounds_total",
            "kemuri_probe_rounds_total",
            "kemuri_disk_free_ratio",
            "kemuri_storage_write_duration_seconds",
        },
    },
    "kemuri-check-analysis.json": {
        "uid": "kemuri-check-analysis",
        "datasource": "yesoreyeram-infinity-datasource",
        "required_paths": {
            "/api/v1/targets?limit=200",
            "/api/v1/targets/${target:percentencode}/checks?limit=200",
        },
    },
}


def walk(value):
    if isinstance(value, dict):
        yield value
        for item in value.values():
            yield from walk(item)
    elif isinstance(value, list):
        for item in value:
            yield from walk(item)


def fail(message):
    print(f"grafana validation: {message}", file=sys.stderr)
    raise SystemExit(1)


for filename, expected in EXPECTED.items():
    path = DASHBOARDS / filename
    try:
        dashboard = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read {filename}: {error}")

    if dashboard.get("uid") != expected["uid"]:
        fail(f"{filename} has an unexpected UID")
    if not dashboard.get("title") or not dashboard.get("panels"):
        fail(f"{filename} must have a title and panels")
    if dashboard.get("timezone") != "browser":
        fail(f"{filename} must use the browser time zone")

    objects = list(walk(dashboard))
    datasource_types = {
        item.get("datasource", {}).get("type")
        for item in objects
        if isinstance(item.get("datasource"), dict)
    }
    if expected["datasource"] not in datasource_types:
        fail(f"{filename} does not use {expected['datasource']}")

    encoded = json.dumps(dashboard)
    for metric in expected.get("required_metrics", set()):
        if metric not in encoded:
            fail(f"{filename} does not query {metric}")
    for api_path in expected.get("required_paths", set()):
        if api_path not in encoded:
            fail(f"{filename} does not query {api_path}")

    if filename == "kemuri-service-health.json":
        forbidden = ('target_id=', 'check_id=', 'target_id=~', 'check_id=~')
        if any(label in encoded for label in forbidden):
            fail("the Prometheus dashboard uses a high-cardinality check label")

    if filename == "kemuri-check-analysis.json":
        queries = [item for item in objects if item.get("type") == "json"]
        if not queries:
            fail("the Infinity dashboard has no JSON queries")
        for query in queries:
            if query.get("parser") != "backend":
                fail("all Infinity JSON queries must use the backend parser")
            if query.get("source") != "url" or query.get("url_options", {}).get("method") != "GET":
                fail("Infinity queries must use HTTP GET URL sources")
        if "${__timeFrom}" not in encoded or "${__timeTo}" not in encoded:
            fail("the Infinity dashboard does not use backend time macros")
        heatmaps = [panel for panel in dashboard["panels"] if panel.get("type") == "heatmap"]
        if len(heatmaps) != 1:
            fail("the Infinity dashboard must have one smoke-style heatmap")
        heatmap = heatmaps[0]
        if heatmap.get("options", {}).get("calculate") is not True:
            fail("the smoke-style heatmap must calculate cells from sample density")
        heatmap_query = heatmap.get("targets", [{}])[0]
        if "max_points=300" not in heatmap_query.get("url", ""):
            fail("the smoke-style heatmap must limit the series to 300 time buckets")
        selectors = {column.get("selector") for column in heatmap_query.get("columns", [])}
        if not {"timestamp_ms", "latency_us"}.issubset(selectors):
            fail("the smoke-style heatmap does not define latency samples")

print(f"validated {len(EXPECTED)} Grafana dashboards")
