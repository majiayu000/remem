#!/usr/bin/env python3
"""Guard release workflow safety and auto-release tag-state behavior."""

from __future__ import annotations

import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError

import check_release_registry
import surface_lifecycle_release


ROOT = Path(__file__).resolve().parents[2]
AUTO_RELEASE = ROOT / ".github/workflows/auto-release.yml"
RELEASE = ROOT / ".github/workflows/release.yml"
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

    release_text = RELEASE.read_text(encoding="utf-8")
    for needle in [
        "fetch-depth: 2",
        "python3 scripts/ci/check_public_surface.py",
        'python3 scripts/ci/check_surface_baseline.py "${GITHUB_SHA}^"',
        "GH_TOKEN: ${{ github.token }}",
        "cp docs/specs/GH969/surface-manifest.json artifacts/surface-manifest.json",
        "artifacts/surface-manifest.json",
    ]:
        if needle not in release_text:
            die(f"release workflow is missing lifecycle asset step {needle!r}")



def check_distribution_channels() -> None:
    text = RELEASE.read_text(encoding="utf-8")
    for job, dependency in (
        ("publish-npm", "release"),
        ("publish-dsh", "release"),
        ("publish-mcp-registry", "publish-npm"),
    ):
        if f"\n  {job}:\n    needs: {dependency}\n" not in text:
            die(f"{job} must depend on {dependency} only")

    wrapper = {"name": "@remem-ai/remem", "version": "1.2.3"}
    with patch.object(check_release_registry, "urlopen", return_value=io.StringIO(json.dumps(wrapper))) as fetch:
        if not check_release_registry.published("npm", wrapper):
            die("an existing npm version must skip publication on retry")
        if not fetch.call_args.args[0].full_url.endswith("/%40remem-ai%2Fremem/1.2.3"):
            die("npm must be queried for the exact scoped package version")

    for status in (404, 503):
        error = HTTPError("https://registry.npmjs.org", status, "fixture", None, None)
        with patch.object(check_release_registry, "urlopen", side_effect=error):
            try:
                exists = check_release_registry.published("npm", wrapper)
            except HTTPError:
                if status == 404:
                    die("a missing package version must allow publication")
            else:
                if exists or status != 404:
                    die("only a missing package version may allow publication")

    manifest = {
        "name": "io.github.majiayu000/remem",
        "version": "1.2.3",
        "packages": [{"registryType": "npm", "identifier": wrapper["name"], "version": wrapper["version"]}],
    }
    with patch.object(check_release_registry, "urlopen", return_value=io.StringIO(json.dumps({"server": manifest}))):
        if not check_release_registry.published("mcp", manifest):
            die("an existing MCP Registry version must skip publication on retry")

    wrong_version = {"server": {**manifest, "packages": [{**manifest["packages"][0], "version": "1.2.2"}]}}
    with patch.object(check_release_registry, "urlopen", return_value=io.StringIO(json.dumps(wrong_version))):
        try:
            check_release_registry.published("mcp", manifest)
        except ValueError:
            pass
        else:
            die("a different npm version cannot satisfy the MCP Registry check")


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


def check_surface_release_verifier() -> None:
    kinds = {"rust_export", "mcp_tool"}
    manifest = {
        "schema_version": 2,
        "records": [
            {"surface_kind": kind, "public_entry_points": [f"fixture-{kind}"]}
            for kind in kinds
        ],
    }
    assets = [{"name": name} for name in surface_lifecycle_release.REQUIRED_ASSETS]
    metadata = {"tagName": "v1.2.3", "isDraft": False, "assets": assets}
    original = surface_lifecycle_release._gh

    def fake_gh(arguments: list[str]) -> str:
        if arguments[1] == "view":
            return json.dumps(metadata)
        directory = Path(arguments[arguments.index("--dir") + 1])
        (directory / "surface-manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        return ""

    surface_lifecycle_release._gh = fake_gh
    try:
        baseline = surface_lifecycle_release.verified_release_baseline("v1.2.3", kinds)
    finally:
        surface_lifecycle_release._gh = original
    expected = {kind: {f"fixture-{kind}"} for kind in kinds}
    if baseline != expected:
        die(f"release surface baseline was wrong: {baseline!r}")
    metadata["isDraft"] = True
    surface_lifecycle_release._gh = fake_gh
    try:
        try:
            surface_lifecycle_release.verified_release_baseline("v1.2.3", kinds)
            die("draft release advanced the published surface baseline")
        except RuntimeError as exc:
            if "non-draft" not in str(exc):
                raise
    finally:
        surface_lifecycle_release._gh = original


def main() -> int:
    if shutil.which("git") is None:
        die("git is required")
    check_workflow_text()
    check_distribution_channels()
    check_tag_state_script()
    check_surface_release_verifier()
    print("release workflow check: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
