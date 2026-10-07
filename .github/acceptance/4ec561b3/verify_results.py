#!/usr/bin/env python3
"""Fail closed on the original files from both exact-Q acceptance runners."""
from __future__ import annotations

import argparse
import json
from pathlib import Path, PurePosixPath
import re
import sys

from bootstrap import (BASE_PATH, CONFIG, ENVIRONMENT_PATHS, FROZEN, GROUPS, OUTPUT,
                       PARENTS, Q, SOURCE, STATE, SUPPORT, TREE, adapted_config,
                       original_modules, read, require, sha, utc, verify_inputs, write)


def safe_file(root, relative):
    relative = PurePosixPath(relative)
    require(not relative.is_absolute() and ".." not in relative.parts, "unsafe artifact-relative path")
    path = root / str(relative)
    require(path.is_file() and not path.is_symlink() and root.resolve() in path.resolve().parents, "artifact file missing or escaped its root")
    return path


def verify_file_index(root):
    index = read(root / "artifact-index.json")
    require(index["candidate_sha"] == Q, "artifact index belongs to another candidate")
    names = [entry["path"] for entry in index["files"]]
    require(len(names) == len(set(names)), "duplicate artifact index path")
    actual = {path.relative_to(root).as_posix() for path in root.rglob("*") if path.is_file()}
    require(actual == set(names) | {"artifact-index.json"}, "artifact index does not cover the complete output set")
    for item in index["files"]:
        data = safe_file(root, item["path"]).read_bytes()
        require(len(data) == item["bytes"] and sha(data) == item["sha256"], "artifact file hash/size mismatch: " + item["path"])


def artifact_path(root, original):
    relative = Path(original).relative_to(OUTPUT)
    return safe_file(root, relative.as_posix())


def test_summaries(text):
    return [(status, int(passed), int(failed), int(ignored)) for status, passed, failed, ignored in
            re.findall(r"^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;", text, re.MULTILINE)]


def require_test_activity(name, text):
    if name in {"consumer_report_path_regression", "snapshot_mutation_regression"}:
        summaries = test_summaries(text)
        require(len(summaries) == 1 and summaries[0][0:3] == ("ok", 1, 0), "exact regression did not execute one passing test")
    elif name == "eval_library_and_e2e":
        summaries = test_summaries(text)
        require(len(summaries) == 2 and all(status == "ok" and passed > 0 and failed == 0 for status, passed, failed, _ in summaries),
                "both eval library and e2e must execute nonzero passing tests")
        require("Running unittests src/lib.rs" in text and "Running tests/e2e_eval.rs" in text, "eval/e2e executable evidence missing")
    elif name == "full_preflight":
        require("\n== Summary" in text, "full preflight summary missing")
        summary = text.rsplit("\n== Summary", 1)[1]
        require("preflight passed" in summary and not re.search(r"^(FAIL|SKIP)\b", summary, re.MULTILINE), "full preflight reported failure or skip")
        marker = "==> Run production cargo tests without eval"
        require(marker in text, "complete production/integration/doc test command did not run")
        production = text.rsplit(marker, 1)[1].split("\n== Summary", 1)[0]
        require("+ cargo test --no-default-features --features local-onnx -- --test-threads 4" in production,
                "full preflight changed the complete production command")
        summaries = test_summaries(production)
        require(summaries and sum(passed for _, passed, _, _ in summaries) > 0
                and all(status == "ok" and failed == 0 for status, _, failed, _ in summaries), "production tests did not actually pass")
        require("Running unittests " in production and "Running tests/" in production and "Doc-tests remem" in production,
                "production, integration or doc test execution evidence missing")
    elif name == "public_claims_self_test":
        require("public claims check self-test: ok" in text, "public-claims self-test activity missing")


def require_coverage(group, entries):
    require([entry["name"] for entry in entries] == GROUPS[group], "gate coverage omitted, duplicated, reordered or substituted a required gate")


def expected_environment(c):
    return {
        "PATH": c["cargo_bin_dir"] + ":" + BASE_PATH, "RUSTUP_TOOLCHAIN": c["rust_toolchain"], "CARGO_TARGET_DIR": c["cargo_target_dir"],
        "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0", "CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "1", "CARGO_NET_OFFLINE": "true",
        "ORT_LIB_PATH": c["ort_lib_dir"], "ORT_PREFER_DYNAMIC_LINK": "1", "LD_LIBRARY_PATH": c["ort_lib_dir"], "RUST_TEST_THREADS": str(c["test_threads"]),
        "RUST_BACKTRACE": "1", "CARGO_TERM_COLOR": "never", "PYTHONUNBUFFERED": "1", "PYTHONPATH": c["python_deps_dir"],
    }



def verify_system_ort(root, host, binding):
    from ort_system import FILENAME, PINNED_BYTES, PINNED_SHA256, VERSION, verify_probe_elf
    destination = "/usr/local/lib/" + FILENAME
    receipt = read(root / "system-ort-preparation.json")
    require(receipt["passed"] is True and receipt["before_identity"] == binding
            and receipt["after_identity"] == binding and "error" not in receipt
            and "post_identity_error" not in receipt, "system ORT preparation failed or changed identity")
    package = receipt["package"]
    installs = read(root / "ort-dependencies.json")["install"]
    ort = [item for item in installs if item["metadata"]["name"].lower().replace("_", "-") == "onnxruntime"]
    require(len(ort) == 1 and ort[0]["metadata"]["version"] == VERSION, "system ORT pip version differs")
    require(package["version"] == VERSION and package["library_sha256"] == PINNED_SHA256
            and package["library_bytes"] == PINNED_BYTES
            and package["wheel_sha256"] == ort[0]["download_info"]["archive_info"]["hashes"]["sha256"]
            and re.fullmatch(r"[0-9a-f]{64}", package["wheel_sha256"]), "system ORT package binding differs")
    expected = {"version": VERSION, "path": destination, "sha256": PINNED_SHA256, "bytes": PINNED_BYTES,
                "links": {name: FILENAME for name in ("libonnxruntime.so", "libonnxruntime.so.1")},
                "loader_cache": {"soname": "libonnxruntime.so.1", "path": "/usr/local/lib/libonnxruntime.so.1", "resolved_path": destination},
                "empty_environment_loaded_path": destination, "probe_requires_soname": "libonnxruntime.so.1",
                "probe_has_rpath_or_runpath": False, "package": package}
    require(host["system_ort"] == expected and receipt["installed_identity"] == expected,
            "installed system ORT identity differs from the fixed ELF and loader contract")
    entries = receipt["commands"]
    phases = [entry["phase"] for entry in entries]
    required = ["compile-probe", "probe-elf", "cache-before", "empty-environment-before",
                "refresh-cache", "cache-after", "empty-environment-after"]
    require(len(phases) == len(set(phases)) and [p for p in phases if p in required] == required,
            "system ORT probe phases are missing, reordered or duplicated")
    require(set(phases) <= set(required + ["install", "link-libonnxruntime.so", "link-libonnxruntime.so.1"]),
            "unexpected system ORT command phase")
    logs = {}
    for entry in entries:
        phase = entry["phase"]
        require("execution_error" not in entry and isinstance(entry["returncode"], int)
                and (phase == "empty-environment-before" or entry["returncode"] == 0), "system ORT command failed")
        log = artifact_path(root, entry["log"])
        require(sha(log.read_bytes()) == entry["log_sha256"], "system ORT command log hash differs")
        logs[phase] = log.read_text(errors="replace")
        if phase.startswith("empty-environment-"):
            require(entry["command"] == ["/usr/bin/env", "-i", str(OUTPUT / "ort-loader-probe")],
                    "system ORT loader proof did not use a cleared environment")
    verify_probe_elf(logs["probe-elf"])
    paths = re.findall(r"^loaded=(.+)$", logs["empty-environment-after"], re.MULTILINE)
    require(len(paths) == 1 and paths[0] in {destination, "/usr/local/lib/libonnxruntime.so.1"},
            "actual empty-environment probe loaded an unexpected ORT path")
    require(re.search(r"^\s*libonnxruntime\.so\.1\s+\([^\n]+\)\s+=>\s+/usr/local/lib/libonnxruntime\.so\.1\s*$",
                      logs["cache-after"], re.MULTILINE), "actual loader cache evidence missing")


def verify_group(root, group, run_id, attempt, support_sha):
    verify_file_index(root)
    specification = verify_inputs()
    for name, item in specification["frozen_files"].items():
        require(sha(safe_file(root, "inputs/" + name).read_bytes()) == item["sha256"], "original frozen input changed in artifact")
    require((root / "inputs/input-hashes.json").read_bytes() == (SUPPORT / "input-hashes.json").read_bytes(), "input hash manifest changed")
    original = read(FROZEN / "config-original.json")
    expected_config, differences = adapted_config(original)
    c = read(root / "config.json")
    require(c == expected_config, "artifact configuration changed beyond approved paths")
    adaptation = read(root / "configuration-adaptation.json")
    require(adaptation["allowed_keys"] == sorted(ENVIRONMENT_PATHS) and adaptation["changed_values"] == differences
            and adaptation["original_sha256"] == sha((FROZEN / "config-original.json").read_bytes())
            and adaptation["adapted_sha256"] == sha((root / "config.json").read_bytes()), "environment-only adaptation proof differs")
    require((root / "publication.json").read_bytes() == (FROZEN / "publication.json").read_bytes(), "publication metadata changed")
    binding = {"sha": Q, "tree": TREE, "parents": PARENTS, "clean": True,
               "intended_pr_body_sha256": specification["frozen_files"]["body.md"]["sha256"],
               **{key: c[key] for key in ("production_input_tree_sha256", "production_pathspec_sha256", "suite_sha256")}}
    record = read(root / f"acceptance-results-{group}.json")
    require(record["record_tag"] == group and record["identity"] == binding, "ledger group or exact source/body identity differs")
    require(record["environment"] == expected_environment(c), "recorded execution environment differs")
    helpers = {
        "config": {"file": str(CONFIG), "sha256": sha((root / "config.json").read_bytes())},
        "runner": {"file": str(FROZEN / "run_acceptance.py"), "sha256": specification["frozen_files"]["run_acceptance.py"]["sha256"]},
        "common": {"file": str(FROZEN / "common.py"), "sha256": specification["frozen_files"]["common.py"]["sha256"]},
        "committed_preflight": {"file": str(SOURCE / "scripts/ci/check_pr_preflight.py"), "sha256": c["preflight_source_sha256"]},
    }
    require(record["execution_helpers"] == helpers, "ledger helper identity differs")
    frozen = read(root / "execution-helper-freeze.json")
    require(frozen["candidate_identity"] == binding and frozen["helpers"] == helpers, "execution helper/config freeze differs")
    require(sha((root / "inputs/committed-preflight.py").read_bytes()) == c["preflight_source_sha256"], "committed full preflight bytes differ")
    for filename in ("exact-source-before.json", "exact-source-after-dependencies.json", "group-post-identity.json", f"exact-candidate-identity-{group}.json"):
        require(read(root / filename) == binding, "pre/post source identity differs: " + filename)
    require(read(root / "collection-diagnostics.json").get("post_identity") == binding, "final artifact collection source identity differs")
    layout = read(root / "acceptance-environment-layout.json")
    require(layout["candidate_identity"] == binding and layout["candidate_physical_root"] == str(SOURCE)
            and layout["target_is_real_directory"] is True and layout["target_physical_path"] == str(SOURCE / "target")
            and layout["shared_target_physical_path"] == str(STATE / "target"), "candidate target isolation differs")
    host = read(root / "host-context.json")
    require(host["runner_environment"] == "github-hosted" and host["run_id"] == run_id and host["run_attempt"] == attempt and host["group"] == group
            and host["support_sha"] == support_sha and host["candidate_sha"] == Q
            and host["candidate_tree"] == TREE and host["candidate_parents"] == PARENTS,
            "runner execution provenance differs")
    require(host["source_root"] == str(SOURCE) and host["target_root"] == str(STATE / "target")
            and host["temporary_root"] == str(STATE / "tmp") and host["stable_base_path"] == BASE_PATH, "runner paths differ")
    require(host["tool_versions"]["rustc"]["version"].startswith("rustc 1.97.0 ") and host["ort_library"]["version"] == "1.24.2", "pinned dependency version differs")
    verify_system_ort(root, host, binding)
    require(read(root / "group-exit.json")["all_original_gates_passed"] is True, "group driver retained an actual failure")
    invocations = read(root / "runner-invocations.json")["invocations"]
    require(len(invocations) == (1 if group == "eval" else 2) and all(item["returncode"] == 0 for item in invocations), "an original runner invocation failed or is missing")
    resources = ["resource-after-dependencies.json", "resource-before-layout.json", f"resource-before-{group}-batch-1.json"]
    if group == "other":
        resources.append("resource-before-other-batch-2.json")
    for filename in resources:
        resource = read(root / filename)
        require(resource["passed"] is True and resource["minimum_available_bytes"] >= 20 * 1024**3
                and all(value["free_bytes"] >= resource["minimum_available_bytes"] for value in resource["measurements"].values()), "resource guard failed or was lowered")
    _, runner = original_modules()
    commands = runner.gates(c)
    require_coverage(group, record["gates"])
    reports = []
    for entry in record["gates"]:
        require(entry["attempt"] == 1 and entry["status"] == "passed" and entry["returncode"] == 0
                and entry["post_identity"] == binding and entry["elapsed_seconds"] > 0, "gate did not successfully execute on the bound candidate")
        log = artifact_path(root, entry["log"])
        require(sha(log.read_bytes()) == entry["log_sha256"], "gate log hash differs")
        command = [entry.get("output_json") if part == "{output_json}" else part for part in commands[entry["name"]]]
        require(entry["command"] == command and entry["cwd"] == str(SOURCE), "original gate command or working directory changed")
        text = log.read_text(errors="replace")
        require_test_activity(entry["name"], text)
        if "{output_json}" in commands[entry["name"]]:
            report = artifact_path(root, entry["output_json"])
            require(sha(report.read_bytes()) == entry["output_sha256"], "gate report hash differs")
            data = read(report)
            if entry["name"] == "committed_root_verifier":
                require(data["passed"] is True and not data["failures"] and data["run_artifacts_checked"] > 0, "committed verifier did not pass real artifacts")
                implementation = data["authority_verdict"]["implementation"]
                require(implementation["build_git_sha"] == Q and implementation["checkout_git_sha"] == Q, "verifier executable/checkout was not built from exact Q")
            else:
                require(data["summary"]["passed"] is True and data["summary"]["metrics_checked"] > 0, "committed eval gates did not pass real metrics")
        if entry["name"] == "full_preflight":
            require(entry["source_command_unmodified"] is True and entry["committed_preflight_sha256"] == c["preflight_source_sha256"], "full preflight source/command was altered")
        reports.append({"name": entry["name"], "log_sha256": entry["log_sha256"], "elapsed_seconds": entry["elapsed_seconds"],
                        "test_summaries": test_summaries(text), "output_sha256": entry.get("output_sha256")})
    return {"identity": binding, "environment": record["environment"], "execution_helpers": helpers,
            "host": host, "gates": reports}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--attempt", required=True)
    parser.add_argument("--support-sha", required=True)
    parser.add_argument("--job-result", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    verdict = {"candidate_sha": Q, "candidate_tree": TREE, "candidate_parents": PARENTS,
               "support_sha": args.support_sha, "run_id": args.run_id, "run_attempt": args.attempt,
               "recorded_at": utc(), "passed": False, "groups": {}, "errors": [],
               "scope": "Remote actual-Q acceptance on two independent GitHub Actions hosts; not a local-container test result"}
    for group in GROUPS:
        root = Path(args.artifacts) / f"remem-q6-acceptance-4ec561b3-{group}-attempt-{args.attempt}"
        try:
            verdict["groups"][group] = verify_group(root, group, args.run_id, args.attempt, args.support_sha)
        except Exception as error:
            verdict["errors"].append({"group": group, "error": repr(error)})
    try:
        require(args.job_result == "success", "one or more matrix jobs did not finish successfully")
        require(set(verdict["groups"]) == set(GROUPS), "both complete result groups are required")
        first, second = [verdict["groups"][group] for group in GROUPS]
        for key in ("identity", "environment", "execution_helpers"):
            require(first[key] == second[key], "cross-runner " + key + " differs")
        for key in ("tool_versions", "ort_library", "system_ort"):
            require(first["host"][key] == second["host"][key], "cross-runner toolchain/library differs")
        require(first["host"]["hostname"] != second["host"]["hostname"], "two independent execution hosts were not demonstrated")
        names = [entry["name"] for group in verdict["groups"].values() for entry in group["gates"]]
        require(len(names) == 8 and len(set(names)) == 8 and set(names) == set(GROUPS["eval"] + GROUPS["other"]), "final required eight-gate union differs")
        verdict["passed"] = not verdict["errors"]
    except Exception as error:
        verdict["errors"].append({"group": "aggregate", "error": repr(error)})
    write(args.output, verdict)
    print(json.dumps({"passed": verdict["passed"], "errors": verdict["errors"], "output": args.output}, indent=2))
    return 0 if verdict["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
