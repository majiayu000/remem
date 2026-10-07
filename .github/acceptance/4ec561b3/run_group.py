#!/usr/bin/env python3
"""Launch unchanged reviewed helpers; preserve every batch's real exit status."""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess
import sys
import time

from bootstrap import (CONFIG, FROZEN, GROUPS, OUTPUT, Q, SHORT_GATES, SOURCE,
                       current_identity, read, resource_check, stable_environment, utc, write)


def invoke(command, label, environment):
    start = time.monotonic()
    with (OUTPUT / (label + ".log")).open("x") as log:
        result = subprocess.run(command, cwd=SOURCE, env=environment, stdout=log, stderr=subprocess.STDOUT, check=False)
    entry = {"command": command, "returncode": result.returncode, "elapsed_seconds": round(time.monotonic() - start, 3),
             "ended_at": utc(), "log": str(OUTPUT / (label + ".log"))}
    print(label, result.returncode, entry["elapsed_seconds"], flush=True)
    return entry


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--group", choices=GROUPS, required=True)
    args = parser.parse_args()
    if os.environ["ACCEPTANCE_GROUP"] != args.group:
        raise ValueError("matrix group differs from explicit group")
    env = stable_environment()
    before = current_identity()
    resource_check("before-layout")
    command = ["python3", "-B", str(FROZEN / "prepare_target_layout.py"), "--config", str(CONFIG), "--sha", Q]
    prepared = invoke(command, "prepare-target-layout", env)
    write(OUTPUT / "layout-command.json", prepared)
    if prepared["returncode"] != 0:
        return prepared["returncode"]
    batches = [GROUPS["eval"]] if args.group == "eval" else [SHORT_GATES, ["full_preflight"]]
    invocations = []
    try:
        for index, gates in enumerate(batches):
            resource_check(f"before-{args.group}-batch-{index + 1}")
            command = ["python3", "-B", str(FROZEN / "run_acceptance.py"), "--config", str(CONFIG), "--sha", Q,
                       "--record-tag", args.group, "--verify-identity", "--continue-after-failure"]
            for gate in gates:
                command.extend(["--gate", gate])
            invocations.append(invoke(command, f"runner-{args.group}-batch-{index + 1}", env))
            write(OUTPUT / "runner-invocations.json", {"group": args.group, "invocations": invocations})
    finally:
        write(OUTPUT / "group-post-identity.json", current_identity())
    record = read(OUTPUT / f"acceptance-results-{args.group}.json")
    valid = (record["identity"] == before and [entry["name"] for entry in record["gates"]] == GROUPS[args.group]
             and all(entry["status"] == "passed" and entry.get("returncode") == 0 for entry in record["gates"])
             and all(invocation["returncode"] == 0 for invocation in invocations))
    write(OUTPUT / "group-exit.json", {"group": args.group, "all_original_gates_passed": valid,
                                      "invocations": invocations, "scope": "exact Q6 on this isolated GitHub runner"})
    return 0 if valid else 1


if __name__ == "__main__":
    sys.exit(main())
