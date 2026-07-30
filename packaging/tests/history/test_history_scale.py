#!/usr/bin/env python3
"""Focused tests for the history scale fixture generator."""

from __future__ import annotations

import hashlib
import importlib.util
from pathlib import Path
import sqlite3
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "history_scale.py"
SPEC = importlib.util.spec_from_file_location("history_scale", SCRIPT)
assert SPEC and SPEC.loader
history_scale = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(history_scale)


class HistoryScaleTests(unittest.TestCase):
    def test_profiles_have_expected_counts_and_classes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            for months in history_scale.VALID_MONTHS:
                database = Path(temporary) / f"history-{months}.db"
                expected = history_scale.generate_database(
                    database,
                    months=months,
                    cadence_minutes=720,
                    targets=2,
                    checks_per_target=1,
                )
                report = history_scale.verify_database(database)
                self.assertEqual(report["integrity"], "ok")
                self.assertEqual(report["counts"]["rounds"], expected["rounds"])
                connection = sqlite3.connect(database)
                classes = set(
                    connection.execute(
                        """SELECT execution_status, resolution_seconds
                        FROM rounds CROSS JOIN
                        (SELECT DISTINCT resolution_seconds FROM rollups)"""
                    )
                )
                connection.close()
                self.assertEqual(
                    classes,
                    {
                        ("completed", 300),
                        ("completed", 3600),
                        ("partial", 300),
                        ("partial", 3600),
                    },
                )

    def test_generation_is_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            paths = [Path(temporary) / f"copy-{index}.db" for index in range(2)]
            for path in paths:
                history_scale.generate_database(
                    path,
                    months=1,
                    cadence_minutes=1440,
                    targets=1,
                    checks_per_target=1,
                )
            hashes = [hashlib.sha256(path.read_bytes()).hexdigest() for path in paths]
            self.assertEqual(hashes[0], hashes[1])

    def test_five_minute_cadence_aggregates_hour_rollups(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            database = Path(temporary) / "five-minute.db"
            expected = history_scale.generate_database(
                database,
                months=1,
                cadence_minutes=5,
                targets=1,
                checks_per_target=1,
            )
            report = history_scale.verify_database(database)
            self.assertEqual(report["integrity"], "ok")
            self.assertLess(expected["rollups"], expected["rounds"] * 2)
            connection = sqlite3.connect(database)
            maximum = connection.execute(
                """SELECT MAX(scheduled_rounds) FROM rollups
                WHERE resolution_seconds = 3600"""
            ).fetchone()[0]
            connection.close()
            self.assertEqual(maximum, 12)

    def test_invalid_scale_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(ValueError):
                history_scale.generate_database(
                    Path(temporary) / "bad.db",
                    months=2,
                )


if __name__ == "__main__":
    unittest.main()
