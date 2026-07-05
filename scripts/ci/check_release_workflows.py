#!/usr/bin/env python3
"""Guard release workflow safety and auto-release tag-state behavior."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
AUTO_RELEASE = ROOT / ".github/workflows/auto-release.yml"
TAG_STATE_SCRIPT = ROOT / "scripts/ci/auto_release_check_tag_state.sh"


def die(message: str) -> None:
    print(f"FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def run(cmd: list[str], cwd: Path, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)
    return subprocess.run(
        cmd,
        cwd=cwd,
        env=merged_env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def require_ok(cmd: list[str], cwd: Path, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    result = run(cmd, cwd, env)
    if result.returncode != 0:
        die(
            f"{' '.join(cmd)} failed in {cwd}\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result


def check_workflow_text() -> None:
    text = AUTO_RELEASE.read_text(encoding="utf-8")
    required = [
        "github.event.workflow_run.event == 'push'",
        "CI_EVENT: ${{ github.event.workflow_run.event }}",
        "CI_HEAD_BRANCH: ${{ github.event.workflow_run.head_branch }}",
        "CI_HEAD_SHA: ${{ github.event.workflow_run.head_sha }}",
        'if [ "$CI_EVENT" != "push" ]; then',
        'if [ "$CI_HEAD_BRANCH" != "main" ]; then',
        'if [ "$CI_HEAD_SHA" != "$(git rev-parse HEAD)" ]; then',
        "run: bash scripts/ci/auto_release_check_tag_state.sh",
        "TAG_EXISTS: ${{ steps.tag.outputs.exists }}",
        "TAG_SHA: ${{ steps.tag.outputs.tag_sha }}",
        'if [ "$TAG_EXISTS" = "true" ] && [ "$TAG_SHA" != "$(git rev-parse HEAD)" ]; then',
        "refusing to dispatch release workflow for an unverified tag",
    ]
    for needle in required:
        if needle not in text:
            die(f"auto-release workflow is missing {needle!r}")

    forbidden = [
        '[ "${{ github.event.workflow_run.head_branch }}"',
        '[ "${{ github.event.workflow_run.head_sha }}"',
    ]
    for needle in forbidden:
        if needle in text:
            die(f"auto-release workflow embeds unsafe shell context {needle!r}")


def git_init(path: Path) -> None:
    require_ok(["git", "init", "-b", "main"], path)
    require_ok(["git", "config", "user.email", "ci@example.invalid"], path)
    require_ok(["git", "config", "user.name", "CI Test"], path)


def commit_file(repo: Path, name: str, text: str) -> None:
    (repo / name).write_text(text, encoding="utf-8")
    require_ok(["git", "add", name], repo)
    require_ok(["git", "commit", "-m", f"commit {name}"], repo)


def make_repo_with_origin(tmp: Path) -> Path:
    remote = tmp / "remote.git"
    repo = tmp / "repo"
    require_ok(["git", "init", "--bare", str(remote)], tmp)
    repo.mkdir()
    git_init(repo)
    require_ok(["git", "remote", "add", "origin", str(remote)], repo)
    return repo


def run_tag_state(repo: Path, tag: str, version: str) -> subprocess.CompletedProcess[str]:
    output = repo / "github-output.txt"
    output.write_text("", encoding="utf-8")
    return run(
        ["bash", str(TAG_STATE_SCRIPT)],
        repo,
        {
            "TAG": tag,
            "VERSION": version,
            "GITHUB_OUTPUT": str(output),
        },
    )


def check_tag_state_script() -> None:
    if not TAG_STATE_SCRIPT.exists():
        die("missing scripts/ci/auto_release_check_tag_state.sh")

    with tempfile.TemporaryDirectory() as raw_tmp:
        tmp = Path(raw_tmp)
        repo = make_repo_with_origin(tmp)
        commit_file(repo, "a.txt", "first\n")
        require_ok(["git", "tag", "-a", "v1.2.3", "-m", "Release v1.2.3"], repo)
        require_ok(["git", "push", "origin", "main", "--tags"], repo)
        first_sha = require_ok(["git", "rev-list", "-n", "1", "v1.2.3"], repo).stdout.strip()

        commit_file(repo, "b.txt", "second\n")
        result = run_tag_state(repo, "v1.2.3", "1.2.3")
        if result.returncode != 0:
            die(f"existing released tag should be a no-op\nstderr:\n{result.stderr}")
        output = (repo / "github-output.txt").read_text(encoding="utf-8")
        if "exists=true" not in output or f"tag_sha={first_sha}" not in output:
            die(f"existing tag output was wrong:\n{output}")

    with tempfile.TemporaryDirectory() as raw_tmp:
        tmp = Path(raw_tmp)
        repo = make_repo_with_origin(tmp)
        commit_file(repo, "a.txt", "first\n")
        require_ok(["git", "tag", "-a", "v1.2.2", "-m", "Release v1.2.2"], repo)
        require_ok(["git", "push", "origin", "main", "--tags"], repo)
        result = run_tag_state(repo, "v1.2.3", "1.2.3")
        if result.returncode != 0:
            die(f"newer staged version should be taggable\nstderr:\n{result.stderr}")
        output = (repo / "github-output.txt").read_text(encoding="utf-8")
        if "exists=false" not in output:
            die(f"new tag output was wrong:\n{output}")

    with tempfile.TemporaryDirectory() as raw_tmp:
        tmp = Path(raw_tmp)
        repo = make_repo_with_origin(tmp)
        commit_file(repo, "a.txt", "first\n")
        require_ok(["git", "tag", "-a", "v1.2.3", "-m", "Release v1.2.3"], repo)
        require_ok(["git", "push", "origin", "main", "--tags"], repo)
        result = run_tag_state(repo, "v1.2.2", "1.2.2")
        if result.returncode == 0:
            die("older staged version without its tag should fail")


def main() -> int:
    if shutil.which("git") is None:
        die("git is required")
    check_workflow_text()
    check_tag_state_script()
    print("release workflow check: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
