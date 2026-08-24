#!/usr/bin/env python3
"""Load a published-surface baseline from a verified GitHub release asset."""

from __future__ import annotations

import json
import re
import subprocess
import tempfile
from pathlib import Path


REPOSITORY = "majiayu000/remem"
DISTRIBUTION_ASSETS = {
    "SHA256SUMS",
    "remem-releases.json",
    "remem-darwin-arm64.tar.gz",
    "remem-darwin-x64.tar.gz",
    "remem-linux-arm64.tar.gz",
    "remem-linux-x64.tar.gz",
}
REQUIRED_ASSETS = {*DISTRIBUTION_ASSETS, "surface-manifest.json"}
OPTIONALLY_EMPTY_KINDS = {"rust_target_export"}


def _gh(arguments: list[str]) -> str:
    result = subprocess.run(
        ["gh", *arguments], text=True, capture_output=True, check=False,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise RuntimeError(f"gh {' '.join(arguments)} failed: {detail}")
    return result.stdout


def _released_entries(path: Path, kinds: set[str]) -> dict[str, set[str]]:
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"released surface manifest is unreadable: {exc}") from exc
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 2:
        raise RuntimeError("released surface manifest must use schema_version 2")
    records = manifest.get("records")
    if not isinstance(records, list) or not records:
        raise RuntimeError("released surface manifest has no records")
    baseline = {kind: set() for kind in kinds}
    for record in records:
        if not isinstance(record, dict) or record.get("surface_kind") not in kinds:
            continue
        kind = str(record["surface_kind"])
        points = record.get("public_entry_points")
        if not isinstance(points, list) or len(points) != 1 or not isinstance(points[0], str) or not points[0].strip():
            raise RuntimeError(f"released {kind} record must have exactly one non-empty entry")
        entry = points[0]
        if entry in baseline[kind]:
            raise RuntimeError(f"released surface manifest duplicates {kind}:{entry}")
        baseline[kind].add(entry)
    missing = sorted(kind for kind, entries in baseline.items() if not entries and kind not in OPTIONALLY_EMPTY_KINDS)
    if missing:
        raise RuntimeError(f"released surface manifest lacks discovered kinds: {', '.join(missing)}")
    return baseline


def verified_release_assets(release: str, required_assets: set[str]) -> set[str]:
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", release):
        raise RuntimeError(f"published release must be an exact vX.Y.Z tag, got {release!r}")
    raw = _gh([
        "release", "view", release, "--repo", REPOSITORY,
        "--json", "tagName,isDraft,assets",
    ])
    try:
        metadata = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"GitHub release metadata is invalid JSON: {exc}") from exc
    if metadata.get("tagName") != release or metadata.get("isDraft") is not False:
        raise RuntimeError(f"{release} must exist as an exact, non-draft GitHub release")
    assets = metadata.get("assets")
    names = {
        asset.get("name") for asset in assets if isinstance(asset, dict) and isinstance(asset.get("name"), str)
    } if isinstance(assets, list) else set()
    missing = sorted(required_assets - names)
    if missing:
        raise RuntimeError(f"GitHub release {release} lacks required assets: {', '.join(missing)}")
    return names


def verified_release_baseline(release: str, kinds: set[str]) -> dict[str, set[str]]:
    verified_release_assets(release, REQUIRED_ASSETS)
    with tempfile.TemporaryDirectory(prefix="remem-release-surface-") as raw_dir:
        directory = Path(raw_dir)
        _gh([
            "release", "download", release, "--repo", REPOSITORY,
            "--pattern", "surface-manifest.json", "--dir", str(directory),
        ])
        return _released_entries(directory / "surface-manifest.json", kinds)
