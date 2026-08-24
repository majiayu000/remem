#!/usr/bin/env python3
"""Discover normalized schemas from remem's actually served MCP tools/list response."""

from __future__ import annotations

import hashlib
import json
import os
import select
import subprocess
import tempfile
import time
from pathlib import Path


def discover_mcp_schema_fingerprints(root: Path) -> dict[str, str]:
    with tempfile.TemporaryDirectory(prefix="remem-surface-mcp-") as data_dir:
        env = os.environ.copy()
        env["REMEM_DATA_DIR"] = data_dir
        env["REMEM_ALLOW_PLAINTEXT_DB"] = "1"
        process = subprocess.Popen(
            ["cargo", "run", "--quiet", "--", "mcp"],
            cwd=root,
            env=env,
            text=True,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        assert process.stdin is not None and process.stdout is not None

        def send(message: dict[str, object]) -> None:
            process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
            process.stdin.flush()

        def receive(response_id: int, timeout: float = 120.0) -> dict[str, object]:
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    detail = process.stderr.read().strip() if process.stderr else ""
                    raise RuntimeError(f"remem mcp exited before response {response_id}: {detail}")
                ready, _, _ = select.select([process.stdout], [], [], min(1.0, deadline - time.monotonic()))
                if not ready:
                    continue
                line = process.stdout.readline()
                if not line:
                    continue
                message = json.loads(line)
                if message.get("id") == response_id:
                    return message
            raise RuntimeError(f"timed out waiting for remem mcp response {response_id}")

        try:
            send({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26", "capabilities": {},
                    "clientInfo": {"name": "surface-lifecycle-guard", "version": "1"},
                },
            })
            receive(1)
            send({"jsonrpc": "2.0", "method": "notifications/initialized"})
            send({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
            response = receive(2)
        finally:
            process.stdin.close()
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=15)

    tools = response.get("result", {}).get("tools") if isinstance(response.get("result"), dict) else None
    if not isinstance(tools, list) or not tools:
        raise RuntimeError("served MCP tools/list response has no tools")
    fingerprints: dict[str, str] = {}
    for tool in tools:
        if not isinstance(tool, dict) or not isinstance(tool.get("name"), str):
            raise RuntimeError("served MCP tool descriptor is malformed")
        name = tool["name"]
        schemas = {"inputSchema": tool.get("inputSchema"), "outputSchema": tool.get("outputSchema")}
        normalized = json.dumps(schemas, sort_keys=True, separators=(",", ":"))
        if name in fingerprints:
            raise RuntimeError(f"served MCP tools/list contains duplicate {name!r}")
        fingerprints[name] = hashlib.sha256(normalized.encode()).hexdigest()
    return fingerprints
