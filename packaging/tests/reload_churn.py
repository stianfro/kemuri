#!/usr/bin/env python3
"""Exercise bounded configuration reload churn against a local Kemuri server."""

from __future__ import annotations

import argparse
import http.server
import json
from pathlib import Path
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
from typing import BinaryIO, Callable
import urllib.error
import urllib.request


class FixtureHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        body = b"ok"
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        return


class FixtureServer(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def request_json(
    url: str, *, method: str = "GET", body: bytes | None = None
) -> tuple[int, dict[str, object]]:
    headers = {"Content-Type": "application/json"} if body is not None else {}
    request = urllib.request.Request(url, method=method, data=body, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=2) as response:
            data = response.read()
            try:
                payload = json.loads(data) if data else {}
            except json.JSONDecodeError:
                payload = {}
            return response.status, payload
    except urllib.error.HTTPError as error:
        try:
            payload = json.load(error)
        except json.JSONDecodeError:
            payload = {}
        return error.code, payload


def wait_until(
    description: str, predicate: Callable[[], bool], *, timeout: float = 10.0
) -> None:
    deadline = time.monotonic() + timeout
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            if predicate():
                return
        except (OSError, sqlite3.Error, urllib.error.URLError) as error:
            last_error = error
        time.sleep(0.05)
    detail = f": {last_error}" if last_error else ""
    raise AssertionError(f"timed out waiting for {description}{detail}")


def render_config(
    server_port: int,
    fixture_port: int,
    database: Path,
    *,
    base_enabled: bool = True,
    include_extra: bool = False,
    timeout: str = "400ms",
) -> str:
    checks = [
        "      - id: base",
        "        profile: local-http",
        f"        enabled: {str(base_enabled).lower()}",
    ]
    if include_extra:
        checks.extend(
            (
                "      - id: extra",
                "        profile: local-http",
            )
        )
    return (
        f"""version: 1
server:
  bind: 127.0.0.1
  port: {server_port}
  public_url: http://127.0.0.1:{server_port}
  shutdown_timeout: 2s
storage:
  path: {database}
  disk_pressure:
    warning_free: 0.5%
    critical_free: 0.1%
scheduler:
  tick_interval: 50ms
  startup_mode: immediate_then_aligned
  default_jitter: 0%
  max_concurrent: 8
profiles:
  - kind: http
    id: local-http
    url: http://127.0.0.1:{fixture_port}/health
    interval: 1s
    timeout: {timeout}
    expected_status: 200
targets:
  - id: reload-target
    address: 127.0.0.1
    group_path: tests/reload
    checks:
"""
        + "\n".join(checks)
        + "\n"
    )


class ChurnTest:
    def __init__(self, binary: Path, iterations: int) -> None:
        self.binary = binary
        self.iterations = iterations
        self.temporary = tempfile.TemporaryDirectory(prefix="kemuri-reload-churn-")
        self.directory = Path(self.temporary.name)
        self.config = self.directory / "kemuri.yaml"
        self.database = self.directory / "kemuri.db"
        self.log = self.directory / "server.log"
        self.server_port = free_port()
        self.fixture = FixtureServer(("127.0.0.1", 0), FixtureHandler)
        self.fixture_port = int(self.fixture.server_address[1])
        self.fixture_thread = threading.Thread(
            target=self.fixture.serve_forever, daemon=True
        )
        self.process: subprocess.Popen[bytes] | None = None
        self.log_file: BinaryIO | None = None
        self.results: dict[str, object] = {}

    @property
    def api(self) -> str:
        return f"http://127.0.0.1:{self.server_port}"

    def write_valid(
        self,
        *,
        base_enabled: bool = True,
        include_extra: bool = False,
        timeout: str = "400ms",
    ) -> None:
        self.config.write_text(
            render_config(
                self.server_port,
                self.fixture_port,
                self.database,
                base_enabled=base_enabled,
                include_extra=include_extra,
                timeout=timeout,
            )
        )
        subprocess.run(
            ["yq", "eval", ".", str(self.config)],
            check=True,
            stdout=subprocess.DEVNULL,
        )

    def status(self) -> dict[str, object]:
        code, payload = request_json(f"{self.api}/api/v1/system/status")
        assert code == 200, f"system status returned HTTP {code}"
        return payload

    def reload(self, expected: str) -> dict[str, object]:
        previous = self.status().get("last_config_reload")
        previous_timestamp = (
            previous.get("timestamp_ms") if isinstance(previous, dict) else None
        )
        time.sleep(0.01)
        code, response = request_json(
            f"{self.api}/api/v1/config/reload", method="POST", body=b"{}"
        )
        assert code == 202, f"reload returned HTTP {code}: {response}"

        latest: dict[str, object] = {}

        def completed() -> bool:
            nonlocal latest
            value = self.status().get("last_config_reload")
            if not isinstance(value, dict):
                return False
            latest = value
            return (
                value.get("result") == expected
                and value.get("timestamp_ms") != previous_timestamp
            )

        wait_until(f"{expected} reload", completed)
        return latest

    def sql_value(self, query: str, parameters: tuple[object, ...] = ()) -> object:
        with sqlite3.connect(self.database, timeout=2) as connection:
            row = connection.execute(query, parameters).fetchone()
        assert row is not None
        return row[0]

    def check_row(self, check_id: str) -> tuple[int, int, str]:
        with sqlite3.connect(self.database, timeout=2) as connection:
            row = connection.execute(
                """
                SELECT c.active, a.active, c.current_revision_id
                FROM checks c
                JOIN targets t ON t.internal_id = c.target_internal_id
                JOIN check_assignments a ON a.check_internal_id = c.internal_id
                WHERE t.target_id = 'reload-target' AND c.check_id = ?
                """,
                (check_id,),
            ).fetchone()
        assert row is not None, f"missing database row for check {check_id}"
        return int(row[0]), int(row[1]), str(row[2])

    def round_count(self, check_id: str) -> int:
        return int(
            self.sql_value(
                """
                SELECT COUNT(*) FROM rounds r
                JOIN checks c ON c.internal_id = r.check_internal_id
                JOIN targets t ON t.internal_id = c.target_internal_id
                WHERE t.target_id = 'reload-target' AND c.check_id = ?
                """,
                (check_id,),
            )
        )

    def api_check_ids(self) -> set[str]:
        code, payload = request_json(
            f"{self.api}/api/v1/targets/reload-target/checks"
        )
        assert code == 200, f"checks endpoint returned HTTP {code}: {payload}"
        checks = payload.get("checks")
        assert isinstance(checks, list)
        return {
            str(check["check_id"])
            for check in checks
            if isinstance(check, dict) and "check_id" in check
        }

    def run(self) -> dict[str, object]:
        self.write_valid()
        self.fixture_thread.start()
        self.log_file = self.log.open("wb")
        self.process = subprocess.Popen(
            [str(self.binary), "serve", "--config", str(self.config)],
            stdout=self.log_file,
            stderr=subprocess.STDOUT,
        )
        wait_until("server readiness", lambda: request_json(f"{self.api}/readyz")[0] == 200)
        wait_until("initial base round", lambda: self.round_count("base") >= 1)
        initial_rounds = self.round_count("base")
        initial_revision = self.check_row("base")[2]

        self.write_valid(include_extra=True)
        self.reload("success")
        assert self.api_check_ids() == {"base", "extra"}
        assert self.check_row("extra")[:2] == (1, 1)
        wait_until("extra check round", lambda: self.round_count("extra") >= 1)

        self.write_valid(include_extra=True, timeout="600ms")
        self.reload("success")
        modified_revision = self.check_row("base")[2]
        assert modified_revision != initial_revision, "modified check kept old revision"
        revision_count = int(
            self.sql_value(
                """
                SELECT COUNT(*) FROM check_revisions cr
                JOIN checks c ON c.internal_id = cr.check_internal_id
                JOIN targets t ON t.internal_id = c.target_internal_id
                WHERE t.target_id = 'reload-target' AND c.check_id = 'base'
                """
            )
        )
        assert revision_count >= 2

        self.write_valid(base_enabled=False, include_extra=True, timeout="600ms")
        self.reload("success")
        assert self.check_row("base")[:2] == (0, 0)
        assert self.api_check_ids() == {"extra"}
        time.sleep(1.0)
        disabled_rounds = self.round_count("base")
        time.sleep(1.25)
        assert (
            self.round_count("base") == disabled_rounds
        ), "disabled check still ran after in-flight work drained"

        self.write_valid(include_extra=True, timeout="600ms")
        self.reload("success")
        assert self.check_row("base")[:2] == (1, 1)
        wait_until(
            "re-enabled base round",
            lambda: self.round_count("base") > disabled_rounds,
        )

        self.write_valid(timeout="600ms")
        self.reload("success")
        assert self.api_check_ids() == {"base"}
        assert self.check_row("extra")[:2] == (0, 0)

        events_before_repeat = int(
            self.sql_value("SELECT COUNT(*) FROM config_events")
        )
        for _ in range(self.iterations):
            self.reload("success")
        events_after_repeat = int(self.sql_value("SELECT COUNT(*) FROM config_events"))
        assert events_after_repeat - events_before_repeat == self.iterations
        assert self.check_row("base")[2] == modified_revision

        valid_config = self.config.read_text()
        rounds_before_invalid = self.round_count("base")
        events_before_invalid = events_after_repeat
        self.config.write_text("version: 2\n")
        subprocess.run(
            ["yq", "eval", ".", str(self.config)],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        failure = self.reload("failure")
        assert failure.get("error"), "invalid reload did not report an error"
        assert int(self.sql_value("SELECT COUNT(*) FROM config_events")) == events_before_invalid
        assert self.api_check_ids() == {"base"}
        assert self.check_row("base")[2] == modified_revision
        wait_until(
            "round after invalid reload",
            lambda: self.round_count("base") > rounds_before_invalid,
        )
        assert self.status().get("status") == "running"
        self.config.write_text(valid_config)

        duplicates = int(
            self.sql_value(
                """
                SELECT COUNT(*) FROM (
                    SELECT check_internal_id, observer_internal_id, scheduled_at
                    FROM rounds
                    GROUP BY check_internal_id, observer_internal_id, scheduled_at
                    HAVING COUNT(*) > 1
                )
                """
            )
        )
        assert duplicates == 0, f"found {duplicates} duplicate scheduled slots"
        assert self.round_count("base") > initial_rounds
        active_assignments = int(
            self.sql_value(
                """
                SELECT COUNT(*) FROM check_assignments a
                JOIN checks c ON c.internal_id = a.check_internal_id
                JOIN targets t ON t.internal_id = c.target_internal_id
                WHERE t.target_id = 'reload-target' AND a.active = 1
                """
            )
        )
        assert active_assignments == 1
        self.results = {
            "status": "passed",
            "valid_repeat_reloads": self.iterations,
            "config_events": events_after_repeat,
            "base_revisions": revision_count,
            "base_rounds": self.round_count("base"),
            "duplicate_scheduled_slots": duplicates,
            "active_assignments": active_assignments,
            "invalid_reload_rolled_back": True,
        }
        return self.results

    def close(self) -> None:
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=2)
        if self.log_file is not None:
            self.log_file.close()
        self.fixture.shutdown()
        self.fixture.server_close()
        if self.fixture_thread.is_alive():
            self.fixture_thread.join(timeout=2)

    def failure_log(self) -> str:
        if not self.log.exists():
            return ""
        return self.log.read_text(errors="replace")[-8000:]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary",
        type=Path,
        default=Path("target/debug/kemuri"),
        help="path to an existing Kemuri binary",
    )
    parser.add_argument(
        "--iterations",
        type=int,
        default=5,
        help="number of unchanged valid reloads (1 to 100)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not 1 <= args.iterations <= 100:
        raise SystemExit("--iterations must be between 1 and 100")
    if shutil.which("yq") is None:
        raise SystemExit("yq is required")
    binary = args.binary.resolve()
    if not binary.is_file():
        raise SystemExit(f"Kemuri binary does not exist: {binary}; run `just build`")
    test = ChurnTest(binary, args.iterations)
    try:
        result = test.run()
        json.dump(result, sys.stdout, indent=2, sort_keys=True)
        sys.stdout.write("\n")
        return 0
    except Exception:
        log = test.failure_log()
        if log:
            print("\nKemuri server log:\n" + log, file=sys.stderr)
        raise
    finally:
        test.close()


if __name__ == "__main__":
    raise SystemExit(main())
