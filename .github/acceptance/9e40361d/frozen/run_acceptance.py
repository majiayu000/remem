#!/usr/bin/env python3
"""Run explicitly selected gates on one actual fetched candidate SHA."""
from __future__ import annotations

import argparse
import datetime
import os
import re
import subprocess
import time
from pathlib import Path

from common import active_manifests, candidate_parents, config, dump, read_json, require, sha, source_identity


def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def record_tag(value):
    if len(value) > 32 or not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", value):
        raise argparse.ArgumentTypeError("record tag must be a lowercase alphanumeric slug of at most 32 characters")
    return value


def original_preflight_command(c):
    return ["python3", "scripts/ci/check_pr_preflight.py", "--base", c["stacked_base_sha"], "--pr-body-file", c["pr_body_file"]]


def execution_helpers(c, config_path):
    helpers_root = Path(__file__).resolve().parent
    source = Path(c["repo_dir"]) / "scripts/ci/check_pr_preflight.py"
    paths = {"config": Path(config_path).resolve(), "runner": Path(__file__).resolve(),
             "common": helpers_root / "common.py", "committed_preflight": source}
    actual = {name: {"file": str(path), "sha256": sha(path.read_bytes())} for name, path in paths.items()}
    require(actual["committed_preflight"]["sha256"] == c["preflight_source_sha256"],
            "committed preflight differs from the reviewed producer source")
    return actual


def gates(c):
    threads = str(c["test_threads"])
    return {
        "default_check": ["cargo", "check", "--locked"],
        "public_claims_self_test": ["python3", "scripts/ci/check_public_claims.py", "--self-test"],
        "committed_root_verifier": ["cargo", "run", "--locked", "--", "bench", "verify", "--root", "eval/public", "--json-out", "{output_json}"],
        "committed_root_eval_gates": ["cargo", "run", "--locked", "--", "eval-gates", "--json-out", "{output_json}"],
        "consumer_report_path_regression": ["cargo", "test", "--locked", "--features", "eval", "--lib", "eval::ship_matrix::tests::consumer_convergence::stale_security_report_tree_cannot_pass_current_implementation_gate", "--", "--exact", "--nocapture", "--test-threads", "1"],
        "snapshot_mutation_regression": ["cargo", "test", "--locked", "--features", "eval", "--lib", "eval::bench_artifact::tests::security_verification::verifier_rejects_placeholder_security_snapshot", "--", "--exact", "--nocapture", "--test-threads", "1"],
        "eval_library_and_e2e": ["cargo", "test", "--locked", "--features", "eval", "--lib", "eval", "--test", "e2e_eval", "--", "--test-threads", threads],
        "production_integration_doc": ["cargo", "test", "--locked", "--no-default-features", "--features", "local-onnx", "--", "--test-threads", threads],
        "full_preflight": original_preflight_command(c),
    }


def identity(c, publication, expected):
    parents = candidate_parents(c)
    require(publication["parent_sha"] == c["producer_sha"], "manifest producer differs")
    require(publication["parents"] == parents, "manifest parents differ from configured ordered parents")
    actual = source_identity(c, expected, clean=True)
    require(actual["tree"] == publication["expected_tree"] and actual["parents"] == parents, "candidate tree/parents differ from approved manifest")
    body = Path(c["pr_body_file"])
    require(sha(body.read_bytes()) == publication["pr_body"]["sha256"], "frozen intended PR body changed")
    active_manifests(c)
    actual.update({"clean": True, "intended_pr_body_sha256": publication["pr_body"]["sha256"]})
    return actual


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--config", required=True)
    p.add_argument("--sha")
    p.add_argument("--gate", action="append", help="explicit gate name; no implicit full-suite execution")
    p.add_argument("--list", action="store_true")
    p.add_argument("--verify-identity", action="store_true")
    p.add_argument("--continue-after-failure", action="store_true")
    p.add_argument("--record-tag", type=record_tag, help="isolate this explicit gate group's evidence files; concurrent groups must use distinct tags")
    args = p.parse_args()
    c = config(args.config)
    commands = gates(c)
    if args.list:
        for name, command in commands.items():
            print(name, command)
        return
    require(args.sha, "--sha is required for an actual acceptance run")
    require(args.verify_identity or args.gate, "select gates explicitly or use --verify-identity")
    require(all(g in commands for g in args.gate or []), "unknown gate")
    output = Path(c["output_dir"])
    suffix = "-" + args.record_tag if args.record_tag else ""
    publication = read_json(output / "publication.json")
    binding = identity(c, publication, args.sha)
    if args.verify_identity:
        dump(output / f"exact-candidate-identity{suffix}.json", binding)
        print(binding)
        if not args.gate:
            return
    helpers = execution_helpers(c, args.config)
    frozen_helpers = read_json(output / "execution-helper-freeze.json")
    require(frozen_helpers["candidate_identity"] == binding and frozen_helpers["helpers"] == helpers,
            "actual candidate execution helpers/configuration must match their prior freeze")
    env = os.environ.copy()
    overrides = {"PATH": c["cargo_bin_dir"] + ":" + env.get("PATH", ""), "RUSTUP_TOOLCHAIN": c["rust_toolchain"], "CARGO_TARGET_DIR": c["cargo_target_dir"],
        "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0", "CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "1", "CARGO_NET_OFFLINE": "true",
        "ORT_LIB_PATH": c["ort_lib_dir"], "ORT_PREFER_DYNAMIC_LINK": "1", "LD_LIBRARY_PATH": c["ort_lib_dir"], "RUST_TEST_THREADS": str(c["test_threads"]),
        "RUST_BACKTRACE": "1", "CARGO_TERM_COLOR": "never", "PYTHONUNBUFFERED": "1", "PYTHONPATH": c["python_deps_dir"]}
    env.update(overrides)
    results = output / f"acceptance-results{suffix}.json"
    record = read_json(results) if results.exists() else {"schema_version": 4, "record_tag": args.record_tag, "identity": binding, "environment": overrides, "execution_helpers": helpers, "started_at": utc(), "gates": []}
    require(record["identity"] == binding and record["environment"] == overrides and record["execution_helpers"] == helpers,
            "existing results belong to another candidate/body/environment/helper set")
    require(record.get("record_tag") == args.record_tag, "existing results belong to another record tag")
    selected_entries = []
    for name in args.gate or []:
        identity(c, publication, args.sha)
        require(execution_helpers(c, args.config) == helpers, "frozen execution helpers changed")
        attempt = 1 + sum(g["name"] == name for g in record["gates"])
        stem = f"{name}{suffix}-attempt-{attempt}"
        log, report = output / (stem + ".log"), output / (stem + ".json")
        require(not log.exists() and not report.exists(), "existing evidence will not be overwritten")
        command = [str(report) if part == "{output_json}" else part for part in commands[name]]
        entry = {"name": name, "attempt": attempt, "command": command, "cwd": c["repo_dir"], "log": str(log), "started_at": utc(), "status": "running"}
        if name == "full_preflight":
            entry.update(source_command_unmodified=True,
                         committed_preflight_sha256=helpers["committed_preflight"]["sha256"])
        record["gates"].append(entry)
        selected_entries.append(entry)
        dump(results, record)
        print("START", name, entry["started_at"], flush=True)
        start = time.monotonic()
        try:
            with log.open("x") as handle:
                handle.write(str({"record_tag": args.record_tag, "identity": binding, "environment": overrides, "execution_helpers": helpers, "command": command}) + "\n")
                handle.flush()
                result = subprocess.run(command, cwd=c["repo_dir"], env=env, stdout=handle, stderr=subprocess.STDOUT, check=False)
            entry.update(returncode=result.returncode, status="passed" if result.returncode == 0 else "failed")
        except BaseException as error:
            entry.update(status="interrupted" if isinstance(error, KeyboardInterrupt) else "execution_error", error=repr(error))
            raise
        finally:
            entry.update(elapsed_seconds=round(time.monotonic() - start, 3), ended_at=utc())
            if log.exists():
                entry["log_sha256"] = sha(log.read_bytes())
            if report.exists():
                entry.update(output_json=str(report), output_sha256=sha(report.read_bytes()))
            try:
                entry["post_identity"] = identity(c, publication, args.sha)
                require(execution_helpers(c, args.config) == helpers, "frozen execution helpers changed during gate")
            except Exception as error:
                entry.update(status="failed", post_identity_error=repr(error))
            record["updated_at"] = utc()
            dump(results, record)
        print("END", name, entry["status"], entry.get("returncode"), entry["elapsed_seconds"], flush=True)
        if entry["status"] != "passed" and not args.continue_after_failure:
            break
    require(all(g["status"] == "passed" for g in selected_entries), "one or more selected gates failed; original logs retained")


if __name__ == "__main__":
    main()
