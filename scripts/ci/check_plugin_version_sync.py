#!/usr/bin/env python3
"""Require the Codex plugin runtime version metadata to match Cargo.toml."""

from __future__ import annotations

import json
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CARGO_TOML = ROOT / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
PLUGIN_JSON = ROOT / "plugins/remem/.codex-plugin/plugin.json"
RELEASES_JSON = ROOT / "plugins/remem/runtimes/remem-releases.json"


def read_json(path: Path) -> dict:
    with path.open("r", encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected JSON object")
    return value


def read_toml(path: Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def cargo_package_version() -> str:
    version = read_toml(CARGO_TOML).get("package", {}).get("version")
    if not isinstance(version, str) or not version.strip():
        raise ValueError("Cargo.toml is missing package.version")
    return version


def lock_version() -> str | None:
    packages = read_toml(CARGO_LOCK).get("package", [])
    matches = [
        package.get("version")
        for package in packages
        if isinstance(package, dict) and package.get("name") == "remem-ai"
    ]
    if len(matches) != 1:
        raise ValueError(f"Cargo.lock must contain exactly one remem-ai package, found {len(matches)}")
    version = matches[0]
    if not isinstance(version, str) or not version.strip():
        raise ValueError("Cargo.lock remem-ai package is missing version")
    return version


def plugin_version() -> str:
    version = read_json(PLUGIN_JSON).get("version")
    if not isinstance(version, str) or not version.strip():
        raise ValueError("plugins/remem/.codex-plugin/plugin.json is missing version")
    return version


def release_versions() -> tuple[set[str], str | None]:
    manifest = read_json(RELEASES_JSON)
    versions = manifest.get("versions")
    if not isinstance(versions, dict):
        raise ValueError("plugins/remem/runtimes/remem-releases.json is missing versions object")
    keys = {str(key) for key in versions.keys()}
    if len(keys) != 1:
        return keys, None
    key = next(iter(keys))
    release = versions.get(key)
    if not isinstance(release, dict):
        raise ValueError(f"release entry for {key} must be an object")
    base_url = release.get("base_url")
    if base_url is not None and not isinstance(base_url, str):
        raise ValueError(f"release entry for {key} has non-string base_url")
    assets = release.get("assets")
    if assets is not None and not isinstance(assets, dict):
        raise ValueError(f"release entry for {key} has non-object assets")
    return keys, base_url


def main() -> int:
    cargo = cargo_package_version()
    locked = lock_version()
    plugin = plugin_version()
    releases, base_url = release_versions()

    errors: list[str] = []
    if locked != cargo:
        errors.append(f"Cargo.lock remem-ai version is {locked}, expected {cargo}")
    if plugin != cargo:
        errors.append(f"plugin.json version is {plugin}, expected {cargo}")
    if releases != {cargo}:
        rendered = ", ".join(sorted(releases)) or "<none>"
        errors.append(f"remem-releases.json versions are {{{rendered}}}, expected only {{{cargo}}}")
    expected_suffix = f"/releases/download/v{cargo}"
    if base_url is None:
        errors.append(f"remem-releases.json entry for {cargo} is missing base_url")
    elif not base_url.endswith(expected_suffix):
        errors.append(f"release base_url is {base_url}, expected suffix {expected_suffix}")

    if errors:
        print("plugin version sync check failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        print(
            "Update Cargo.toml, Cargo.lock, plugins/remem/.codex-plugin/plugin.json, "
            "and plugins/remem/runtimes/remem-releases.json together.",
            file=sys.stderr,
        )
        return 1

    print(
        "plugin version sync: "
        f"{cargo} across Cargo.toml, Cargo.lock, plugin.json, and remem-releases.json"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
