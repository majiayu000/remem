#!/usr/bin/env python3
"""Build remem and run the isolated SessionStart smoke against Cargo's artifact."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CARGO_COMMAND = [
    "cargo",
    "build",
    "--locked",
    "--bin",
    "remem",
    "--message-format=json",
]
SMOKE_SCRIPT = ROOT / "scripts/ci/smoke_sessionstart_context_gate.sh"
HOSTILE_PARENT_ENV = {
    "HOME": "/nonexistent/remem-smoke-parent-home",
    "XDG_CONFIG_HOME": "/nonexistent/remem-smoke-parent-xdg-config",
    "XDG_DATA_HOME": "/nonexistent/remem-smoke-parent-xdg-data",
    "REMEM_CONTEXT_HOST": "claude-code",
    "REMEM_CONTEXT_GATE": "off",
    "REMEM_CONTEXT_GATE_HOSTS": "claude-code",
    "REMEM_CONTEXT_DEBUG": "1",
    "REMEM_CONTEXT_GATE_RETENTION_DAYS": "0",
    "REMEM_CONTEXT_BUNDLE_RENDER_MODE": "invalid",
    "REMEM_CONTEXT_TOTAL_CHAR_LIMIT": "invalid",
    "REMEM_UNDECLARED_PARENT_SENTINEL": "hostile",
}


def cargo_messages(output: str) -> list[dict[str, object]]:
    messages: list[dict[str, object]] = []
    for line in output.splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            messages.append(value)
    return messages


def parse_remem_executable(output: str) -> Path | None:
    executable: Path | None = None
    for message in cargo_messages(output):
        target = message.get("target")
        if (
            message.get("reason") != "compiler-artifact"
            or not isinstance(target, dict)
            or target.get("name") != "remem"
            or "bin" not in target.get("kind", [])
            or not isinstance(message.get("executable"), str)
        ):
            continue
        candidate = Path(message["executable"])
        executable = candidate if candidate.is_absolute() else (ROOT / candidate).resolve()
    return executable


def print_cargo_diagnostics(output: str) -> None:
    for message in cargo_messages(output):
        diagnostic = message.get("message")
        if not isinstance(diagnostic, dict):
            continue
        rendered = diagnostic.get("rendered")
        if isinstance(rendered, str):
            sys.stderr.write(rendered)


def main() -> int:
    build = subprocess.run(
        CARGO_COMMAND,
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if build.stderr:
        sys.stderr.write(build.stderr)
    print_cargo_diagnostics(build.stdout)
    if build.returncode != 0:
        print(
            f"error: Cargo failed to build the SessionStart smoke binary "
            f"(exit {build.returncode})",
            file=sys.stderr,
        )
        return 1

    executable = parse_remem_executable(build.stdout)
    if executable is None:
        print(
            "error: Cargo did not report an executable artifact for the remem binary",
            file=sys.stderr,
        )
        return 1
    if not executable.is_file() or not os.access(executable, os.X_OK):
        print(
            f"error: Cargo reported a missing or non-executable remem artifact: {executable}",
            file=sys.stderr,
        )
        return 1

    smoke_env = os.environ.copy()
    smoke_env.update(HOSTILE_PARENT_ENV)
    smoke = subprocess.run(
        [str(SMOKE_SCRIPT), str(executable)],
        cwd=ROOT,
        env=smoke_env,
        check=False,
    )
    return smoke.returncode


if __name__ == "__main__":
    raise SystemExit(main())
