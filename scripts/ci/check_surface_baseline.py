#!/usr/bin/env python3
"""Authenticate committed published surfaces against PR history or a release."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

from surface_lifecycle_discovery import DISCOVERED_SURFACE_KINDS
from surface_lifecycle_release import (
    DISTRIBUTION_ASSETS,
    REPOSITORY,
    verified_release_assets,
    verified_release_baseline,
)


ROOT = Path(__file__).resolve().parents[2]
MANIFEST = Path("docs/specs/GH969/surface-manifest.json")


def _run(arguments: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(arguments, cwd=ROOT, text=True, capture_output=True, check=False)


def _manifest_at(ref: str) -> dict[str, object] | None:
    result = _run(["git", "show", f"{ref}:{MANIFEST.as_posix()}"])
    if result.returncode != 0:
        return None
    value = json.loads(result.stdout)
    if not isinstance(value, dict):
        raise RuntimeError(f"surface manifest at {ref} is not an object")
    return value


def _tag_commit(release: str) -> str:
    result = _run([
        "git", "ls-remote", f"https://github.com/{REPOSITORY}.git",
        f"refs/tags/{release}", f"refs/tags/{release}^{{}}",
    ])
    if result.returncode != 0:
        raise RuntimeError(f"cannot resolve remote release tag {release}: {result.stderr.strip()}")
    refs = dict(line.split("\t", 1)[::-1] for line in result.stdout.splitlines() if "\t" in line)
    return refs.get(f"refs/tags/{release}^{{}}") or refs.get(f"refs/tags/{release}") or ""


def _recorded_surfaces(manifest: dict[str, object]) -> dict[str, list[str]]:
    records = manifest.get("records")
    if not isinstance(records, list):
        raise RuntimeError("surface manifest records are missing")
    surfaces = {kind: [] for kind in DISCOVERED_SURFACE_KINDS}
    for record in records:
        if not isinstance(record, dict) or record.get("surface_kind") not in surfaces:
            continue
        points = record.get("public_entry_points")
        if not isinstance(points, list) or len(points) != 1 or not isinstance(points[0], str):
            raise RuntimeError("discovered surface record must contain exactly one string entry")
        surfaces[str(record["surface_kind"])].append(points[0])
    return {kind: sorted(entries) for kind, entries in surfaces.items()}


def _release_version(release: object) -> tuple[int, int, int]:
    match = re.fullmatch(r"v([0-9]+)\.([0-9]+)\.([0-9]+)", str(release))
    if not match:
        raise RuntimeError(f"invalid published release {release!r}")
    return tuple(int(part) for part in match.groups())


def verify(base: str) -> None:
    current = json.loads((ROOT / MANIFEST).read_text(encoding="utf-8"))
    if not isinstance(current, dict):
        raise RuntimeError("current surface manifest is not an object")
    release = current.get("published_release")
    published = current.get("published_surfaces")
    if not isinstance(release, str) or not isinstance(published, dict):
        raise RuntimeError("current surface manifest lacks published release metadata")
    previous = _manifest_at(base)
    if previous is not None:
        if previous.get("published_release") == release and previous.get("published_surfaces") == published:
            return
        if _release_version(release) <= _release_version(previous.get("published_release")):
            raise RuntimeError("published baseline changes must advance the release version monotonically")
        prior_surfaces = previous.get("published_surfaces")
        if not isinstance(prior_surfaces, dict):
            raise RuntimeError("base surface manifest lacks published_surfaces")
        for kind in DISCOVERED_SURFACE_KINDS:
            before = prior_surfaces.get(kind)
            after = published.get(kind)
            if not isinstance(before, list) or not isinstance(after, list) or not set(before) <= set(after):
                raise RuntimeError(f"published baseline promotion removed prior {kind} entries")
        released = verified_release_baseline(release, DISCOVERED_SURFACE_KINDS)
        expected = {kind: sorted(entries) for kind, entries in released.items()}
        if published != expected:
            raise RuntimeError("changed published_surfaces do not equal the verified release manifest")
        return

    verified_release_assets(release, DISTRIBUTION_ASSETS)
    base_commit = _run(["git", "rev-parse", f"{base}^{{commit}}"])
    if base_commit.returncode != 0 or base_commit.stdout.strip() != _tag_commit(release):
        raise RuntimeError("initial published baseline base is not the verified release tag commit")
    product_diff = _run(["git", "diff", "--quiet", base, "HEAD", "--", "Cargo.toml", "src"])
    if product_diff.returncode != 0:
        raise RuntimeError("initial published baseline cannot bootstrap across product-source changes")
    if published != _recorded_surfaces(current):
        raise RuntimeError("initial published baseline must equal every discovered release-source record")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("base", help="PR base ref or prior push SHA")
    args = parser.parse_args()
    try:
        verify(args.base)
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print("published surface baseline check: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
