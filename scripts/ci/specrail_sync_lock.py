#!/usr/bin/env python3
"""Read and verify the vendored SpecRail sync lock."""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path


def split_classified(paths: list[str]) -> tuple[list[str], list[str]]:
    separator = paths.index("--")
    return paths[:separator], paths[separator + 1 :]


def write_sync_lock(
    lock_path: Path, upstream_sha: str, classified: list[str]
) -> None:
    files, excluded = split_classified(classified)
    repo_root = lock_path.resolve().parent.parent
    entries = []
    for relative_path in files:
        path = repo_root / relative_path
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        entries.append({"path": relative_path, "sha256": digest})
    lock = {
        "upstream_repo": "https://github.com/majiayu000/specrail",
        "upstream_sha": upstream_sha,
        "excluded": excluded,
        "files": entries,
    }
    lock_path.write_text(json.dumps(lock, indent=2) + "\n", encoding="utf-8")
    print(f"lock written: {lock_path} @ upstream {upstream_sha}")


def verify_sync_lock(lock_path: Path, classified: list[str]) -> None:
    expected_files, expected_excluded = split_classified(classified)
    repo_root = lock_path.resolve().parent.parent
    with lock_path.open(encoding="utf-8") as fh:
        lock = json.load(fh)
    failed = False
    entries = lock.get("files")
    if not isinstance(entries, list):
        print("INVALID: lock files must be a list")
        raise SystemExit(1)
    lock_files = [entry.get("path") for entry in entries if isinstance(entry, dict)]
    if len(lock_files) != len(entries) or lock_files != expected_files:
        print("INVALID: sync managed file list does not match lock")
        print(f"script: {expected_files}")
        print(f"lock:   {lock_files}")
        failed = True
    excluded = lock.get("excluded")
    if excluded != expected_excluded:
        print("INVALID: local-owned excluded file list does not match lock")
        print(f"script: {expected_excluded}")
        print(f"lock:   {excluded}")
        failed = True
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        path = repo_root / entry["path"]
        if not path.exists():
            print(f"MISSING: {entry['path']}")
            failed = True
            continue
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        if digest != entry["sha256"]:
            print(f"DRIFT: {entry['path']}")
            failed = True
    if failed:
        print(
            f"vendored SpecRail files drifted from lock (upstream {lock['upstream_sha']}); "
            "re-run scripts/sync-specrail-checks.sh <specrail-repo> or restore the files"
        )
        raise SystemExit(1)
    print(
        f"ok: {len(lock['files'])} upstream-managed files match lock "
        f"(upstream {lock['upstream_sha']})"
    )
    print(
        f"ok: {len(expected_excluded)} local-owned files explicitly excluded "
        "from upstream sync"
    )


def main(argv: list[str]) -> None:
    if len(argv) < 3:
        raise SystemExit(
            "usage: specrail_sync_lock.py write|verify LOCK_PATH "
            "[UPSTREAM_SHA] FILES -- EXCLUDED"
        )
    command = argv[1]
    lock_path = Path(argv[2])
    if command == "write":
        if len(argv) < 5:
            raise SystemExit("write requires an upstream SHA and classified paths")
        write_sync_lock(lock_path, argv[3], argv[4:])
        return
    if command == "verify":
        verify_sync_lock(lock_path, argv[3:])
        return
    raise SystemExit(f"unknown command: {command}")


if __name__ == "__main__":
    main(sys.argv)
