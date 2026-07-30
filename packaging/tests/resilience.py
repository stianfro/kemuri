#!/usr/bin/env python3
"""Run bounded alert, restart, and resource-pressure scenarios for Kemuri."""

from __future__ import annotations

import argparse
import http.server
import json
import os
from pathlib import Path
import resource
import signal
import sqlite3
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request

from load import free_port, wait_ready


class FixtureState:
    def __init__(self) -> None:
        self.unhealthy = False
        self.delay_seconds = 0.0
        self.active_requests = 0
        self.fail_webhook = True
        self.webhooks = {"fast": 0, "slow": 0, "fail": 0}
        self.lock = threading.Lock()


class Handler(http.server.BaseHTTPRequestHandler):
    state: FixtureState

    def do_GET(self) -> None:
        if self.path != "/probe":
            self.send_response(404)
            self.end_headers()
            return
        with self.state.lock:
            self.state.active_requests += 1
            delay = self.state.delay_seconds
            unhealthy = self.state.unhealthy
        try:
            if delay:
                time.sleep(delay)
            body = b"unhealthy" if unhealthy else b"ok"
            self.send_response(503 if unhealthy else 200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass
        finally:
            with self.state.lock:
                self.state.active_requests -= 1

    def do_POST(self) -> None:
        kind = self.path.removeprefix("/webhook/")
        if kind not in self.state.webhooks:
            self.send_response(404)
            self.end_headers()
            return
        length = int(self.headers.get("Content-Length", "0"))
        self.rfile.read(length)
        with self.state.lock:
            self.state.webhooks[kind] += 1
        if kind == "slow":
            time.sleep(0.15)
        with self.state.lock:
            should_fail = kind == "fail" and self.state.fail_webhook
        self.send_response(503 if should_fail else 204)
        self.end_headers()

    def log_message(self, _format: str, *_args: object) -> None:
        return


class Fixture(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/kemuri"))
    parser.add_argument("--checks", type=int, default=12)
    parser.add_argument(
        "--scenario",
        choices=("all", "alerts", "restart", "resources"),
        default="all",
    )
    parser.add_argument("--output", default="-", help="JSON result path or '-' for stdout")
    args = parser.parse_args()
    if not 1 <= args.checks <= 100:
        parser.error("--checks must be between 1 and 100")
    if not args.binary.is_file():
        parser.error(f"Kemuri binary not found: {args.binary}")
    return args


def request(url: str, method: str = "GET", body: bytes | None = None) -> tuple[int, bytes]:
    headers = {"Content-Type": "application/json"} if body is not None else {}
    req = urllib.request.Request(url, method=method, data=body, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=3) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def wait_until(description: str, predicate, timeout: float = 20.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            if predicate():
                return
        except (OSError, urllib.error.URLError):
            pass
        time.sleep(0.1)
    raise RuntimeError(f"timed out waiting for {description}")


def write_config(
    path: Path,
    database: Path,
    server_port: int,
    fixture_port: int,
    checks: int,
    *,
    alerts: bool,
    warning_free: str = "0.5%",
    critical_free: str = "0.1%",
) -> None:
    data: dict[str, object] = {
        "version": 1,
        "server": {
            "bind": "127.0.0.1",
            "port": server_port,
            "shutdown_timeout": "10s",
        },
        "storage": {
            "path": str(database),
            "disk_pressure": {
                "warning_free": warning_free,
                "critical_free": critical_free,
            },
        },
        "scheduler": {
            "startup_mode": "immediate_then_aligned",
            "default_jitter": "0%",
            "tick_interval": "25ms",
            "max_concurrent": min(checks, 64),
            "max_concurrent_by_probe": {"http": min(checks, 64)},
        },
        "profiles": [
            {
                "kind": "http",
                "id": "resilience-http",
                "url": f"http://127.0.0.1:{fixture_port}/probe",
                "interval": "1s",
                "timeout": "2s",
                "expected_status": 200,
                "measure_until": "body",
            }
        ],
        "targets": [
            {
                "id": f"resilience-{index:03d}",
                "address": "127.0.0.1",
                "group_path": "resilience",
                "checks": [{"id": "http", "profile": "resilience-http"}],
            }
            for index in range(checks)
        ],
    }
    if alerts:
        data["notifiers"] = [
            {
                "kind": "webhook",
                "id": f"{kind}-webhook",
                "url": f"http://127.0.0.1:{fixture_port}/webhook/{kind}",
                "timeout": "2s",
            }
            for kind in ("fast", "slow", "fail")
        ]
        data["rules"] = [
            {
                "id": f"{kind}-storm",
                "profile": "resilience-http",
                "metric": "consecutive_unhealthy_rounds",
                "operator": "gte",
                "threshold": "1",
                "window": "1m",
                "notifier": f"{kind}-webhook",
                "minimum_rounds": 1,
            }
            for kind in ("fast", "slow", "fail")
        ]
    path.write_text(json.dumps(data, indent=2) + "\n")


def child_limits() -> None:
    resource.setrlimit(resource.RLIMIT_NOFILE, (128, 128))
    resource.setrlimit(resource.RLIMIT_CPU, (30, 30))
    address_limit = 2 * 1024 * 1024 * 1024
    resource.setrlimit(resource.RLIMIT_AS, (address_limit, address_limit))


def start_process(binary: Path, config: Path, log_file, limited: bool = False):
    return subprocess.Popen(
        [str(binary), "serve", "--config", str(config)],
        stdout=log_file,
        stderr=subprocess.STDOUT,
        preexec_fn=child_limits if limited else None,
    )


def database_counts(database: Path) -> dict[str, int]:
    with sqlite3.connect(database) as connection:
        return {
            "rounds": connection.execute("SELECT COUNT(*) FROM rounds").fetchone()[0],
            "firing_events": connection.execute(
                "SELECT COUNT(*) FROM alert_events WHERE event_type = 'firing'"
            ).fetchone()[0],
            "resolved_events": connection.execute(
                "SELECT COUNT(*) FROM alert_events WHERE event_type = 'resolved'"
            ).fetchone()[0],
            "delivered": connection.execute(
                "SELECT COUNT(*) FROM notification_outbox WHERE status = 'delivered'"
            ).fetchone()[0],
            "retried": connection.execute(
                "SELECT COUNT(*) FROM notification_outbox WHERE attempt_count >= 1"
            ).fetchone()[0],
            "pending": connection.execute(
                "SELECT COUNT(*) FROM notification_outbox WHERE status = 'pending'"
            ).fetchone()[0],
            "failed": connection.execute(
                "SELECT COUNT(*) FROM notification_outbox WHERE status = 'failed'"
            ).fetchone()[0],
        }


def integrity(database: Path) -> dict[str, object]:
    with sqlite3.connect(database) as connection:
        result = connection.execute("PRAGMA integrity_check").fetchone()[0]
        foreign_keys = connection.execute("PRAGMA foreign_key_check").fetchall()
    if result != "ok" or foreign_keys:
        raise RuntimeError(f"database validation failed: {result}, {len(foreign_keys)} FK rows")
    return {"integrity_check": result, "foreign_key_errors": len(foreign_keys)}


def stop(process: subprocess.Popen[bytes], timeout: float = 12.0) -> float:
    started = time.monotonic()
    process.send_signal(signal.SIGTERM)
    process.wait(timeout=timeout)
    elapsed = time.monotonic() - started
    if process.returncode != 0:
        raise RuntimeError(f"Kemuri stopped with status {process.returncode}")
    return elapsed


def run_alerts(binary: Path, root: Path, fixture: Fixture, state: FixtureState, checks: int):
    database = root / "alerts.db"
    config = root / "alerts.json"
    log = root / "alerts.log"
    server_port = free_port()
    write_config(config, database, server_port, fixture.server_port, checks, alerts=True)
    subprocess.run(["yq", "eval", ".", str(config)], check=True, stdout=subprocess.DEVNULL)
    with log.open("wb") as output:
        process = start_process(binary, config, output)
        try:
            wait_ready(f"http://127.0.0.1:{server_port}", process)
            with state.lock:
                state.unhealthy = True
            wait_until(
                "all alert rules to fire",
                lambda: database_counts(database)["firing_events"] >= checks * 3,
            )
            wait_until(
                "fast and slow delivery plus failing webhook retry",
                lambda: (
                    database_counts(database)["delivered"] >= checks * 2
                    and state.webhooks["fail"] >= checks
                ),
                timeout=30,
            )
            with state.lock:
                state.fail_webhook = False
            with sqlite3.connect(database) as connection:
                connection.execute(
                    """UPDATE notification_outbox
                    SET next_attempt_at = datetime('now')
                    WHERE notifier_id = 'fail-webhook' AND status = 'pending'"""
                )
                connection.commit()
            wait_until(
                "failing webhook queue recovery",
                lambda: database_counts(database)["delivered"] >= checks * 3,
                timeout=20,
            )
            with state.lock:
                state.unhealthy = False
            wait_until(
                "all alert rules to resolve",
                lambda: database_counts(database)["resolved_events"] >= checks * 3,
            )
            wait_until(
                "recovery notification delivery",
                lambda: database_counts(database)["delivered"] >= checks * 6,
                timeout=30,
            )
            shutdown_seconds = stop(process)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
    counts = database_counts(database)
    if counts["pending"] or counts["failed"]:
        raise RuntimeError("notification queue did not recover")
    return {
        "checks": checks,
        "database": counts,
        "webhook_requests": dict(state.webhooks),
        "failing_webhook_recovered": counts["pending"] == 0 and counts["failed"] == 0,
        "shutdown_seconds": round(shutdown_seconds, 3),
        **integrity(database),
    }


def run_restart(binary: Path, root: Path, fixture: Fixture, state: FixtureState, checks: int):
    database = root / "restart.db"
    config = root / "restart.json"
    log = root / "restart.log"
    server_port = free_port()
    write_config(config, database, server_port, fixture.server_port, checks, alerts=False)
    subprocess.run(["yq", "eval", ".", str(config)], check=True, stdout=subprocess.DEVNULL)
    with state.lock:
        state.unhealthy = False
        state.delay_seconds = 0.5
    with log.open("wb") as output:
        first = start_process(binary, config, output)
        wait_ready(f"http://127.0.0.1:{server_port}", first)
        wait_until("an active probe", lambda: state.active_requests > 0, timeout=5)
        first_shutdown = stop(first)
        first_counts = database_counts(database)
        second = start_process(binary, config, output)
        try:
            wait_ready(f"http://127.0.0.1:{server_port}", second)
            wait_until(
                "rounds after restart",
                lambda: database_counts(database)["rounds"] > first_counts["rounds"],
                timeout=5,
            )
            second_shutdown = stop(second)
        finally:
            if second.poll() is None:
                second.kill()
                second.wait()
    with state.lock:
        state.delay_seconds = 0.0
    return {
        "rounds_before_restart": first_counts["rounds"],
        "rounds_after_restart": database_counts(database)["rounds"],
        "first_shutdown_seconds": round(first_shutdown, 3),
        "second_shutdown_seconds": round(second_shutdown, 3),
        **integrity(database),
    }


def run_resources(binary: Path, root: Path, fixture: Fixture, checks: int):
    database = root / "resources.db"
    config = root / "resources.json"
    log = root / "resources.log"
    server_port = free_port()
    write_config(
        config,
        database,
        server_port,
        fixture.server_port,
        checks,
        alerts=False,
        warning_free="100%",
        critical_free="99.99%",
    )
    subprocess.run(["yq", "eval", ".", str(config)], check=True, stdout=subprocess.DEVNULL)
    base_url = f"http://127.0.0.1:{server_port}"
    with log.open("wb") as output:
        process = start_process(binary, config, output, limited=True)
        try:
            wait_until("HTTP service start", lambda: request(base_url + "/api/v1/info")[0] == 200)
            wait_until("disk-pressure readiness failure", lambda: request(base_url + "/readyz")[0] == 503, timeout=8)
            wait_until("system API during disk pause", lambda: request(base_url + "/api/v1/system/status")[0] == 200)
            time.sleep(1.2)
            paused_rounds = database_counts(database)["rounds"]
            time.sleep(2.0)
            if database_counts(database)["rounds"] != paused_rounds:
                raise RuntimeError("round count increased while disk pressure paused scheduling")
            write_config(config, database, server_port, fixture.server_port, checks, alerts=False)
            status, _ = request(base_url + "/api/v1/config/reload", "POST", b"{}")
            if status != 202:
                raise RuntimeError(f"reload returned HTTP {status}")
            wait_until("disk-pressure recovery", lambda: request(base_url + "/readyz")[0] == 200, timeout=10)
            wait_until("rounds after disk recovery", lambda: database_counts(database)["rounds"] > paused_rounds, timeout=5)
            shutdown_seconds = stop(process)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
    return {
        "limits": {"open_files": 128, "cpu_seconds": 30, "address_space_bytes": 2 * 1024**3},
        "paused_rounds": paused_rounds,
        "final_rounds": database_counts(database)["rounds"],
        "shutdown_seconds": round(shutdown_seconds, 3),
        **integrity(database),
    }


def main() -> int:
    args = parse_args()
    binary = args.binary.resolve()
    state = FixtureState()
    Handler.state = state
    fixture = Fixture(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=fixture.serve_forever, daemon=True)
    thread.start()
    failures: list[str] = []
    scenarios: dict[str, object] = {}
    selected = ("alerts", "restart", "resources") if args.scenario == "all" else (args.scenario,)
    try:
        with tempfile.TemporaryDirectory(prefix="kemuri-resilience-") as temporary:
            root = Path(temporary)
            for name in selected:
                try:
                    if name == "alerts":
                        scenarios[name] = run_alerts(binary, root, fixture, state, args.checks)
                    elif name == "restart":
                        scenarios[name] = run_restart(binary, root, fixture, state, args.checks)
                    else:
                        scenarios[name] = run_resources(binary, root, fixture, args.checks)
                except Exception as error:
                    failures.append(f"{name}: {error}")
    finally:
        fixture.shutdown()
        fixture.server_close()
        thread.join(timeout=2)
    result = {
        "schema_version": 1,
        "status": "passed" if not failures else "failed",
        "scenarios": scenarios,
        "failures": failures,
    }
    encoded = json.dumps(result, indent=2) + "\n"
    if args.output == "-":
        print(encoded, end="")
    else:
        Path(args.output).write_text(encoded)
    return 0 if not failures else 1


if __name__ == "__main__":
    raise SystemExit(main())
