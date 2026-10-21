#!/usr/bin/env python3
"""Exercise the four GH935 smoke fixtures offline, without launching hosts.

Every score runs in a fresh temporary repository. Reference/wrong patches and
hidden files are scorer-private fixture data, never part of the target recipe.
This checks fixture discrimination; it produces no benchmark result or claim.
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

import run_dry


class SmokeFixtureTests(unittest.TestCase):
    def test_four_registered_smoke_scores_discriminate(self) -> None:
        fixture_path = run_dry.SUITE_ROOT / "fixtures" / "smoke.json"
        fixture = run_dry.load_json(fixture_path)
        schema = run_dry.load_json(run_dry.TASK_SCHEMA)
        tasks = []
        for directory in run_dry.TASK_DIRS.values():
            for path in sorted(directory.glob("*.json")):
                task = run_dry.load_json(path)
                if task.get("fixture") == "fixtures/smoke.json":
                    tasks.append(task)
        self.assertEqual(len(tasks), 4)
        self.assertEqual(
            {(task["direction"], task["category"]) for task in tasks},
            {
                (direction, category)
                for direction in run_dry.TASK_DIRS
                for category in ("architecture_decision", "failed_attempt_lesson")
            },
        )
        for task in tasks:
            with self.subTest(task=task["id"]):
                self.assertEqual(run_dry.validate(task, schema), [])
                self.assertEqual(task["status"], "skeleton_todo")
                self.assertTrue(task["todo"])
                episodes = task["history_episodes"]
                self.assertGreaterEqual(len(episodes), 2)
                epochs = [episode["reference_time_epoch"] for episode in episodes]
                self.assertEqual(epochs, sorted(set(epochs)))
                events = {
                    event["event_id"]: event
                    for episode in episodes
                    for event in episode["raw_events"]
                }
                for episode in episodes:
                    self.assertEqual(episode["host"], task["source_host"])
                    for memory in episode["memories"]:
                        self.assertTrue(
                            any(memory["text"] in event["sanitized_content"]
                                for event in episode["raw_events"])
                        )
                self.assertTrue(set(task["gold_memory"]["supporting_event_ids"]) <= events.keys())
                self.assertTrue(task["score"]["commands"])
                self.assertTrue(task["score"]["hidden_files"])
                self.assertFalse(set(task["score"]["hidden_files"]) & fixture["repo"]["files"].keys())
                category = task["category"]
                cases = [("base", {}, False)]
                cases.append(("reference", fixture["reference_patches"][category], True))
                cases.extend((name, patch, False) for name, patch in fixture["wrong_patches"][category].items())
                for name, patch, expected_pass in cases:
                    with self.subTest(patch=name):
                        self.check_score(fixture, task, patch, expected_pass)
                print(f"{task['id']}: base rejected, reference passed, "
                      f"{len(cases) - 2} wrong patches rejected")

    def check_score(self, fixture: dict, task: dict, patch: dict, expected_pass: bool) -> None:
        with tempfile.TemporaryDirectory(prefix="gh935-offline-smoke-") as raw:
            repo = Path(raw)
            files = {**fixture["repo"]["files"], **patch, **task["score"]["hidden_files"]}
            for name, content in files.items():
                path = repo / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content, encoding="utf-8")
            env = {key: value for key, value in os.environ.items()
                   if key not in {"PYTHONPATH", "PYTHONHOME"}}
            env["PYTHONDONTWRITEBYTECODE"] = "1"
            for command in task["score"]["commands"]:
                result = subprocess.run(
                    command, cwd=repo, env=env, text=True,
                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                    timeout=10, check=False,
                )
                if expected_pass:
                    self.assertEqual(result.returncode, 0, result.stdout)
                else:
                    self.assertEqual(result.returncode, 1, result.stdout)
                    self.assertIn("FAIL:", result.stdout)
                    self.assertNotIn("ERROR:", result.stdout)


if __name__ == "__main__":
    unittest.main()
