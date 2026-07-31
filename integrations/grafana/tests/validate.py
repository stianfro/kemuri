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
        state_panel = next(
            (panel for panel in dashboard["panels"] if panel.get("title") == "Current state"),
            None,
        )
        if state_panel is None:
            fail("the Infinity dashboard has no current-state panel")
        state_query = state_panel.get("targets", [{}])[0]
        if state_query.get("root_selector") != "$":
            fail("the current-state query must select the API response root")
        state_columns = state_query.get("columns", [])
        if not any(
            column.get("selector") == "state" and column.get("type") == "string"
            for column in state_columns
        ):
            fail("the current-state query must read the string state field")
        state_codes = state_query.get("computed_columns", [])
        if not any(
            column.get("text") == "State code" and column.get("type") == "number"
            for column in state_codes
        ):
            fail("the current-state query must calculate a numeric state code")
        if state_panel.get("options", {}).get("reduceOptions", {}).get("fields") != "State code":
            fail("the current-state panel must display the calculated state code")
        heatmaps = [panel for panel in dashboard["panels"] if panel.get("type") == "heatmap"]
        if len(heatmaps) != 1:
            fail("the Infinity dashboard must have one smoke-style heatmap")
        heatmap = heatmaps[0]
        heatmap_options = heatmap.get("options", {})
        if heatmap_options.get("calculate") is not True:
            fail("the smoke-style heatmap must calculate cells from sample density")
        calculation = heatmap_options.get("calculation", {})
        if calculation.get("xBuckets", {}).get("value") != "24":
            fail("the smoke-style heatmap must use 24 time buckets")
        if calculation.get("yBuckets", {}).get("value") != "48":
            fail("the smoke-style heatmap must use 48 latency buckets")
        if heatmap_options.get("cellGap") != 0:
            fail("the smoke-style heatmap must not put gaps between density cells")
        if heatmap_options.get("color", {}).get("scheme") != "Blues":
            fail("the smoke-style heatmap must use a theme-safe density color scheme")
        panel_by_title = {panel.get("title"): panel for panel in dashboard["panels"]}
        loss_position = panel_by_title.get("Loss and health failures", {}).get("gridPos", {})
        status_position = panel_by_title.get("Bucket status", {}).get("gridPos", {})
        heatmap_position = heatmap.get("gridPos", {})
        expected_detail_y = heatmap_position.get("y", 0) + heatmap_position.get("h", 0)
        if loss_position.get("y") != expected_detail_y or status_position.get("y") != expected_detail_y:
            fail("loss and bucket status must appear directly below the smoke-style heatmap")
        heatmap_query = heatmap.get("targets", [{}])[0]
        if "max_points=300" not in heatmap_query.get("url", ""):
            fail("the smoke-style heatmap must limit the series to 300 time buckets")
        selectors = {column.get("selector") for column in heatmap_query.get("columns", [])}
        if not {"timestamp_ms", "latency_us"}.issubset(selectors):
            fail("the smoke-style heatmap does not define latency samples")

print(f"validated {len(EXPECTED)} Grafana dashboards")
