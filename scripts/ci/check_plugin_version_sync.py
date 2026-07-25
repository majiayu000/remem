#!/usr/bin/env python3
"""Require remem package, plugin, and runtime metadata to stay coherent."""

from __future__ import annotations

import json
import re
import argparse
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CARGO_TOML = ROOT / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
CHANGELOG = ROOT / "CHANGELOG.md"
PLUGIN_JSON = ROOT / "plugins/remem/.codex-plugin/plugin.json"
RELEASES_JSON = ROOT / "plugins/remem/runtimes/remem-releases.json"
NPM_PACKAGE_JSON = ROOT / "npm/remem/package.json"
MCP_SERVER_JSON = ROOT / "server.json"
NPM_INSTALL_JS = ROOT / "npm/remem/scripts/install.js"
NPM_INSTALL_VERSION_SOURCE = 'require("../package.json").version'
RELEASE_PLATFORMS = ("darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$", re.IGNORECASE)


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


def npm_version() -> str:
    version = read_json(NPM_PACKAGE_JSON).get("version")
    if not isinstance(version, str) or not version.strip():
        raise ValueError("npm/remem/package.json is missing version")
    return version


def mcp_server_json_errors(cargo: str) -> list[str]:
    manifest = read_json(MCP_SERVER_JSON)
    errors: list[str] = []
    version = manifest.get("version")
    if version != cargo:
        errors.append(f"server.json version is {version!r}, expected {cargo}")
    packages = manifest.get("packages")
    if not isinstance(packages, list) or not packages:
        errors.append("server.json is missing a non-empty packages array")
        return errors
    for index, package in enumerate(packages):
        if not isinstance(package, dict):
            errors.append(f"server.json packages[{index}] must be an object")
            continue
        package_version = package.get("version")
        if package_version != cargo:
            errors.append(
                f"server.json packages[{index}] version is {package_version!r}, expected {cargo}"
            )
    return errors


def npm_install_version_source_error() -> str | None:
    source = NPM_INSTALL_JS.read_text(encoding="utf-8")
    if NPM_INSTALL_VERSION_SOURCE not in source:
        return "npm/remem/scripts/install.js must read VERSION from package.json"
    if re.search(r'\bVERSION\s*=\s*["\']', source):
        return "npm/remem/scripts/install.js must not hardcode a VERSION string"
    return None


def release_metadata() -> tuple[set[str], str | None, dict, str | None]:
    manifest = read_json(RELEASES_JSON)
    versions = manifest.get("versions")
    if not isinstance(versions, dict):
        raise ValueError("plugins/remem/runtimes/remem-releases.json is missing versions object")
    keys = {str(key) for key in versions.keys()}
    if len(keys) != 1:
        return keys, None, {}, None
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
    state = release.get("state")
    if state is not None and state not in {"unreleased", "published"}:
        raise ValueError(f"release entry for {key} has unsupported state {state!r}")
    return keys, base_url, assets or {}, state


def release_metadata_errors(
    cargo: str,
    releases: set[str],
    base_url: str | None,
    assets: dict,
    state: str | None,
) -> list[str]:
    errors: list[str] = []
    if releases != {cargo}:
        rendered = ", ".join(sorted(releases)) or "<none>"
        errors.append(f"remem-releases.json versions are {{{rendered}}}, expected only {{{cargo}}}")
        return errors

    expected_suffix = f"/releases/download/v{cargo}"
    if not assets:
        if state != "unreleased":
            errors.append("empty remem-releases.json assets must set state to 'unreleased'")
        if base_url is not None:
            errors.append(
                "remem-releases.json has empty assets but still sets base_url; "
                "unpublished source versions must not point installers at a non-existent GitHub Release"
            )
        return errors

    if state == "unreleased":
        errors.append("remem-releases.json cannot set state='unreleased' when assets are present")

    if base_url is None:
        errors.append(f"remem-releases.json entry for {cargo} is missing base_url")
    elif not base_url.endswith(expected_suffix):
        errors.append(f"release base_url is {base_url}, expected suffix {expected_suffix}")

    expected_keys = set(RELEASE_PLATFORMS)
    actual_keys = set(assets.keys())
    missing = sorted(expected_keys - actual_keys)
    extra = sorted(actual_keys - expected_keys)
    if missing:
        errors.append(f"remem-releases.json is missing assets for: {', '.join(missing)}")
    if extra:
        errors.append(f"remem-releases.json has unsupported assets for: {', '.join(extra)}")

    for key in RELEASE_PLATFORMS:
        asset = assets.get(key)
        if not isinstance(asset, dict):
            continue
        expected_file = f"remem-{key}.tar.gz"
        file = asset.get("file")
        if file != expected_file:
            errors.append(f"asset {key} file is {file!r}, expected {expected_file!r}")
        sha = asset.get("sha256")
        if not isinstance(sha, str) or not SHA256_RE.match(sha):
            errors.append(f"asset {key} is missing a valid sha256")
    return errors


def changelog_current_version_error(cargo: str) -> str | None:
    text = CHANGELOG.read_text(encoding="utf-8")
    headings = list(re.finditer(r"^##\s+(.+)$", text, re.MULTILINE))
    if not headings:
        return "CHANGELOG.md is missing a version or Unreleased section"
    first = headings[0]
    title = first.group(1)
    body_start = first.end()
    body_end = headings[1].start() if len(headings) > 1 else len(text)
    body = text[body_start:body_end]
    if cargo in title:
        return None
    if "Unreleased" in title and cargo in body:
        return None
    return (
        f"CHANGELOG.md top section must be the current version {cargo} "
        "or an Unreleased section that mentions it"
    )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Require release metadata versions to match Cargo.toml."
    )
    parser.add_argument(
        "--expected-version",
        help="Optional bare version that Cargo.toml and release metadata must match.",
    )
    args = parser.parse_args()

    cargo = cargo_package_version()
    locked = lock_version()
    plugin = plugin_version()
    npm = npm_version()
    releases, base_url, assets, release_state = release_metadata()

    errors: list[str] = []
    if args.expected_version and cargo != args.expected_version:
        errors.append(f"Cargo.toml version is {cargo}, expected tag version {args.expected_version}")
    if locked != cargo:
        errors.append(f"Cargo.lock remem-ai version is {locked}, expected {cargo}")
    if plugin != cargo:
        errors.append(f"plugin.json version is {plugin}, expected {cargo}")
    if npm != cargo:
        errors.append(f"npm/remem/package.json version is {npm}, expected {cargo}")
    npm_install_error = npm_install_version_source_error()
    if npm_install_error is not None:
        errors.append(npm_install_error)
    errors.extend(mcp_server_json_errors(cargo))
    errors.extend(release_metadata_errors(cargo, releases, base_url, assets, release_state))
    changelog_error = changelog_current_version_error(cargo)
    if changelog_error is not None:
        errors.append(changelog_error)

    if errors:
        print("plugin version sync check failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        print(
            "Update Cargo.toml, Cargo.lock, plugins/remem/.codex-plugin/plugin.json, "
            "plugins/remem/runtimes/remem-releases.json, npm/remem/package.json, "
            "server.json, and CHANGELOG.md together.",
            file=sys.stderr,
        )
        return 1

    rendered_release_state = "published-assets" if assets else "staged-unpublished"
    print(
        "plugin version sync: "
        f"{cargo} across Cargo.toml, Cargo.lock, plugin.json, remem-releases.json, "
        f"npm, server.json, and CHANGELOG ({rendered_release_state})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
