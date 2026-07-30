#!/usr/bin/env python3
"""Generate and exercise deterministic Kemuri history databases.

The default data set is intentionally small enough for CI. Use a shorter
cadence and more targets for an opt-in scale run. Generated databases are
ignored by the repository's ``*.db`` rule.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


GENERATOR_VERSION = 1
VALID_MONTHS = (1, 6, 12)
DEFAULT_ANCHOR = dt.datetime(2026, 1, 1, tzinfo=dt.timezone.utc)
ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "crates" / "kemuri-storage" / "migrations"


def timestamp(value: dt.datetime) -> str:
    return (
        value.astimezone(dt.timezone.utc)
        .isoformat(timespec="seconds")
        .replace("+00:00", "Z")
    )


def create_schema(connection: sqlite3.Connection) -> None:
    connection.execute(
        """CREATE TABLE _sqlx_migrations (
        version BIGINT PRIMARY KEY, description TEXT NOT NULL,
        installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        success BOOLEAN NOT NULL, checksum BLOB NOT NULL,
        execution_time BIGINT NOT NULL)"""
    )
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        version_text, description = migration.stem.split("_", 1)
        contents = migration.read_bytes()
        connection.executescript(contents.decode())
        connection.execute(
            """INSERT INTO _sqlx_migrations
            (version, description, installed_on, success, checksum, execution_time)
            VALUES (?, ?, ?, 1, ?, 0)""",
            (
                int(version_text),
                description.replace("_", " "),
                timestamp(DEFAULT_ANCHOR),
                hashlib.sha384(contents).digest(),
            ),
        )
    connection.execute(
        """CREATE TABLE history_scale_metadata (
        generator_version INTEGER NOT NULL, months INTEGER NOT NULL,
        anchor TEXT NOT NULL, cadence_minutes INTEGER NOT NULL,
        targets INTEGER NOT NULL, checks_per_target INTEGER NOT NULL)"""
    )


def generate_database(
    output: Path,
    *,
    months: int,
    anchor: dt.datetime = DEFAULT_ANCHOR,
    cadence_minutes: int = 360,
    targets: int = 12,
    checks_per_target: int = 2,
    max_rounds: int = 250_000,
) -> dict[str, int | str]:
    """Create a deterministic database and return its expected row counts."""
    if months not in VALID_MONTHS:
        raise ValueError(f"months must be one of {VALID_MONTHS}")
    if min(cadence_minutes, targets, checks_per_target) < 1:
        raise ValueError("cadence, targets, and checks must be positive")
    expected_rounds = (
        months * 30 * 24 * 60 // cadence_minutes * targets * checks_per_target
    )
    if expected_rounds > max_rounds:
        raise ValueError(
            f"requested {expected_rounds} rounds exceeds --max-rounds {max_rounds}; "
            "raise the limit explicitly for an opt-in scale run"
        )
    if output.exists():
        output.unlink()
    output.parent.mkdir(parents=True, exist_ok=True)

    connection = sqlite3.connect(output)
    try:
        connection.execute("PRAGMA journal_mode=DELETE")
        connection.execute("PRAGMA foreign_keys=ON")
        create_schema(connection)
        connection.execute(
            "INSERT INTO history_scale_metadata VALUES (?, ?, ?, ?, ?, ?)",
            (
                GENERATOR_VERSION,
                months,
                timestamp(anchor),
                cadence_minutes,
                targets,
                checks_per_target,
            ),
        )
        start = anchor - dt.timedelta(days=months * 30)
        connection.execute(
            """INSERT INTO observers
            (internal_id, observer_id, status, first_seen_at, last_seen_at)
            VALUES (1, 'local', 'active', ?, ?)""",
            (timestamp(start), timestamp(anchor)),
        )

        step = dt.timedelta(minutes=cadence_minutes)
        observation_count = int((anchor - start) / step)
        check_internal_id = 0
        round_rows: list[tuple[object, ...]] = []
        rollup_rows: dict[
            tuple[int, int, str], list[int]
        ] = {}
        for target_index in range(targets):
            target_id = f"history-{target_index:04d}"
            target_internal_id = target_index + 1
            connection.execute(
                """INSERT INTO targets
                (internal_id, target_id, name, group_path, labels, active,
                 first_seen_at, last_seen_at)
                VALUES (?, ?, ?, ?, ?, 1, ?, ?)""",
                (
                    target_internal_id,
                    target_id,
                    f"History target {target_index:04d}",
                    f"scale/group-{target_index % 4}",
                    json.dumps({"fixture": "history-scale"}, separators=(",", ":")),
                    timestamp(start),
                    timestamp(anchor),
                ),
            )
            for check_index in range(checks_per_target):
                check_internal_id += 1
                check_id = f"check-{check_index:03d}"
                revision_id = f"history-v{GENERATOR_VERSION}"
                connection.execute(
                    """INSERT INTO checks
                    (internal_id, target_internal_id, check_id, probe_type, active,
                     current_revision_id, first_seen_at, last_seen_at, profile_id,
                     config_generation, redacted_resolved_config, observer_assignment)
                    VALUES (?, ?, ?, 'http', 1, ?, ?, ?, 'history-http',
                            'history-scale', '{}', 'local')""",
                    (
                        check_internal_id,
                        target_internal_id,
                        check_id,
                        revision_id,
                        timestamp(start),
                        timestamp(anchor),
                    ),
                )
                connection.execute(
                    "INSERT INTO check_assignments VALUES (?, 1, 1, ?)",
                    (check_internal_id, timestamp(start)),
                )
                connection.execute(
                    """INSERT INTO check_revisions
                    (check_internal_id, revision_id, effective_at, redacted_config)
                    VALUES (?, ?, ?, '{}')""",
                    (check_internal_id, revision_id, timestamp(start)),
                )
                connection.execute(
                    """INSERT INTO check_current_state
                    (check_internal_id, observer_internal_id, state, last_round_at,
                     last_latency_ns, last_measurement_loss_ratio,
                     last_health_failure_ratio, updated_at)
                    VALUES (?, 1, 'healthy', ?, 12000000, 0.0, 0.0, ?)""",
                    (check_internal_id, timestamp(anchor - step), timestamp(anchor)),
                )

                for observation_index in range(observation_count):
                    observed_at = start + observation_index * step
                    partial = observation_index % 17 == 0
                    unhealthy = observation_index % 29 == 0
                    attempted = 2 if partial else 3
                    healthy = attempted - int(unhealthy)
                    execution_status = "partial" if partial else "completed"
                    latency = 10_000_000 + (
                        (target_index * 97 + check_index * 31 + observation_index) % 500
                    ) * 10_000
                    observed = timestamp(observed_at)
                    round_rows.append(
                        (
                            check_internal_id,
                            observed,
                            execution_status,
                            3,
                            attempted,
                            attempted,
                            healthy,
                            int(unhealthy),
                            0,
                            latency,
                            latency,
                            latency,
                            json.dumps(
                                {"healthy": healthy, "unhealthy": int(unhealthy)},
                                separators=(",", ":"),
                            ),
                            revision_id,
                            observed,
                        )
                    )
                    for resolution in (300, 3600):
                        bucket_seconds = (
                            int(observed_at.timestamp()) // resolution * resolution
                        )
                        bucket = timestamp(
                            dt.datetime.fromtimestamp(
                                bucket_seconds, tz=dt.timezone.utc
                            )
                        )
                        key = (check_internal_id, resolution, bucket)
                        aggregate = rollup_rows.setdefault(
                            key,
                            [
                                0,
                                0,
                                0,
                                0,
                                0,
                                0,
                                0,
                                0,
                                latency,
                                latency,
                                0,
                            ],
                        )
                        aggregate[0] += 1
                        aggregate[1] += int(not partial)
                        aggregate[2] += int(partial)
                        aggregate[3] += 3
                        aggregate[4] += attempted
                        aggregate[5] += attempted
                        aggregate[6] += healthy
                        aggregate[7] += int(unhealthy)
                        aggregate[8] = min(aggregate[8], latency)
                        aggregate[9] = max(aggregate[9], latency)
                        aggregate[10] += latency * attempted

        connection.executemany(
            """INSERT INTO rounds
            (check_internal_id, observer_internal_id, scheduled_at, started_at,
             finished_at, execution_status, configured_samples, attempted_samples,
             latency_bearing_samples, healthy_samples, unhealthy_samples,
             measurement_loss_samples, min_latency_ns, median_latency_ns,
             max_latency_ns, outcome_summary, config_generation, check_revision_id,
             created_at)
            VALUES (?, 1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'history-scale', ?, ?)""",
            [
                (
                    row[0],
                    row[1],
                    row[1],
                    row[1],
                    row[2],
                    *row[3:],
                )
                for row in round_rows
            ],
        )
        connection.executemany(
            """INSERT INTO rollups
            (check_internal_id, observer_internal_id, resolution_seconds,
             bucket_start, scheduled_rounds, completed_rounds, partial_rounds,
             configured_sample_slots, attempted_samples, latency_bearing_samples,
             healthy_samples, unhealthy_samples, measurement_loss_samples,
             outcome_counts, min_latency_ns, max_latency_ns, sum_latency_ns,
             histogram_version, no_data_counts)
            VALUES (?, 1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, '{}', ?, ?, ?, 1, '{}')""",
            [
                (
                    *key,
                    *aggregate,
                )
                for key, aggregate in sorted(rollup_rows.items())
            ],
        )
        connection.commit()
        connection.execute("VACUUM")
    finally:
        connection.close()
    return {
        "generator_version": GENERATOR_VERSION,
        "months": months,
        "targets": targets,
        "checks": targets * checks_per_target,
        "rounds": targets * checks_per_target * observation_count,
        "rollups": len(rollup_rows),
        "anchor": timestamp(anchor),
    }


def verify_database(path: Path, *, retention_days: int = 7) -> dict[str, object]:
    """Check fixture invariants, backup, and retention safety on a copy."""
    connection = sqlite3.connect(path)
    try:
        integrity = connection.execute("PRAGMA integrity_check").fetchone()[0]
        metadata = connection.execute(
            """SELECT generator_version, months, anchor, cadence_minutes,
            targets, checks_per_target FROM history_scale_metadata"""
        ).fetchone()
        if integrity != "ok" or metadata is None:
            raise RuntimeError(f"database integrity check failed: {integrity}")
        version, months, anchor_text, _, targets, checks_per_target = metadata
        if version != GENERATOR_VERSION:
            raise RuntimeError(f"unsupported generator version: {version}")
        counts = {
            table: connection.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
            for table in ("targets", "checks", "rounds", "rollups")
        }
        if counts["targets"] != targets or counts["checks"] != targets * checks_per_target:
            raise RuntimeError("fixture entity counts do not match metadata")
        resolutions = {
            row[0]
            for row in connection.execute(
                "SELECT DISTINCT resolution_seconds FROM rollups"
            )
        }
        statuses = {
            row[0]
            for row in connection.execute(
                "SELECT DISTINCT execution_status FROM rounds"
            )
        }
        complete_rollups = connection.execute(
            """SELECT COUNT(*) FROM rollups
            WHERE completed_rounds > 0 AND partial_rounds = 0"""
        ).fetchone()[0]
        partial_rollups = connection.execute(
            "SELECT COUNT(*) FROM rollups WHERE partial_rounds > 0"
        ).fetchone()[0]
        if (
            resolutions != {300, 3600}
            or statuses != {"completed", "partial"}
            or complete_rollups == 0
            or partial_rollups == 0
        ):
            raise RuntimeError("raw or rollup fixture classes are incomplete")

        with tempfile.TemporaryDirectory(prefix="kemuri-history-verify-") as temporary:
            backup_path = Path(temporary) / "backup.db"
            backup = sqlite3.connect(backup_path)
            connection.backup(backup)
            backup.close()
            backup_check = sqlite3.connect(backup_path)
            backup_integrity = backup_check.execute("PRAGMA integrity_check").fetchone()[0]
            backup_check.close()

            retention_path = Path(temporary) / "retention.db"
            shutil.copy2(path, retention_path)
            retention = sqlite3.connect(retention_path)
            anchor = dt.datetime.fromisoformat(anchor_text.replace("Z", "+00:00"))
            cutoff = timestamp(anchor - dt.timedelta(days=retention_days))
            before = retention.execute(
                "SELECT count(*) FROM rounds WHERE scheduled_at < ?", (cutoff,)
            ).fetchone()[0]
            cursor = retention.execute(
                """DELETE FROM rounds WHERE scheduled_at < ? AND EXISTS (
                SELECT 1 FROM rollups ru
                WHERE ru.check_internal_id = rounds.check_internal_id
                  AND ru.observer_internal_id = rounds.observer_internal_id
                  AND ru.resolution_seconds = 300
                  AND CAST(strftime('%s', rounds.scheduled_at) AS INTEGER)
                      >= CAST(strftime('%s', ru.bucket_start) AS INTEGER)
                  AND CAST(strftime('%s', rounds.scheduled_at) AS INTEGER)
                      < CAST(strftime('%s', ru.bucket_start) AS INTEGER) + 300)""",
                (cutoff,),
            )
            retention.commit()
            retention.close()
        if backup_integrity != "ok" or cursor.rowcount != before:
            raise RuntimeError("backup or covered-raw retention check failed")
        return {
            "integrity": integrity,
            "backup_integrity": backup_integrity,
            "retention_rows_deleted": cursor.rowcount,
            "counts": counts,
        }
    finally:
        connection.close()


def write_config(
    path: Path,
    database: Path,
    *,
    targets: int,
    checks_per_target: int,
    port: int,
) -> None:
    lines = [
        "version: 1",
        "server:",
        "  bind: 127.0.0.1",
        f"  port: {port}",
        f"  public_url: http://127.0.0.1:{port}",
        "  shutdown_timeout: 3s",
        "storage:",
        f"  path: {database.resolve()}",
        "  retention:",
        "    raw_rounds: forever",
        "    rollup_5m: forever",
        "    rollup_1h: forever",
        "    alert_events: forever",
        "    notification_records: forever",
        "  disk_pressure:",
        "    warning_free: 99.999%",
        "    critical_free: 99.99%",
        "profiles:",
        "  - kind: http",
        "    id: history-http",
        "    url: http://127.0.0.1:1/",
        "    interval: 24h",
        "    timeout: 1s",
        "targets:",
    ]
    for target_index in range(targets):
        lines.extend(
            [
                f"  - id: history-{target_index:04d}",
                "    address: 127.0.0.1",
                f"    name: History target {target_index:04d}",
                f"    group_path: scale/group-{target_index % 4}",
                "    checks:",
            ]
        )
        for check_index in range(checks_per_target):
            lines.extend(
                [
                    f"      - id: check-{check_index:03d}",
                    "        profile: history-http",
                ]
            )
    path.write_text("\n".join(lines) + "\n")


def request_json(base_url: str, path: str) -> dict[str, object]:
    with urllib.request.urlopen(base_url + path, timeout=5) as response:
        return json.load(response)


def exercise_server(
    database: Path, *, binary: Path, port: int = 18120
) -> dict[str, object]:
    """Start Kemuri on a copy and test startup, pagination, series, and backup."""
    metadata = sqlite3.connect(database).execute(
        "SELECT months, anchor, targets, checks_per_target FROM history_scale_metadata"
    ).fetchone()
    months, anchor_text, targets, checks_per_target = metadata
    with tempfile.TemporaryDirectory(prefix="kemuri-history-server-") as temporary:
        temporary_path = Path(temporary)
        work_db = temporary_path / "history.db"
        shutil.copy2(database, work_db)
        config = temporary_path / "kemuri.yaml"
        write_config(
            config,
            work_db,
            targets=targets,
            checks_per_target=checks_per_target,
            port=port,
        )
        subprocess.run(
            ["yq", "eval", ".", str(config)],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        process = subprocess.Popen(
            [str(binary), "serve", "--config", str(config)],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        base_url = f"http://127.0.0.1:{port}"
        try:
            for _ in range(100):
                try:
                    urllib.request.urlopen(base_url + "/healthz", timeout=0.2).close()
                    break
                except (OSError, urllib.error.URLError):
                    if process.poll() is not None:
                        raise RuntimeError(process.stdout.read())
                    time.sleep(0.05)
            else:
                raise RuntimeError("server did not become ready")

            seen_targets: list[str] = []
            cursor = None
            while True:
                query = "?limit=5" + (
                    "&cursor=" + urllib.parse.quote(cursor) if cursor else ""
                )
                page = request_json(base_url, "/api/v1/targets" + query)
                seen_targets.extend(item["target_id"] for item in page["targets"])
                cursor = page["next_cursor"]
                if not cursor:
                    break
            if len(seen_targets) != targets or seen_targets != sorted(set(seen_targets)):
                raise RuntimeError("target pagination lost or duplicated records")

            round_page = request_json(
                base_url,
                "/api/v1/targets/history-0000/checks/check-000/rounds?limit=7",
            )
            next_cursor = round_page["next_cursor"]
            second_round_page = request_json(
                base_url,
                "/api/v1/targets/history-0000/checks/check-000/rounds"
                f"?limit=7&cursor={urllib.parse.quote(next_cursor)}",
            )
            first_timestamps = {item["timestamp_ms"] for item in round_page["rounds"]}
            second_timestamps = {
                item["timestamp_ms"] for item in second_round_page["rounds"]
            }
            if first_timestamps & second_timestamps:
                raise RuntimeError("round pagination returned duplicates")

            anchor = dt.datetime.fromisoformat(anchor_text.replace("Z", "+00:00"))
            full_start = anchor - dt.timedelta(days=months * 30)
            full_query = (
                f"?from_ms={int(full_start.timestamp() * 1000)}"
                f"&to_ms={int(anchor.timestamp() * 1000)}&max_points=10"
            )
            full_series = request_json(
                base_url,
                "/api/v1/targets/history-0000/checks/check-000/series" + full_query,
            )
            recent_start = anchor - dt.timedelta(hours=12)
            raw_query = (
                f"?from_ms={int(recent_start.timestamp() * 1000)}"
                f"&to_ms={int(anchor.timestamp() * 1000)}&max_points=100"
            )
            raw_series = request_json(
                base_url,
                "/api/v1/targets/history-0000/checks/check-000/series" + raw_query,
            )
            if full_series["source"] != "rollup" or raw_series["source"] != "raw":
                raise RuntimeError("series did not select expected raw and rollup sources")

            backup_path = temporary_path / "cli-backup.db"
            subprocess.run(
                [
                    str(binary),
                    "database",
                    "backup",
                    "--config",
                    str(config),
                    "--output",
                    str(backup_path),
                ],
                check=True,
                capture_output=True,
                text=True,
            )
            backup_integrity = sqlite3.connect(backup_path).execute(
                "PRAGMA integrity_check"
            ).fetchone()[0]
            return {
                "targets_paginated": len(seen_targets),
                "round_pages": 2,
                "raw_points": len(raw_series["points"]),
                "rollup_points": len(full_series["points"]),
                "backup_integrity": backup_integrity,
            }
        finally:
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
                process.wait(timeout=5)


def parse_anchor(value: str) -> dt.datetime:
    parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise argparse.ArgumentTypeError("anchor must include a timezone")
    return parsed.astimezone(dt.timezone.utc)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--months", type=int, choices=VALID_MONTHS, default=1)
    parser.add_argument("--anchor", type=parse_anchor, default=DEFAULT_ANCHOR)
    parser.add_argument("--cadence-minutes", type=int, default=360)
    parser.add_argument("--targets", type=int, default=12)
    parser.add_argument("--checks-per-target", type=int, default=2)
    parser.add_argument(
        "--max-rounds",
        type=int,
        default=250_000,
        help="safety limit; raise it explicitly for an opt-in scale run",
    )
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--exercise", action="store_true")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/kemuri")
    parser.add_argument("--port", type=int, default=18120)
    args = parser.parse_args()
    report: dict[str, object] = {
        "generated": generate_database(
            args.output,
            months=args.months,
            anchor=args.anchor,
            cadence_minutes=args.cadence_minutes,
            targets=args.targets,
            checks_per_target=args.checks_per_target,
            max_rounds=args.max_rounds,
        )
    }
    if args.verify or args.exercise:
        report["verified"] = verify_database(args.output)
    if args.exercise:
        report["exercised"] = exercise_server(
            args.output, binary=args.binary.resolve(), port=args.port
        )
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
