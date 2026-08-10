#!/usr/bin/env python3

from __future__ import annotations

import copy
import importlib.util
import json
import sys
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("memory_baseline.py")
SPEC = importlib.util.spec_from_file_location("memory_baseline", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
baseline = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = baseline
SPEC.loader.exec_module(baseline)


def sample_status() -> dict:
    return {
        "version": "0.6.59 (schema v80)",
        "totals": {
            "memories": 10,
            "observations": 2,
            "sessions": 3,
            "raw_messages": 40,
        },
        "review_queue": {
            "pending": 4,
            "median_age_secs": 50,
            "max_age_secs": 90,
            "inflow_7d": 3,
            "resolved_7d": 1,
            "projects": [{"project": "/secret/repo", "pending": 4}],
        },
        "candidate_promotion": [
            {
                "source_kind": "summary",
                "review_status": "pending_review",
                "block_reason": "source_trust_below_floor",
                "total": 4,
                "last_7_days": 3,
            }
        ],
        "raw_archive": {
            "messages": 40,
            "ingest_failures": 1,
            "parse_errors": 0,
            "insert_errors": 2,
            "latest_failure_path": "/secret/transcript.jsonl",
            "latest_failure_message": "secret contents",
        },
        "capture_pipeline": {
            "captured": 20,
            "dropped": 5,
            "unrecovered_spills": 0,
            "latest_drop_detail": "secret command",
            "extract_todo": 2,
            "extract_running": 0,
            "extract_expired": 0,
            "extract_failed": 1,
            "retryable_replay_ranges": 1,
            "active_replay_ranges": 0,
            "quarantined_replay_ranges": 0,
            "pending_candidates": 4,
            "pending_graph_candidates": 0,
        },
        "database": {"path": "/secret/remem.db"},
        "top_projects": [{"project": "/secret/repo"}],
    }


class MemoryBaselineTests(unittest.TestCase):
    def test_report_is_deterministic_for_same_metrics(self) -> None:
        first = baseline.build_report(sample_status())
        second_input = sample_status()
        second_input["candidate_promotion"].reverse()
        second = baseline.build_report(second_input)
        self.assertEqual(first, second)

    def test_report_excludes_paths_and_content(self) -> None:
        encoded = json.dumps(baseline.build_report(sample_status()))
        self.assertNotIn("/secret", encoded)
        self.assertNotIn("secret contents", encoded)
        self.assertNotIn("secret command", encoded)
        self.assertNotIn("projects", encoded)

    def test_digest_changes_when_allowlisted_metric_changes(self) -> None:
        original = baseline.build_report(sample_status())
        changed_input = copy.deepcopy(sample_status())
        changed_input["review_queue"]["pending"] += 1
        changed = baseline.build_report(changed_input)
        self.assertNotEqual(original["metrics_sha256"], changed["metrics_sha256"])

    def test_missing_required_field_fails_closed(self) -> None:
        value = sample_status()
        del value["capture_pipeline"]["extract_failed"]
        with self.assertRaisesRegex(baseline.BaselineError, "extract_failed"):
            baseline.build_report(value)


if __name__ == "__main__":
    unittest.main()
