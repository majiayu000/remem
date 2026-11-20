#!/usr/bin/env python3
"""Read back the exact npm or MCP Registry version described by a manifest."""

import argparse
import json
from pathlib import Path
import sys
import time
from urllib.error import HTTPError
from urllib.parse import quote
from urllib.request import Request, urlopen


def published(channel, manifest):
    name, version = manifest["name"], manifest["version"]
    encoded_name = quote(name, safe="")
    encoded_version = quote(version, safe="")
    if channel == "npm":
        url = f"https://registry.npmjs.org/{encoded_name}/{encoded_version}"
    else:
        url = f"https://registry.modelcontextprotocol.io/v0.1/servers/{encoded_name}/versions/{encoded_version}"
    request = Request(url, headers={"Accept": "application/json", "Cache-Control": "no-cache"})
    try:
        with urlopen(request, timeout=15) as response:
            record = json.load(response)
    except HTTPError as error:
        if error.code == 404:
            return False
        raise

    if channel == "mcp":
        record = record["server"]
        for package in manifest["packages"]:
            if not any(
                all(candidate.get(key) == package[key] for key in ("registryType", "identifier", "version"))
                for candidate in record.get("packages", [])
            ):
                raise ValueError(f"MCP Registry has a different package for {name}@{version}")
    if record.get("name") != name or record.get("version") != version:
        raise ValueError(f"{channel} returned a different record for {name}@{version}")
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("channel", choices=("npm", "mcp"))
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--wait", action="store_true", help="Require the published version, allowing time for propagation")
    args = parser.parse_args()
    try:
        manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
        attempts = 6 if args.wait else 1
        for attempt in range(attempts):
            if published(args.channel, manifest):
                print("true")
                return 0
            if attempt + 1 < attempts:
                time.sleep(3)
        if args.wait:
            raise ValueError(f"{manifest['name']}@{manifest['version']} is unavailable on {args.channel}")
        print("false")
        return 0
    except (KeyError, OSError, ValueError) as error:
        print(f"Registry check failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
