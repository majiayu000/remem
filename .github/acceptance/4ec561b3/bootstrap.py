#!/usr/bin/env python3
"""Prepare a disposable CI host around immutable Q6 inputs; never edit Q6."""
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys

Q = "4ec561b35932f4317b1c794c7364aad13c90b42f"
TREE = "6645e15dd6086ba2a9e568f8befafbaacfc6f7f9"
PARENTS = ["c144b36a39415a28cd761d48b5ff78c88a6dfb10", "9e40361d9346d09270864da675d3ef17aebd6ad7", "2619eb5ee28f7d63978fb1457be533656f122cd4"]
BRANCH = "refs/heads/audit/acceptance-runner-20261007-4ec561b3"
WORKSPACE = Path("/home/runner/work/remem/remem")
SUPPORT = WORKSPACE / "orchestration/.github/acceptance/4ec561b3"
SOURCE = WORKSPACE / "candidate"
STATE = WORKSPACE / "acceptance"
OUTPUT = STATE / "results"
FROZEN = SUPPORT / "frozen"
CONFIG = OUTPUT / "config.json"
BASE_PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
MIN_FREE = 20 * 1024**3
ENVIRONMENT_PATHS = {
    "repo_dir": str(SOURCE),
    "output_dir": str(OUTPUT),
    "pr_body_file": str(FROZEN / "body.md"),
    "epic_body_file": str(FROZEN / "epic-body.md"),
    "cargo_bin_dir": "/home/runner/.cargo/bin",
    "python_deps_dir": str(STATE / "python"),
    "cargo_target_dir": str(STATE / "target"),
    "ort_lib_dir": str(STATE / "onnx/onnxruntime/capi"),
}
EVAL_GATES = ["eval_library_and_e2e"]
SHORT_GATES = ["default_check", "public_claims_self_test", "committed_root_verifier",
               "committed_root_eval_gates", "consumer_report_path_regression", "snapshot_mutation_regression"]
OTHER_GATES = SHORT_GATES + ["full_preflight"]
GROUPS = {"eval": EVAL_GATES, "other": OTHER_GATES}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read(path):
    return json.loads(Path(path).read_bytes())


def write(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def git(directory, *args):
    return subprocess.check_output(["git", "--no-optional-locks", *args], cwd=directory, text=True).strip()


def adapted_config(original):
    adapted = {**original, **ENVIRONMENT_PATHS}
    changed = {key for key in set(original) | set(adapted) if original.get(key) != adapted.get(key)}
    require(changed <= set(ENVIRONMENT_PATHS), "non-environment configuration change")
    require(all(Path(value).is_absolute() for value in ENVIRONMENT_PATHS.values()), "environment paths must be absolute")
    require(set(original) == set(adapted), "configuration keys changed")
    require(all(adapted[key] == value for key, value in original.items() if key not in ENVIRONMENT_PATHS), "frozen configuration value changed")
    return adapted, {key: {"before": original[key], "after": adapted[key]} for key in sorted(changed)}


def stable_environment():
    env = dict(os.environ)
    env.update(PATH=BASE_PATH, TMPDIR=str(STATE / "tmp"),
               PYTHONDONTWRITEBYTECODE="1", PYTHONUNBUFFERED="1")
    return env


def verify_inputs():
    specification = read(SUPPORT / "input-hashes.json")
    require(specification["candidate_sha"] == Q and specification["candidate_tree"] == TREE
            and specification["candidate_parents"] == PARENTS, "support candidate binding differs")
    require(set(specification["frozen_files"]) == {p.name for p in FROZEN.iterdir() if p.is_file()}, "frozen input set differs")
    for name, expected in specification["frozen_files"].items():
        path = FROZEN / name
        require(path.is_file() and not path.is_symlink(), "frozen input is not a regular file")
        data = path.read_bytes()
        require(len(data) == expected["bytes"] and sha(data) == expected["sha256"], f"frozen input changed: {name}")
    return specification


def verify_host_and_source():
    require(os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted",
            "environment mutation requires a GitHub-hosted Actions VM")
    require(os.environ.get("GITHUB_SERVER_URL") == "https://github.com", "unexpected GitHub server")
    require(Path(os.environ["GITHUB_WORKSPACE"]).resolve() == WORKSPACE, "unexpected hosted workspace path")
    require(os.environ["GITHUB_REPOSITORY"] == "majiayu000/remem", "unexpected repository")
    require(os.environ["GITHUB_EVENT_NAME"] == "push" and os.environ["GITHUB_REF"] == BRANCH, "unexpected support trigger")
    require(os.environ["RUNNER_OS"] == "Linux" and os.environ["RUNNER_ARCH"] == "X64", "expected independent Linux x64 host")
    require(SUPPORT.resolve() == SUPPORT and SOURCE.resolve() == SOURCE, "source/support path is a symlink")
    require(git(WORKSPACE / "orchestration", "rev-parse", "HEAD") == os.environ["GITHUB_SHA"], "support checkout differs from event")
    require(git(WORKSPACE / "orchestration", "show", "-s", "--format=%P", "HEAD").split() == [Q], "support must have Q as its sole parent")
    changes = git(WORKSPACE / "orchestration", "diff", "--name-status", Q, "HEAD").splitlines()
    require(changes and all(line.startswith("A\t") and (line.split("\t")[1].startswith(".github/acceptance/4ec561b3/")
            or line.split("\t")[1] == ".github/workflows/q6-exact-acceptance.yml") for line in changes), "support changed files outside its new orchestration paths")
    require(git(SOURCE, "rev-parse", "HEAD") == Q and git(SOURCE, "rev-parse", "HEAD^{tree}") == TREE, "wrong candidate checkout")
    require(git(SOURCE, "show", "-s", "--format=%P", "HEAD").split() == PARENTS, "wrong candidate ordered parents")
    require(not git(SOURCE, "status", "--porcelain", "--untracked-files=all"), "candidate checkout is not clean")
    require(git(SOURCE, "config", "--get", "remote.origin.url") in
            {"https://github.com/majiayu000/remem", "https://github.com/majiayu000/remem.git"}, "unexpected candidate remote")
    return changes


def original_modules():
    sys.path.insert(0, str(FROZEN))
    import common
    import run_acceptance
    return common, run_acceptance


def current_identity():
    verify_inputs()
    common, runner = original_modules()
    c = common.config(CONFIG)
    expected, _ = adapted_config(read(FROZEN / "config-original.json"))
    require(c == expected, "runtime configuration differs from the approved environment-only adaptation")
    return runner.identity(c, read(FROZEN / "publication.json"), Q)


def resource_measurement():
    paths = {"source": SOURCE, "target": STATE / "target", "temporary": STATE / "tmp"}
    measurements = {name: {"path": str(path), "free_bytes": shutil.disk_usage(path).free,
                            "device": path.stat().st_dev} for name, path in paths.items()}
    return {"recorded_at": utc(), "minimum_available_bytes": MIN_FREE, "measurements": measurements,
            "passed": all(value["free_bytes"] >= MIN_FREE for value in measurements.values())}


def reclaim_unused_android(label):
    # This fixed path is authorized only on the disposable, authenticated CI VM.
    # Check the platform first, before any Git reads or subprocess mutation.
    require(os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted",
            "Android cleanup is forbidden outside GitHub-hosted Actions")
    verify_host_and_source()
    current_identity()
    android = Path("/usr/local/lib/android")
    require(android.is_dir() and not android.is_symlink() and android.resolve() == android,
            "the sole authorized reclaimable directory is absent or non-canonical")
    command = ["sudo", "-n", "rm", "-rf", "--", str(android)]
    with (OUTPUT / f"resource-{label}-android-cleanup.log").open("x") as log:
        result = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT, check=False)
    write(OUTPUT / f"resource-{label}-android-cleanup.json", {
        "command": command, "exact_path": str(android), "returncode": result.returncode,
        "recorded_at": utc(), "runner_environment": os.environ["RUNNER_ENVIRONMENT"],
        "candidate_sha": Q, "support_sha": os.environ["GITHUB_SHA"],
    })
    require(result.returncode == 0, "authorized Android cleanup failed")


def resource_check(label):
    record = resource_measurement()
    if not record["passed"]:
        write(OUTPUT / f"resource-{label}-before-reclaim.json", record)
        try:
            reclaim_unused_android(label)
        finally:
            record = resource_measurement()
            write(OUTPUT / f"resource-{label}.json", record)
    write(OUTPUT / f"resource-{label}.json", record)
    require(record["passed"], "at least 20 GiB actually available is required before a gate batch")
    return record


def logged(command, filename, *, cwd=SOURCE, env=None):
    print("SETUP", filename, flush=True)
    with (OUTPUT / filename).open("x") as log:
        log.write(json.dumps({"command": command, "cwd": str(cwd), "scope": "environment preparation, not an acceptance gate"}) + "\n")
        log.flush()
        result = subprocess.run(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT, check=False)
    require(result.returncode == 0, f"environment preparation failed ({result.returncode}): {filename}")


def initialize():
    require(not STATE.exists(), "fresh isolated runner state is required")
    OUTPUT.mkdir(parents=True)
    for directory in (STATE / "tmp", STATE / "target/doc", STATE / "target/debug"):
        directory.mkdir(parents=True, exist_ok=True)
    specification = verify_inputs()
    changes = verify_host_and_source()
    original = read(FROZEN / "config-original.json")
    c, differences = adapted_config(original)
    write(CONFIG, c)
    for name in specification["frozen_files"]:
        destination = OUTPUT / "inputs" / name
        destination.parent.mkdir(exist_ok=True)
        shutil.copyfile(FROZEN / name, destination)
    shutil.copyfile(SUPPORT / "input-hashes.json", OUTPUT / "inputs/input-hashes.json")
    shutil.copyfile(FROZEN / "publication.json", OUTPUT / "publication.json")
    shutil.copyfile(SOURCE / "scripts/ci/check_pr_preflight.py", OUTPUT / "inputs/committed-preflight.py")
    write(OUTPUT / "configuration-adaptation.json", {
        "allowed_keys": sorted(ENVIRONMENT_PATHS), "changed_values": differences,
        "original_sha256": sha((FROZEN / "config-original.json").read_bytes()),
        "adapted_sha256": sha(CONFIG.read_bytes()), "only_environment_paths_changed": True,
        "source_body_commands_test_threads_and_producer_authority_unchanged": True,
    })
    write(OUTPUT / "exact-source-before.json", current_identity())
    write(OUTPUT / "host-context.json", {
        "scope": "Remote GitHub Actions acceptance of exact Q; not execution in the shared local container",
        "recorded_at": utc(), "hostname": socket.gethostname(),
        "run_id": os.environ["GITHUB_RUN_ID"], "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"],
        "job": os.environ["GITHUB_JOB"], "group": os.environ["ACCEPTANCE_GROUP"],
        "support_sha": os.environ["GITHUB_SHA"], "candidate_sha": Q, "candidate_tree": TREE,
        "candidate_parents": PARENTS, "support_only_changes": changes,
        "workspace": str(WORKSPACE), "source_root": str(SOURCE), "target_root": str(STATE / "target"),
        "temporary_root": str(STATE / "tmp"), "stable_base_path": BASE_PATH,
        "runner_name": os.environ.get("RUNNER_NAME"), "runner_image": os.environ.get("ImageOS"),
        "runner_environment": os.environ["RUNNER_ENVIRONMENT"],
        "runner_image_version": os.environ.get("ImageVersion"),
        "meminfo": Path("/proc/meminfo").read_text(),
    })


def dependencies():
    current_identity()
    env = stable_environment()
    python = "/usr/bin/python3"
    probe = subprocess.run([python, "-m", "pip", "--version"], env=env, stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, text=True, check=False)
    write(OUTPUT / "python-pip-before.json", {"command": [python, "-m", "pip", "--version"],
                                               "returncode": probe.returncode, "output": probe.stdout})
    if probe.returncode:
        verify_host_and_source()
        logged(["sudo", "-n", "apt-get", "update"], "setup-python-pip-apt-update.log", env=env)
        logged(["sudo", "-n", "apt-get", "install", "-y", "--no-install-recommends", "python3-pip"],
               "setup-python-pip-install.log", env=env)
    pip_version = subprocess.check_output([python, "-m", "pip", "--version"], env=env, text=True).strip()
    write(OUTPUT / "python-pip-after.json", {"command": [python, "-m", "pip", "--version"], "output": pip_version})
    logged([python, "-m", "pip", "install", "--disable-pip-version-check", "--requirement", str(SOURCE / "scripts/ci/requirements.txt"),
            "--target", ENVIRONMENT_PATHS["python_deps_dir"], "--report", str(OUTPUT / "python-dependencies.json")], "setup-python.log", env=env)
    logged([python, "-m", "pip", "install", "--disable-pip-version-check", "onnxruntime==1.24.2", "--target", str(STATE / "onnx"),
            "--report", str(OUTPUT / "ort-dependencies.json")], "setup-ort.log", env=env)
    env.update(PATH=ENVIRONMENT_PATHS["cargo_bin_dir"] + ":" + BASE_PATH,
               RUSTUP_TOOLCHAIN="1.97.0", CARGO_NET_OFFLINE="false")
    logged(["cargo", "fetch", "--locked"], "setup-cargo-fetch.log", env=env)
    versions = {}
    for executable, args in {"python3": ["--version"], "node": ["--version"], "git": ["--version"],
                             "gh": ["--version"], "cargo": ["--version"], "rustc": ["--version"],
                             "openssl": ["version"]}.items():
        path = shutil.which(executable, path=env["PATH"])
        require(path is not None, f"required tool absent from stable PATH: {executable}")
        versions[executable] = {"path": path, "version": subprocess.check_output([path, *args], env=env, text=True).strip()}
    require(versions["rustc"]["version"].startswith("rustc 1.97.0 "), "wrong Rust version")
    library = Path(ENVIRONMENT_PATHS["ort_lib_dir"]) / "libonnxruntime.so.1.24.2"
    require(library.is_file(), "pinned ONNX Runtime library missing")
    links = {}
    for name in ("libonnxruntime.so", "libonnxruntime.so.1"):
        link = library.with_name(name)
        require(not link.exists() and not link.is_symlink(), "unexpected pre-existing ONNX library link")
        link.symlink_to(library.name)
        require(link.resolve() == library.resolve(), "ONNX library link resolution differs")
        links[name] = os.readlink(link)
    # The module repeats this full identity check independently before host mutation.
    verify_host_and_source()
    current_identity()
    from ort_system import prepare_system_ort
    system_ort = prepare_system_ort()
    host = read(OUTPUT / "host-context.json")
    host.update(system_ort=system_ort, tool_versions=versions, ort_library={"path": str(library), "sha256": sha(library.read_bytes()), "version": "1.24.2", "links": links})
    write(OUTPUT / "host-context.json", host)
    resource_check("after-dependencies")
    write(OUTPUT / "exact-source-after-dependencies.json", current_identity())


def collect():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    diagnostics = {"recorded_at": utc(), "scope": "original files only; no missing output is fabricated"}
    try:
        diagnostics["post_identity"] = current_identity()
    except Exception as error:
        diagnostics["identity_error"] = repr(error)
    diagnostics["disk"] = {"free_bytes": shutil.disk_usage(OUTPUT).free}
    write(OUTPUT / "collection-diagnostics.json", diagnostics)
    files = []
    for path in sorted(OUTPUT.rglob("*")):
        require(not path.is_symlink(), "artifact output symlink rejected")
        if path.is_file() and path.name != "artifact-index.json":
            data = path.read_bytes()
            files.append({"path": path.relative_to(OUTPUT).as_posix(), "bytes": len(data), "sha256": sha(data)})
    write(OUTPUT / "artifact-index.json", {"schema_version": 1, "candidate_sha": Q, "files": files})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("phase", choices=["initialize", "dependencies", "collect"])
    args = parser.parse_args()
    {"initialize": initialize, "dependencies": dependencies, "collect": collect}[args.phase]()


if __name__ == "__main__":
    main()
