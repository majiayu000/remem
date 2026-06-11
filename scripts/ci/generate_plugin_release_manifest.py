#!/usr/bin/env python3
"""Generate the Codex plugin runtime release manifest from SHA256SUMS."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


EXPECTED_ASSETS = {
    "remem-darwin-arm64.tar.gz": "darwin-arm64",
    "remem-darwin-x64.tar.gz": "darwin-x64",
    "remem-linux-arm64.tar.gz": "linux-arm64",
    "remem-linux-x64.tar.gz": "linux-x64",
}
SHA256_RE = re.compile(r"^([0-9a-fA-F]{64})\s+\*?(.+)$")


def parse_checksums(path: Path) -> dict[str, dict[str, str]]:
    assets: dict[str, dict[str, str]] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        match = SHA256_RE.match(line.strip())
        if not match:
            raise ValueError(f"invalid SHA256SUMS line: {line!r}")
        checksum, artifact = match.groups()
        filename = Path(artifact).name
        if filename not in EXPECTED_ASSETS:
            raise ValueError(f"unexpected runtime artifact in SHA256SUMS: {filename}")
        platform = EXPECTED_ASSETS[filename]
        assets[platform] = {
            "file": filename,
            "sha256": checksum.lower(),
        }
    missing = sorted(set(EXPECTED_ASSETS.values()) - set(assets.keys()))
    if missing:
        raise ValueError(f"missing runtime artifact checksums for: {', '.join(missing)}")
    return assets


def write_manifest(version: str, artifacts_dir: Path, output: Path, base_url: str | None) -> None:
    if not version or version.startswith("v"):
        raise ValueError("version must be the bare package version, for example 0.5.28")
    checksums = artifacts_dir / "SHA256SUMS"
    if not checksums.exists():
        raise ValueError(f"missing checksum file: {checksums}")
    manifest = {
        "versions": {
            version: {
                "base_url": base_url
                or f"https://github.com/majiayu000/remem/releases/download/v{version}",
                "assets": parse_checksums(checksums),
            }
        }
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(f"{json.dumps(manifest, indent=2, sort_keys=True)}\n", encoding="utf-8")


def main(argv: list[str]) -> int:
    if len(argv) not in (3, 4):
        print(
            "usage: generate_plugin_release_manifest.py VERSION ARTIFACTS_DIR OUTPUT [BASE_URL]",
            file=sys.stderr,
        )
        return 2
    version = argv[0]
    artifacts_dir = Path(argv[1])
    output = Path(argv[2])
    base_url = argv[3] if len(argv) == 4 else None
    write_manifest(version, artifacts_dir, output, base_url)
    print(f"wrote plugin release manifest: {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
