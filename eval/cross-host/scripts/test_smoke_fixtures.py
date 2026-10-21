#!/usr/bin/env python3
"""Check all 24 GH935 task fixtures offline, without launching hosts.

These are 12 logical scenarios repeated in two directions, not 24 independent
samples. Every score runs in a fresh temporary repository; repairs and hidden
files stay scorer-private. This produces no live result or comparative claim.
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

import run_dry


def write_files(repo: Path, files: dict) -> None:
    for name, content in files.items():
        path = repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


def prepare_case(repo: Path, category: str, env: dict) -> None:
    """Materialize only the fixed Git/directory evidence needed by these cases."""
    if category == "same_name_repo_isolation":
        for tenant in ("tenant-a", "tenant-b"):
            project = repo / tenant / "atlas"
            project.mkdir(parents=True)
            (project / "project-marker.txt").write_text(tenant + "\n")
    if category not in {"branch_specific_truth", "git_evidence"}:
        return

    def git(*args: str) -> None:
        subprocess.run(
            ["git", "-c", f"core.hooksPath={os.devnull}",
             "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", *args],
            cwd=repo, env=env, check=True, timeout=10,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )

    def commit_file(name: str, content: str, message: str) -> None:
        (repo / name).write_text(content, encoding="utf-8")
        git("add", name)
        git("commit", "--quiet", "-m", message)

    git("init", "--quiet", "--template=", "-b", "main")
    git("add", ".")
    git("commit", "--quiet", "-m", "initial fixture")
    if category == "branch_specific_truth":
        git("checkout", "--quiet", "-b", "stable")
        commit_file("branch-marker.txt", "stable\n", "stable branch evidence")
        git("checkout", "--quiet", "-b", "next", "main")
        commit_file("branch-marker.txt", "next\n", "next branch evidence")
        git("checkout", "--quiet", "stable")
    else:
        commit_file("retry-budget.txt", "5\n", "reviewed retry budget")
        git("tag", "reviewed-budget")
        commit_file("retry-budget.txt", "2\n", "newer unapproved experiment")


class SmokeFixtureTests(unittest.TestCase):
    def test_all_registered_offline_scores_discriminate(self) -> None:
        fixture = run_dry.load_json(run_dry.SUITE_ROOT / "fixtures" / "smoke.json")
        schema = run_dry.load_json(run_dry.TASK_SCHEMA)
        charter = run_dry.load_json(run_dry.CHARTER)
        categories = set(charter["task_requirements"]["required_categories"])
        tasks = []
        for directory in run_dry.TASK_DIRS.values():
            for path in sorted(directory.glob("*.json")):
                task = run_dry.load_json(path)
                if task.get("fixture") == "fixtures/smoke.json":
                    tasks.append(task)
        self.assertEqual(len(tasks), 24)
        self.assertEqual(
            {(task["direction"], task["category"]) for task in tasks},
            {(direction, category) for direction in run_dry.TASK_DIRS for category in categories},
        )
        score_processes = 0
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
                self.assertEqual(len(cases), 4)
                for name, patch, expected_pass in cases:
                    with self.subTest(patch=name):
                        score_processes += self.check_score(fixture, task, patch, expected_pass)
                print(f"{task['id']}: base rejected, reference passed, "
                      f"{len(cases) - 2} wrong patches rejected", flush=True)
        self.assertEqual(score_processes, 96)
        print("12 logical scenarios x 2 directions; 96 offline score processes", flush=True)

    def check_score(self, fixture: dict, task: dict, patch: dict, expected_pass: bool) -> int:
        with tempfile.TemporaryDirectory(prefix="gh935-offline-smoke-") as raw:
            repo = Path(raw)
            env = {key: value for key, value in os.environ.items()
                   if key not in {"PYTHONPATH", "PYTHONHOME"} and not key.startswith("GIT_")}
            env.update({
                "PYTHONDONTWRITEBYTECODE": "1",
                "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                "GIT_AUTHOR_NAME": "GH935 fixture", "GIT_COMMITTER_NAME": "GH935 fixture",
                "GIT_AUTHOR_EMAIL": "fixture@example.invalid", "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
                "GIT_AUTHOR_DATE": "2001-01-01T00:00:00Z", "GIT_COMMITTER_DATE": "2001-01-01T00:00:00Z",
            })
            write_files(repo, fixture["repo"]["files"])
            prepare_case(repo, task["category"], env)
            write_files(repo, {**patch, **task["score"]["hidden_files"]})
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
            return len(task["score"]["commands"])


if __name__ == "__main__":
    unittest.main()
