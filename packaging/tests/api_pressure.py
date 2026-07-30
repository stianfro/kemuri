#!/usr/bin/env python3
"""Exercise bounded API pagination and concurrent SSE connections."""

from __future__ import annotations

import argparse
import concurrent.futures
import http.server
import json
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request


TARGET_COUNT = 205


class FixtureHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        body = b"ok"
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def log_message(self, _format: str, *_args: object) -> None:
        return


class FixtureServer(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def parse_args() -> argparse.Namespace:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary",
        type=Path,
        default=root / "target" / "debug" / "kemuri",
        help="Kemuri binary to test",
    )
    parser.add_argument(
        "--sse-clients",
        type=int,
        default=8,
        help="Number of concurrent SSE clients, from 1 through 32",
    )
    parser.add_argument(
        "--event-timeout",
        type=float,
        default=4.0,
        help="Maximum seconds to wait for each SSE event",
    )
    args = parser.parse_args()
    if not args.binary.is_file():
        parser.error(f"Kemuri binary not found: {args.binary}")
    if not 1 <= args.sse_clients <= 32:
        parser.error("--sse-clients must be between 1 and 32")
    if args.event_timeout <= 0:
        parser.error("--event-timeout must be greater than zero")
    return args


def write_config(path: Path, database: Path, port: int, fixture_port: int) -> None:
    lines = [
        "version: 1",
        "server:",
        "  bind: 127.0.0.1",
        f"  port: {port}",
        "  shutdown_timeout: 3s",
        "storage:",
        f"  path: {json.dumps(str(database))}",
        "  disk_pressure:",
        "    warning_free: 0.5%",
        "    critical_free: 0.1%",
        "scheduler:",
        "  startup_mode: immediate_then_aligned",
        "  default_jitter: 0%",
        "  tick_interval: 50ms",
        f"  max_concurrent: {TARGET_COUNT}",
        "profiles:",
        "  - kind: http",
        "    id: pressure-http",
        f"    url: http://127.0.0.1:{fixture_port}/health",
        "    interval: 1s",
        "    timeout: 2s",
        "    expected_status: 200",
        "targets:",
    ]
    for index in range(TARGET_COUNT):
        lines.extend(
            [
                f"  - id: pressure-{index:03d}",
                "    address: 127.0.0.1",
                f"    group_path: pressure/group-{index:03d}",
                "    checks:",
                "      - id: http",
                "        profile: pressure-http",
            ]
        )
    path.write_text("\n".join(lines) + "\n")


def request(base_url: str, path: str) -> tuple[int, dict[str, object]]:
    try:
        with urllib.request.urlopen(base_url + path, timeout=3) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, json.load(error)


def require_status(
    base_url: str, path: str, expected: int
) -> dict[str, object]:
    status, body = request(base_url, path)
    if status != expected:
        raise AssertionError(f"{path}: expected HTTP {expected}, got {status}: {body}")
    return body


def wait_ready(base_url: str, process: subprocess.Popen[bytes]) -> None:
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Kemuri exited during startup: {process.returncode}")
        try:
            with urllib.request.urlopen(base_url + "/readyz", timeout=0.5) as response:
                if response.status == 200:
                    return
        except (OSError, urllib.error.URLError):
            pass
        time.sleep(0.05)
    raise RuntimeError("Kemuri did not become ready")


def check_pagination(base_url: str) -> dict[str, object]:
    one = require_status(base_url, "/api/v1/targets?limit=1", 200)
    if len(one["targets"]) != 1 or not one["next_cursor"]:
        raise AssertionError("limit=1 did not return one target and a cursor")

    large = require_status(base_url, "/api/v1/targets?limit=200", 200)
    if len(large["targets"]) != 200 or not large["next_cursor"]:
        raise AssertionError("limit=200 did not return 200 targets and a cursor")

    pages: list[list[str]] = []
    cursor: str | None = None
    while True:
        query = "/api/v1/targets?limit=100"
        if cursor:
            query += "&cursor=" + urllib.parse.quote(cursor)
        page = require_status(base_url, query, 200)
        ids = [str(target["target_id"]) for target in page["targets"]]
        pages.append(ids)
        cursor_value = page["next_cursor"]
        cursor = str(cursor_value) if cursor_value is not None else None
        if cursor is None:
            break
    flattened = [target_id for page in pages for target_id in page]
    expected = [f"pressure-{index:03d}" for index in range(TARGET_COUNT)]
    if flattened != expected or [len(page) for page in pages] != [100, 100, 5]:
        raise AssertionError("first, middle, and final cursor pages were not stable")

    group_paths: list[str] = []
    cursor = None
    group_page_sizes: list[int] = []
    while True:
        query = "/api/v1/groups?limit=100"
        if cursor:
            query += "&cursor=" + urllib.parse.quote(cursor)
        page = require_status(base_url, query, 200)
        current = [str(group["group_path"]) for group in page["groups"]]
        group_paths.extend(current)
        group_page_sizes.append(len(current))
        cursor_value = page["next_cursor"]
        cursor = str(cursor_value) if cursor_value is not None else None
        if cursor is None:
            break
    expected_groups = [
        f"pressure/group-{index:03d}" for index in range(TARGET_COUNT)
    ]
    if group_paths != expected_groups or group_page_sizes != [100, 100, 5]:
        raise AssertionError("group cursor pages lost, repeated, or truncated groups")
    final_group = require_status(
        base_url,
        "/api/v1/groups/" + urllib.parse.quote(expected_groups[-1], safe=""),
        200,
    )
    if final_group.get("group_path") != expected_groups[-1]:
        raise AssertionError("group detail failed after the first 200 group paths")

    paginated_paths = (
        "/api/v1/targets",
        "/api/v1/groups",
        "/api/v1/targets/pressure-000/checks",
        "/api/v1/targets/pressure-000/checks/http/rounds",
        "/api/v1/alerts",
        "/api/v1/alert-events",
    )
    for path in paginated_paths:
        separator = "&" if "?" in path else "?"
        require_status(base_url, f"{path}{separator}limit=1", 200)
        require_status(base_url, f"{path}{separator}limit=200", 200)
        for value in ("0", "201", "-1", "invalid"):
            body = require_status(
                base_url, f"{path}{separator}limit={value}", 400
            )
            if body.get("code") != "bad_request":
                raise AssertionError(
                    f"{path}: invalid limit {value!r} lacked a bad_request body"
                )
        for cursor_value in ("zz", "ff"):
            body = require_status(
                base_url, f"{path}{separator}cursor={cursor_value}", 400
            )
            if body.get("code") != "bad_request":
                raise AssertionError(
                    f"{path}: invalid cursor {cursor_value!r} lacked a bad_request body"
                )

    return {
        "targets": len(flattened),
        "page_sizes": [len(page) for page in pages],
        "groups": len(group_paths),
        "group_page_sizes": group_page_sizes,
        "paginated_routes": len(paginated_paths),
        "limit_boundaries": [1, 200],
        "invalid_limits": len(paginated_paths) * 4,
        "invalid_cursors": len(paginated_paths) * 2,
    }


def read_sse_events(
    base_url: str, event_count: int, timeout: float
) -> list[dict[str, object]]:
    started = time.monotonic()
    request_value = urllib.request.Request(
        base_url + "/api/v1/events", headers={"Accept": "text/event-stream"}
    )
    events: list[dict[str, object]] = []
    with urllib.request.urlopen(request_value, timeout=timeout) as response:
        if response.status != 200:
            raise AssertionError(f"SSE returned HTTP {response.status}")
        if not response.headers.get_content_type() == "text/event-stream":
            raise AssertionError("SSE response has the wrong content type")
        current_type: str | None = None
        current_data: str | None = None
        while len(events) < event_count:
            line = response.readline().decode("utf-8").rstrip("\r\n")
            if line.startswith("event:"):
                current_type = line.removeprefix("event:").strip()
            elif line.startswith("data:"):
                current_data = line.removeprefix("data:").strip()
            elif not line and current_type is not None and current_data is not None:
                events.append(
                    {
                        "type": current_type,
                        "data": json.loads(current_data),
                        "elapsed_seconds": time.monotonic() - started,
                    }
                )
                current_type = None
                current_data = None
    return events


def check_sse(
    base_url: str, clients: int, event_timeout: float
) -> dict[str, object]:
    with concurrent.futures.ThreadPoolExecutor(max_workers=clients) as executor:
        futures = [
            executor.submit(read_sse_events, base_url, 5, event_timeout)
            for _ in range(clients)
        ]
        client_events = [future.result(timeout=event_timeout + 1) for future in futures]

    expected_type = "round.completed"
    if any(
        not any(event["type"] == expected_type for event in events)
        for events in client_events
    ):
        raise AssertionError("an SSE client did not receive a round event")
    first_event_times = [
        float(events[0]["elapsed_seconds"]) for events in client_events
    ]
    if max(first_event_times) > event_timeout:
        raise AssertionError("an SSE client's first event exceeded the event timeout")

    reconnect_started = time.monotonic()
    reconnected = read_sse_events(base_url, 5, event_timeout)
    reconnect_seconds = time.monotonic() - reconnect_started
    if not any(event["type"] == expected_type for event in reconnected):
        raise AssertionError("reconnected SSE client did not receive a round event")

    return {
        "clients": clients,
        "events": sum(len(events) for events in client_events) + len(reconnected),
        "max_first_event_seconds": round(max(first_event_times), 3),
        "reconnect_event_seconds": round(reconnect_seconds, 3),
    }


def main() -> int:
    args = parse_args()
    fixture = FixtureServer(("127.0.0.1", 0), FixtureHandler)
    fixture_thread = threading.Thread(target=fixture.serve_forever, daemon=True)
    fixture_thread.start()
    process: subprocess.Popen[bytes] | None = None
    with tempfile.TemporaryDirectory(prefix="kemuri-api-pressure-") as temporary:
        temporary_path = Path(temporary)
        config = temporary_path / "kemuri.yaml"
        log = temporary_path / "kemuri.log"
        port = free_port()
        write_config(
            config,
            temporary_path / "kemuri.db",
            port,
            int(fixture.server_address[1]),
        )
        base_url = f"http://127.0.0.1:{port}"
        try:
            with log.open("wb") as log_file:
                process = subprocess.Popen(
                    [str(args.binary.resolve()), "serve", "--config", str(config)],
                    stdout=log_file,
                    stderr=subprocess.STDOUT,
                )
                wait_ready(base_url, process)
                result = {
                    "pagination": check_pagination(base_url),
                    "sse": check_sse(
                        base_url, args.sse_clients, args.event_timeout
                    ),
                }
                process.send_signal(signal.SIGTERM)
                process.wait(timeout=10)
                if process.returncode != 0:
                    raise RuntimeError(f"Kemuri exited with {process.returncode}")
            print(json.dumps(result, indent=2, sort_keys=True))
            return 0
        except Exception:
            if process is not None and process.poll() is None:
                process.send_signal(signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
            if log.exists():
                print(log.read_text(errors="replace"), end="")
            raise
        finally:
            fixture.shutdown()
            fixture.server_close()
            fixture_thread.join(timeout=2)


if __name__ == "__main__":
    raise SystemExit(main())
