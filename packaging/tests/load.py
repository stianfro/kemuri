#!/usr/bin/env python3
"""Run a bounded Kemuri load test against local endpoints."""

from __future__ import annotations

import argparse
import datetime as dt
import http.server
import json
import math
import os
from pathlib import Path
import re
import signal
import socket
import socketserver
import sqlite3
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request


SCHEMA_VERSION = 1
DEFAULT_MAX_CHECKS = 1_000
DEFAULT_MAX_DURATION_SECONDS = 3_600


class FixtureHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        if self.path in ("/health", "/unhealthy"):
            status = 200 if self.path == "/health" else 503
            body = b"ok" if status == 200 else b"unhealthy"
            self.send_response(status)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except BrokenPipeError:
                pass
            return
        self.send_response(404)
        self.end_headers()

    def log_message(self, _format: str, *_args: object) -> None:
        return


class FixtureServer(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True
    request_queue_size = 1_024


class TcpFixtureHandler(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        return


class DnsUdpFixtureHandler(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        query, server = self.request
        server.sendto(dns_response(query), self.client_address)


class DnsTcpFixtureHandler(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        length_bytes = receive_exact(self.request, 2)
        if len(length_bytes) != 2:
            return
        query = receive_exact(self.request, int.from_bytes(length_bytes, "big"))
        response = dns_response(query)
        self.request.sendall(len(response).to_bytes(2, "big") + response)


class ThreadingTcpFixtureServer(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True
    request_queue_size = 1_024


class ThreadingUdpFixtureServer(socketserver.ThreadingUDPServer):
    daemon_threads = True
    allow_reuse_address = True


class FixtureSet:
    def __init__(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="kemuri-fixtures-")
        temporary_path = Path(self.temporary.name)
        self.certificate = temporary_path / "ca-cert.pem"
        ca_private_key = temporary_path / "ca-key.pem"
        server_certificate = temporary_path / "server-cert.pem"
        server_private_key = temporary_path / "server-key.pem"
        certificate_request = temporary_path / "server.csr"
        certificate_extensions = temporary_path / "server.ext"
        certificate_extensions.write_text(
            "\n".join(
                (
                    "basicConstraints=critical,CA:FALSE",
                    "keyUsage=critical,digitalSignature,keyEncipherment",
                    "extendedKeyUsage=serverAuth",
                    "subjectAltName=DNS:localhost,IP:127.0.0.1",
                )
            )
            + "\n"
        )
        subprocess.run(
            [
                "openssl",
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=Kemuri Load Test CA",
                "-addext",
                "basicConstraints=critical,CA:TRUE",
                "-keyout",
                str(ca_private_key),
                "-out",
                str(self.certificate),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        subprocess.run(
            [
                "openssl",
                "req",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-subj",
                "/CN=localhost",
                "-keyout",
                str(server_private_key),
                "-out",
                str(certificate_request),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        subprocess.run(
            [
                "openssl",
                "x509",
                "-req",
                "-in",
                str(certificate_request),
                "-CA",
                str(self.certificate),
                "-CAkey",
                str(ca_private_key),
                "-CAcreateserial",
                "-days",
                "1",
                "-sha256",
                "-extfile",
                str(certificate_extensions),
                "-out",
                str(server_certificate),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        tls_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls_context.load_cert_chain(server_certificate, server_private_key)
        self.http = FixtureServer(("127.0.0.1", 0), FixtureHandler)
        self.https = FixtureServer(("127.0.0.1", 0), FixtureHandler)
        self.https.socket = tls_context.wrap_socket(
            self.https.socket, server_side=True
        )
        self.tcp = ThreadingTcpFixtureServer(
            ("127.0.0.1", 0), TcpFixtureHandler
        )
        self.tls = ThreadingTcpFixtureServer(
            ("127.0.0.1", 0), TcpFixtureHandler
        )
        self.tls.socket = tls_context.wrap_socket(self.tls.socket, server_side=True)
        self.dns_udp = ThreadingUdpFixtureServer(
            ("127.0.0.1", 0), DnsUdpFixtureHandler
        )
        dns_port = int(self.dns_udp.server_address[1])
        self.dns_tcp = ThreadingTcpFixtureServer(
            ("127.0.0.1", dns_port), DnsTcpFixtureHandler
        )
        self.servers = (
            self.http,
            self.https,
            self.tcp,
            self.tls,
            self.dns_udp,
            self.dns_tcp,
        )
        self.closed_port = free_port()
        self.threads = [
            threading.Thread(target=server.serve_forever, daemon=True)
            for server in self.servers
        ]

    @property
    def ports(self) -> dict[str, int]:
        return {
            "http": int(self.http.server_address[1]),
            "https": int(self.https.server_address[1]),
            "tcp": int(self.tcp.server_address[1]),
            "tls": int(self.tls.server_address[1]),
            "dns": int(self.dns_udp.server_address[1]),
            "closed": self.closed_port,
        }

    def start(self) -> None:
        for thread in self.threads:
            thread.start()

    def close(self) -> None:
        for server in self.servers:
            server.shutdown()
        for server in self.servers:
            server.server_close()
        for thread in self.threads:
            thread.join(timeout=2)
        self.temporary.cleanup()


def receive_exact(connection: socket.socket, length: int) -> bytes:
    data = bytearray()
    while len(data) < length:
        part = connection.recv(length - len(data))
        if not part:
            break
        data.extend(part)
    return bytes(data)


def dns_response(query: bytes) -> bytes:
    end = 12
    while end < len(query) and query[end] != 0:
        end += query[end] + 1
    end = min(end + 5, len(query))
    response = bytearray(query[:end])
    if len(response) < 12:
        return bytes(response)
    response[2:4] = b"\x81\x80"
    response[6:12] = b"\x00\x01\x00\x00\x00\x00"
    response.extend(
        b"\xc0\x0c\x00\x01\x00\x01\x00\x00\x00\x3c\x00\x04\x7f\x00\x00\x01"
    )
    return bytes(response)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checks", type=int, default=100)
    parser.add_argument("--duration", type=int, default=10, help="run duration in seconds")
    parser.add_argument("--interval", default="5s")
    parser.add_argument("--concurrency", type=int, default=64)
    parser.add_argument(
        "--probe-concurrency",
        type=int,
        help="per-probe concurrency limit, defaults to the global limit",
    )
    parser.add_argument(
        "--probe",
        choices=("http", "https", "icmp", "tcp", "tls", "dns", "mixed"),
        default="http",
    )
    parser.add_argument(
        "--dns-protocol",
        choices=("udp", "tcp"),
        default="udp",
        help="DNS transport for DNS and mixed workloads",
    )
    parser.add_argument("--samples", type=int, default=20)
    parser.add_argument(
        "--failure-percent",
        type=int,
        default=0,
        help="percentage of checks that use a deterministic unhealthy fixture",
    )
    parser.add_argument("--binary", type=Path, default=Path("target/debug/kemuri"))
    parser.add_argument(
        "--output",
        default="auto",
        help="result JSON path, '-' for stdout only, or 'auto' for XDG state",
    )
    parser.add_argument(
        "--allow-large-run",
        action="store_true",
        help=f"allow more than {DEFAULT_MAX_CHECKS} checks",
    )
    parser.add_argument(
        "--allow-long-run",
        action="store_true",
        help=f"allow more than {DEFAULT_MAX_DURATION_SECONDS} seconds",
    )
    parser.add_argument("--max-rss-mib", type=int, default=1_024)
    parser.add_argument("--max-storage-mib", type=int, default=1_024)
    parser.add_argument("--min-available-memory-mib", type=int, default=512)
    parser.add_argument("--max-api-latency-seconds", type=float, default=2.0)
    args = parser.parse_args()
    if args.checks < 1:
        parser.error("--checks must be at least 1")
    if args.duration < 1:
        parser.error("--duration must be at least 1")
    if args.concurrency < 1:
        parser.error("--concurrency must be at least 1")
    if args.probe_concurrency is not None and args.probe_concurrency < 1:
        parser.error("--probe-concurrency must be at least 1")
    if args.samples < 1:
        parser.error("--samples must be at least 1")
    if not 0 <= args.failure_percent <= 100:
        parser.error("--failure-percent must be between 0 and 100")
    if args.checks > DEFAULT_MAX_CHECKS and not args.allow_large_run:
        parser.error(
            f"more than {DEFAULT_MAX_CHECKS} checks requires --allow-large-run"
        )
    if args.duration > DEFAULT_MAX_DURATION_SECONDS and not args.allow_long_run:
        parser.error(
            f"more than {DEFAULT_MAX_DURATION_SECONDS} seconds requires --allow-long-run"
        )
    for name in ("max_rss_mib", "max_storage_mib", "min_available_memory_mib"):
        if getattr(args, name) < 1:
            parser.error(f"--{name.replace('_', '-')} must be at least 1")
    if args.max_api_latency_seconds <= 0:
        parser.error("--max-api-latency-seconds must be greater than zero")
    if not args.binary.is_file():
        parser.error(f"Kemuri binary not found: {args.binary}")
    return args


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def yaml_string(value: str) -> str:
    return json.dumps(value)


def duration_seconds(value: str) -> float:
    match = re.fullmatch(r"([0-9]+(?:\.[0-9]+)?)(ms|s|m|h)", value)
    if not match:
        raise ValueError(f"unsupported interval for load validation: {value}")
    amount = float(match.group(1))
    scale = {"ms": 0.001, "s": 1.0, "m": 60.0, "h": 3600.0}
    return amount * scale[match.group(2)]


def write_config(
    path: Path,
    database: Path,
    server_port: int,
    fixture_ports: dict[str, int],
    checks: int,
    interval: str,
    concurrency: int,
    probe: str,
    samples: int,
    dns_protocol: str,
    failure_percent: int,
    root_certificate: Path,
    probe_concurrency: int | None,
) -> None:
    probe_counts = workload_probe_counts(probe, checks)
    scheduler_counts = {
        "icmp": probe_counts.get("icmp", 0),
        "http": probe_counts.get("http", 0) + probe_counts.get("https", 0),
        "tcp": probe_counts.get("tcp", 0) + probe_counts.get("tls", 0),
        "dns": probe_counts.get("dns", 0),
    }
    effective_probe_concurrency = probe_concurrency or concurrency
    probe_limits = {
        kind: min(effective_probe_concurrency, count)
        for kind, count in scheduler_counts.items()
        if count > 0
    }
    lines = [
        "version: 1",
        "server:",
        "  bind: 127.0.0.1",
        f"  port: {server_port}",
        "  shutdown_timeout: 10s",
        "storage:",
        f"  path: {yaml_string(str(database))}",
        "  disk_pressure:",
        "    warning_free: 0.5%",
        "    critical_free: 0.1%",
        "scheduler:",
        "  startup_mode: immediate_then_aligned",
        "  default_jitter: 10%",
        "  tick_interval: 50ms",
        f"  max_concurrent: {concurrency}",
        "  max_concurrent_by_probe:",
        "profiles:",
    ]
    profiles_index = lines.index("profiles:")
    lines[profiles_index:profiles_index] = [
        f"    {kind}: {limit}" for kind, limit in probe_limits.items()
    ]
    if probe_counts.get("http", 0):
        lines.extend(
            [
                "  - kind: http",
                "    id: load-http",
                f"    url: http://127.0.0.1:{fixture_ports['http']}/health",
                f"    interval: {yaml_string(interval)}",
                "    timeout: 2s",
                "    expected_status: 200",
                "    measure_until: headers",
            ]
        )
    if probe_counts.get("https", 0):
        lines.extend(
            [
                "  - kind: http",
                "    id: load-https",
                f"    url: https://127.0.0.1:{fixture_ports['https']}/health",
                f"    interval: {yaml_string(interval)}",
                "    timeout: 2s",
                "    expected_status: 200",
                "    measure_until: body",
                "    tls_validate: true",
                "    root_certificates:",
                f"      - {yaml_string(str(root_certificate))}",
            ]
        )
    if probe_counts.get("icmp", 0):
        lines.extend(
            [
                "  - kind: icmp",
                "    id: load-icmp",
                f"    interval: {yaml_string(interval)}",
                "    timeout: 5s",
                f"    count: {samples}",
                "    address_family: ipv4",
                "    payload_size: 56",
            ]
        )
    if probe_counts.get("tcp", 0):
        lines.extend(
            [
                "  - kind: tcp",
                "    id: load-tcp",
                "    host: 127.0.0.1",
                f"    port: {fixture_ports['tcp']}",
                f"    interval: {yaml_string(interval)}",
                "    timeout: 2s",
                "    address_family: ipv4",
            ]
        )
    if probe_counts.get("tls", 0):
        lines.extend(
            [
                "  - kind: tcp",
                "    id: load-tls",
                "    host: 127.0.0.1",
                f"    port: {fixture_ports['tls']}",
                f"    interval: {yaml_string(interval)}",
                "    timeout: 2s",
                "    address_family: ipv4",
                "    tls:",
                "      enabled: true",
                "      server_name: localhost",
                "      tls_validate: true",
                "      root_certificates:",
                f"        - {yaml_string(str(root_certificate))}",
            ]
        )
    if probe_counts.get("dns", 0):
        lines.extend(
            [
                "  - kind: dns",
                "    id: load-dns",
                "    name: fixture.test",
                f"    server: 127.0.0.1:{fixture_ports['dns']}",
                "    record_type: A",
                f"    protocol: {dns_protocol}",
                "    expected_rcode: noerror",
                "    require_answer: true",
                f"    interval: {yaml_string(interval)}",
                "    timeout: 2s",
            ]
        )
    lines.append("targets:")
    failed_counts = workload_failure_counts(probe_counts, failure_percent)
    index = 0
    for kind, count in probe_counts.items():
        for kind_index in range(count):
            failed = kind_index < failed_counts[kind]
            address = "192.0.2.1" if kind == "icmp" and failed else "127.0.0.1"
            lines.extend(
                [
                    f"  - id: load-{index:06d}",
                    f"    address: {address}",
                    f"    group_path: load/{kind}",
                    "    checks:",
                    f"      - id: {kind}",
                    f"        profile: load-{kind}",
                ]
            )
            if failed:
                if kind in ("http", "https"):
                    scheme = kind
                    lines.append(
                        f"        url: {scheme}://127.0.0.1:{fixture_ports[kind]}/unhealthy"
                    )
                elif kind == "tcp":
                    lines.append(f"        port: {fixture_ports['closed']}")
                elif kind == "tls":
                    lines.append(f"        port: {fixture_ports['tcp']}")
                elif kind == "dns":
                    lines.append("        expected_rcode: nxdomain")
            index += 1
    path.write_text("\n".join(lines) + "\n")


def workload_probe_counts(probe: str, checks: int) -> dict[str, int]:
    if probe != "mixed":
        return {probe: checks}
    weights = (
        ("icmp", 70),
        ("http", 5),
        ("https", 5),
        ("tcp", 5),
        ("tls", 5),
        ("dns", 10),
    )
    counts = {kind: checks * weight // 100 for kind, weight in weights}
    assigned = sum(counts.values())
    for kind, _ in weights:
        if assigned >= checks:
            break
        counts[kind] += 1
        assigned += 1
    return counts


def workload_failure_counts(
    probe_counts: dict[str, int], failure_percent: int
) -> dict[str, int]:
    total_checks = sum(probe_counts.values())
    desired_failures = math.ceil(total_checks * failure_percent / 100)
    exact = {
        kind: count * failure_percent / 100
        for kind, count in probe_counts.items()
    }
    failures = {kind: math.floor(value) for kind, value in exact.items()}
    remaining = desired_failures - sum(failures.values())
    order = {kind: index for index, kind in enumerate(probe_counts)}
    ranked = sorted(
        probe_counts,
        key=lambda kind: (exact[kind] - failures[kind], -order[kind]),
        reverse=True,
    )
    for kind in ranked[:remaining]:
        failures[kind] += 1
    return failures


def get(url: str, timeout: float = 2.0) -> tuple[int, bytes, float]:
    started = time.monotonic()
    try:
        with urllib.request.urlopen(url, timeout=timeout) as response:
            return response.status, response.read(), time.monotonic() - started
    except urllib.error.HTTPError as error:
        return error.code, error.read(), time.monotonic() - started


def wait_ready(base_url: str, process: subprocess.Popen[bytes]) -> None:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Kemuri exited before readiness with {process.returncode}")
        try:
            status, _, _ = get(f"{base_url}/readyz", timeout=0.5)
            if status == 200:
                return
        except (OSError, urllib.error.URLError):
            pass
        time.sleep(0.05)
    raise RuntimeError("Kemuri did not become ready in 30 seconds")


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, math.ceil(len(ordered) * fraction) - 1)
    return ordered[index]


def process_sample(pid: int) -> dict[str, int]:
    status: dict[str, int] = {}
    status_path = Path(f"/proc/{pid}/status")
    if not status_path.exists():
        return status
    for line in status_path.read_text().splitlines():
        if line.startswith("VmRSS:"):
            status["rss_bytes"] = int(line.split()[1]) * 1024
        elif line.startswith("Threads:"):
            status["threads"] = int(line.split()[1])
    return status


def available_memory_bytes() -> int | None:
    for line in Path("/proc/meminfo").read_text().splitlines():
        if line.startswith("MemAvailable:"):
            return int(line.split()[1]) * 1024
    return None


def storage_bytes(database: Path) -> int:
    return sum(
        path.stat().st_size
        for path in (database, Path(f"{database}-wal"), Path(f"{database}-shm"))
        if path.exists()
    )


def metric_sum(metrics_text: str, metric_name: str) -> float | None:
    values = []
    for line in metrics_text.splitlines():
        if line.startswith("#"):
            continue
        name, separator, value = line.rpartition(" ")
        if not separator:
            continue
        base_name = name.split("{", 1)[0]
        if base_name == metric_name:
            values.append(float(value))
    return sum(values) if values else None


def git_commit(root: Path) -> str:
    return subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def git_dirty(root: Path) -> bool:
    return bool(
        subprocess.run(
            ["git", "status", "--porcelain"],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )


def validate_database(
    database: Path, expected_checks: int, interval: str, failure_percent: int
) -> dict[str, object]:
    connection = sqlite3.connect(database)
    try:
        integrity = connection.execute("PRAGMA integrity_check").fetchone()[0]
        foreign_key_errors = connection.execute("PRAGMA foreign_key_check").fetchall()
        active_checks = connection.execute(
            "SELECT count(*) FROM checks WHERE active = 1"
        ).fetchone()[0]
        rounds = connection.execute("SELECT count(*) FROM rounds").fetchone()[0]
        duplicate_slots = connection.execute(
            """
            SELECT count(*) FROM (
                SELECT check_internal_id, observer_internal_id, scheduled_at
                FROM rounds
                GROUP BY check_internal_id, observer_internal_id, scheduled_at
                HAVING count(*) > 1
            )
            """
        ).fetchone()[0]
        checks_without_rounds = connection.execute(
            """
            SELECT count(*)
            FROM checks c
            WHERE c.active = 1
              AND NOT EXISTS (
                SELECT 1 FROM rounds r WHERE r.check_internal_id = c.internal_id
              )
            """
        ).fetchone()[0]
        status_counts = dict(
            connection.execute(
                "SELECT execution_status, count(*) FROM rounds GROUP BY execution_status"
            ).fetchall()
        )
        sample_totals = connection.execute(
            """
            SELECT
                coalesce(sum(healthy_samples), 0),
                coalesce(sum(unhealthy_samples), 0),
                coalesce(sum(measurement_loss_samples), 0)
            FROM rounds
            """
        ).fetchone()
        profile_sample_totals = {
            row[0]: {
                "rounds": row[1],
                "attempted_samples": row[2],
                "healthy_samples": row[3],
                "unhealthy_samples": row[4],
                "measurement_loss_samples": row[5],
            }
            for row in connection.execute(
                """
                SELECT
                    c.profile_id,
                    count(*),
                    coalesce(sum(r.attempted_samples), 0),
                    coalesce(sum(r.healthy_samples), 0),
                    coalesce(sum(r.unhealthy_samples), 0),
                    coalesce(sum(r.measurement_loss_samples), 0)
                FROM rounds r
                JOIN checks c ON c.internal_id = r.check_internal_id
                GROUP BY c.profile_id
                ORDER BY c.profile_id
                """
            ).fetchall()
        }
        dispatch_delays = [
            max(0.0, float(row[0]))
            for row in connection.execute(
                """
                SELECT (julianday(started_at) - julianday(scheduled_at)) * 86400.0
                FROM rounds
                WHERE execution_status IN ('complete', 'partial', 'internal_error')
                  AND started_at IS NOT NULL
                """
            ).fetchall()
        ]
        interval_value = duration_seconds(interval)
        missing_slot_gaps = connection.execute(
            """
            SELECT count(*)
            FROM (
                SELECT (
                    julianday(scheduled_at)
                    - julianday(lag(scheduled_at) OVER (
                        PARTITION BY check_internal_id, observer_internal_id
                        ORDER BY scheduled_at
                    ))
                ) * 86400.0 AS gap_seconds
                FROM rounds
            )
            WHERE gap_seconds > ?
            """,
            (interval_value * 1.5,),
        ).fetchone()[0]
    finally:
        connection.close()

    failures = []
    if integrity != "ok":
        failures.append(f"SQLite integrity check returned {integrity!r}")
    if foreign_key_errors:
        failures.append(f"SQLite has {len(foreign_key_errors)} foreign-key errors")
    if active_checks != expected_checks:
        failures.append(f"expected {expected_checks} active checks, found {active_checks}")
    if duplicate_slots:
        failures.append(f"found {duplicate_slots} duplicate scheduled slots")
    if checks_without_rounds:
        failures.append(f"{checks_without_rounds} active checks have no stored round")
    if missing_slot_gaps:
        failures.append(f"found {missing_slot_gaps} gaps larger than one scheduled slot")
    if status_counts.get("internal_error", 0):
        failures.append(
            f"found {status_counts['internal_error']} internally failed rounds"
        )
    healthy_samples, unhealthy_samples, measurement_loss_samples = sample_totals
    failed_samples = unhealthy_samples + measurement_loss_samples
    if failure_percent > 0 and failed_samples == 0:
        failures.append("failure scenario did not record an unhealthy or lost sample")
    if failure_percent == 0 and failed_samples > 0:
        failures.append(
            f"healthy scenario recorded {failed_samples} unhealthy or lost samples"
        )
    if failure_percent < 100 and healthy_samples == 0:
        failures.append("scenario did not record a healthy sample")
    dispatch_limit = max(1.0, interval_value * 0.05)
    late_rounds = sum(delay > dispatch_limit for delay in dispatch_delays)
    on_time_ratio = (
        (len(dispatch_delays) - late_rounds) / len(dispatch_delays)
        if dispatch_delays
        else 0.0
    )
    if on_time_ratio < 0.999:
        failures.append(
            f"only {on_time_ratio:.3%} of dispatched rounds met the dispatch limit"
        )

    return {
        "integrity": integrity,
        "foreign_key_errors": len(foreign_key_errors),
        "active_checks": active_checks,
        "rounds": rounds,
        "duplicate_slots": duplicate_slots,
        "checks_without_rounds": checks_without_rounds,
        "missing_slot_gaps": missing_slot_gaps,
        "execution_status_counts": status_counts,
        "healthy_samples": healthy_samples,
        "unhealthy_samples": unhealthy_samples,
        "measurement_loss_samples": measurement_loss_samples,
        "profile_sample_totals": profile_sample_totals,
        "dispatch_limit_seconds": dispatch_limit,
        "dispatch_delay_p99_seconds": percentile(dispatch_delays, 0.99),
        "dispatched_rounds": len(dispatch_delays),
        "late_dispatched_rounds": late_rounds,
        "on_time_dispatch_ratio": on_time_ratio,
        "failures": failures,
    }


def output_path(value: str, started: dt.datetime) -> Path | None:
    if value == "-":
        return None
    if value != "auto":
        return Path(value)
    state_home = Path(
        os.environ.get("XDG_STATE_HOME", str(Path.home() / ".local" / "state"))
    )
    stamp = started.strftime("%Y%m%dT%H%M%SZ")
    return state_home / "kemuri" / "load-tests" / f"{stamp}.json"


def main() -> int:
    args = parse_args()
    root = Path(__file__).resolve().parents[2]
    binary = args.binary.resolve()
    started = dt.datetime.now(dt.timezone.utc)
    probe_counts = workload_probe_counts(args.probe, args.checks)
    fixture = FixtureSet()
    fixture.start()
    server_port = free_port()

    process: subprocess.Popen[bytes] | None = None
    result: dict[str, object]
    with tempfile.TemporaryDirectory(prefix="kemuri-load-") as temporary:
        work = Path(temporary)
        database = work / "kemuri.db"
        config = work / "kemuri.yaml"
        log = work / "kemuri.log"
        write_config(
            config,
            database,
            server_port,
            fixture.ports,
            args.checks,
            args.interval,
            min(args.concurrency, args.checks),
            args.probe,
            args.samples,
            args.dns_protocol,
            args.failure_percent,
            fixture.certificate,
            args.probe_concurrency,
        )
        base_url = f"http://127.0.0.1:{server_port}"
        api_latencies: list[float] = []
        resource_samples: list[dict[str, int]] = []
        metrics_text = ""
        run_error: str | None = None
        shutdown_clean = False

        with log.open("wb") as log_file:
            process = subprocess.Popen(
                [str(binary), "serve", "--config", str(config)],
                cwd=root,
                stdout=log_file,
                stderr=subprocess.STDOUT,
            )
            try:
                wait_ready(base_url, process)
                deadline = time.monotonic() + args.duration
                endpoints = [
                    "/api/v1/targets?limit=200",
                    "/api/v1/system/status",
                    "/metrics",
                ]
                request_index = 0
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise RuntimeError(
                            f"Kemuri exited during the run with {process.returncode}"
                        )
                    endpoint = endpoints[request_index % len(endpoints)]
                    request_index += 1
                    status, _, elapsed = get(f"{base_url}{endpoint}")
                    if status != 200:
                        raise RuntimeError(f"{endpoint} returned HTTP {status}")
                    if elapsed > args.max_api_latency_seconds:
                        raise RuntimeError(
                            f"{endpoint} exceeded the API latency safety limit"
                        )
                    api_latencies.append(elapsed)
                    sample = process_sample(process.pid)
                    resource_samples.append(sample)
                    if sample.get("rss_bytes", 0) > args.max_rss_mib * 1024 * 1024:
                        raise RuntimeError("Kemuri exceeded the RSS safety limit")
                    available = available_memory_bytes()
                    if (
                        available is not None
                        and available < args.min_available_memory_mib * 1024 * 1024
                    ):
                        raise RuntimeError("host available memory crossed the safety limit")
                    if storage_bytes(database) > args.max_storage_mib * 1024 * 1024:
                        raise RuntimeError("Kemuri storage crossed the safety limit")
                    time.sleep(0.1)
                status, metrics_body, _ = get(f"{base_url}/metrics")
                if status != 200:
                    raise RuntimeError(f"/metrics returned HTTP {status}")
                metrics_text = metrics_body.decode(errors="replace")
            except Exception as error:  # Keep the result artifact on a failed run.
                run_error = str(error)
            finally:
                if process.poll() is None:
                    process.send_signal(signal.SIGTERM)
                    try:
                        process.wait(timeout=12)
                        shutdown_clean = process.returncode == 0
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()

        database_result: dict[str, object]
        if database.exists():
            database_result = validate_database(
                database, args.checks, args.interval, args.failure_percent
            )
        else:
            database_result = {"failures": ["Kemuri did not create the database"]}
        log_text = log.read_text(errors="replace")
        log_failures = [
            marker
            for marker in (
                "panicked at",
                "writer task exited unexpectedly",
                "failed to write round result",
            )
            if marker in log_text
        ]
        rss_values = [
            sample["rss_bytes"] for sample in resource_samples if "rss_bytes" in sample
        ]
        thread_values = [
            sample["threads"] for sample in resource_samples if "threads" in sample
        ]
        failures = list(database_result.get("failures", []))
        if run_error:
            failures.append(run_error)
        if not shutdown_clean:
            failures.append("Kemuri did not stop cleanly")
        failures.extend(f"log contains {marker!r}" for marker in log_failures)
        for forbidden_label in ('target_id="', 'check_id="'):
            if forbidden_label in metrics_text:
                failures.append(
                    f"metrics contain forbidden high-cardinality label {forbidden_label[:-1]!r}"
                )
        api_p95 = percentile(api_latencies, 0.95)
        if api_p95 is not None and api_p95 > 0.5:
            failures.append(f"API p95 was {api_p95:.3f} seconds, above 0.5 seconds")

        finished = dt.datetime.now(dt.timezone.utc)
        result = {
            "schema_version": SCHEMA_VERSION,
            "status": "passed" if not failures else "failed",
            "git_commit": git_commit(root),
            "source_dirty": git_dirty(root),
            "started_at": started.isoformat(),
            "finished_at": finished.isoformat(),
            "environment": {
                "class": "development-host",
                "platform": sys.platform,
                "storage_profile": "host-local",
            },
            "workload": {
                "probe": args.probe,
                "probe_counts": probe_counts,
                "failed_check_counts": workload_failure_counts(
                    probe_counts, args.failure_percent
                ),
                "dns_protocol": (
                    args.dns_protocol
                    if probe_counts.get("dns", 0)
                    else None
                ),
                "failure_percent": args.failure_percent,
                "samples_per_round": (
                    args.samples if args.probe == "icmp" else 1
                    if args.probe != "mixed"
                    else None
                ),
                "samples_per_icmp_round": (
                    args.samples
                    if probe_counts.get("icmp", 0)
                    else None
                ),
                "checks": args.checks,
                "interval": args.interval,
                "duration_seconds": args.duration,
                "max_concurrent": min(args.concurrency, args.checks),
                "max_concurrent_by_probe": (
                    min(args.probe_concurrency, args.checks)
                    if args.probe_concurrency is not None
                    else min(args.concurrency, args.checks)
                ),
            },
            "safety_limits": {
                "maximum_checks_without_override": DEFAULT_MAX_CHECKS,
                "maximum_duration_seconds_without_override": (
                    DEFAULT_MAX_DURATION_SECONDS
                ),
                "max_rss_bytes": args.max_rss_mib * 1024 * 1024,
                "max_storage_bytes": args.max_storage_mib * 1024 * 1024,
                "min_available_memory_bytes": (
                    args.min_available_memory_mib * 1024 * 1024
                ),
                "max_api_latency_seconds": args.max_api_latency_seconds,
            },
            "api": {
                "requests": len(api_latencies),
                "p50_ms": (
                    round((percentile(api_latencies, 0.50) or 0) * 1000, 3)
                    if api_latencies
                    else None
                ),
                "p95_ms": (
                    round((api_p95 or 0) * 1000, 3)
                    if api_latencies
                    else None
                ),
                "p99_ms": (
                    round((percentile(api_latencies, 0.99) or 0) * 1000, 3)
                    if api_latencies
                    else None
                ),
            },
            "process": {
                "peak_rss_bytes": max(rss_values, default=None),
                "peak_threads": max(thread_values, default=None),
                "exit_code": process.returncode if process else None,
                "shutdown_clean": shutdown_clean,
            },
            "metrics": {
                "active_checks": metric_sum(
                    metrics_text, "kemuri_scheduler_active_checks"
                ),
                "in_flight": metric_sum(
                    metrics_text, "kemuri_scheduler_in_flight"
                ),
                "queue_depth": metric_sum(
                    metrics_text, "kemuri_scheduler_queue_depth"
                ),
                "writer_queue_depth": metric_sum(
                    metrics_text, "kemuri_writer_queue_depth"
                ),
                "rounds_dispatched": metric_sum(
                    metrics_text, "kemuri_scheduler_rounds_total"
                ),
                "rounds_skipped_overlap": metric_sum(
                    metrics_text, "kemuri_scheduler_rounds_skipped_overlap"
                ),
                "storage_writes": metric_sum(
                    metrics_text, "kemuri_storage_writes_total"
                ),
            },
            "storage": {
                **database_result,
                "database_size_bytes": database.stat().st_size if database.exists() else 0,
            },
            "failures": failures,
        }

    fixture.close()
    destination = output_path(args.output, started)
    rendered = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if destination is not None:
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(rendered)
        print(destination)
    else:
        print(rendered, end="")
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130) from None
