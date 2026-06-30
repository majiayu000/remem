#!/usr/bin/env bash
set -euo pipefail

: "${TAG:?TAG is required}"
: "${VERSION:?VERSION is required}"
: "${GITHUB_OUTPUT:?GITHUB_OUTPUT is required}"

git fetch --force --tags origin
head_sha="$(git rev-parse HEAD)"

if git show-ref --tags --verify --quiet "refs/tags/${TAG}"; then
  tag_sha="$(git rev-list -n 1 "${TAG}")"
  {
    echo "exists=true"
    echo "tag_sha=${tag_sha}"
  } >> "$GITHUB_OUTPUT"

  if [ "$tag_sha" = "$head_sha" ]; then
    echo "${TAG} already points at ${head_sha}; no-op"
  else
    echo "${TAG} already points at ${tag_sha}; leaving immutable tag in place"
  fi
  exit 0
fi

latest="$(git tag -l 'v[0-9]*.[0-9]*.[0-9]*' --sort=-version:refname | head -n 1 || true)"
if [ -n "$latest" ]; then
  LATEST_TAG="$latest" python3 - <<'PY'
import os
import re
import sys


def parse(version: str) -> tuple[int, int, int]:
    match = re.fullmatch(r"v?(\d+)\.(\d+)\.(\d+)", version)
    if not match:
        print(f"unsupported release tag: {version}", file=sys.stderr)
        raise SystemExit(1)
    return tuple(int(part) for part in match.groups())


current = os.environ["VERSION"]
latest = os.environ["LATEST_TAG"]
if parse(current) <= parse(latest):
    print(
        f"source version {current} is not newer than latest tag {latest}",
        file=sys.stderr,
    )
    raise SystemExit(1)
PY
fi

{
  echo "exists=false"
  echo "tag_sha="
} >> "$GITHUB_OUTPUT"
