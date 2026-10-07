#!/usr/bin/env python3
"""Provision only the pinned ORT loader dependency on an authenticated CI VM."""
from __future__ import annotations

import csv
import io
import os
from pathlib import Path
import re
import subprocess

import bootstrap as context

VERSION = "1.24.2"
FILENAME = "libonnxruntime.so.1.24.2"
PINNED_BYTES = 22069152
PINNED_SHA256 = "7d5242d7bfbb8b0a2c6ae4d4883adac34c883129c8091f74f3e1bc9f740b1d62"
HOST_ROOT = Path("/usr/local/lib")
SOURCE_ROOT = context.STATE / "onnx"
SOURCE_LIBRARY = SOURCE_ROOT / "onnxruntime/capi" / FILENAME
OUTPUT = context.OUTPUT
LINK_NAMES = ("libonnxruntime.so", "libonnxruntime.so.1")
PROBE_SOURCE = r'''#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
extern const void *OrtGetApiBase(void);
int main(void) {
    Dl_info info = {0};
    void *symbol = dlsym(RTLD_DEFAULT, "OrtGetApiBase");
    if (!OrtGetApiBase() || !symbol || !dladdr(symbol, &info) || !info.dli_fname)
        return 2;
    printf("loaded=%s\n", info.dli_fname);
    return 0;
}
'''


def guard():
    # This independent module checks the host before calling the caller's full
    # Q/R/ref/workspace/clean-tree/body guard and before any subprocess mutation.
    context.require(os.environ.get("GITHUB_ACTIONS") == "true"
                    and os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted"
                    and os.environ.get("GITHUB_SERVER_URL") == "https://github.com",
                    "system ORT preparation is forbidden outside GitHub-hosted Actions")
    context.verify_host_and_source()
    identity = context.current_identity()
    context.require(identity["sha"] == context.Q and identity["tree"] == context.TREE
                    and identity["parents"] == context.PARENTS, "wrong candidate identity for system ORT")
    return identity


def validate_library(path):
    context.require(path.is_file() and not path.is_symlink() and path.resolve() == path,
                    "ORT library must be a canonical regular file")
    data = path.read_bytes()
    context.require(len(data) == PINNED_BYTES and context.sha(data) == PINNED_SHA256,
                    "ORT library differs from the explicitly pinned original 1.24.2 ELF")


def validate_package():
    validate_library(SOURCE_LIBRARY)
    report = context.read(OUTPUT / "ort-dependencies.json")
    packages = [item for item in report["install"]
                if item["metadata"]["name"].lower().replace("_", "-") == "onnxruntime"]
    context.require(len(packages) == 1 and packages[0]["metadata"]["version"] == VERSION,
                    "pip report did not install exactly ONNX Runtime 1.24.2")
    wheel_sha = packages[0]["download_info"]["archive_info"]["hashes"]["sha256"]
    context.require(re.fullmatch(r"[0-9a-f]{64}", wheel_sha) is not None, "wheel hash missing from pip report")
    metadata = SOURCE_ROOT / f"onnxruntime-{VERSION}.dist-info"
    fields = (metadata / "METADATA").read_text()
    context.require(re.search(r"^Name: onnxruntime$", fields, re.MULTILINE)
                    and re.search(r"^Version: 1\.24\.2$", fields, re.MULTILINE), "installed package metadata differs")
    rows = list(csv.reader(io.StringIO((metadata / "RECORD").read_text())))
    library_rows = [row for row in rows if row[0] == "onnxruntime/capi/" + FILENAME]
    import base64
    record_hash = base64.urlsafe_b64encode(bytes.fromhex(PINNED_SHA256)).decode().rstrip("=")
    context.require(library_rows == [["onnxruntime/capi/" + FILENAME, "sha256=" + record_hash, str(PINNED_BYTES)]],
                    "wheel RECORD does not bind the pinned library bytes")
    return {"version": VERSION, "wheel_sha256": wheel_sha, "library_sha256": PINNED_SHA256,
            "library_bytes": PINNED_BYTES, "pin_provenance": "Previously read original local 1.24.2 ELF; not a claim of R1 artifact verification"}


def validate_destinations():
    context.require(HOST_ROOT.is_dir() and not HOST_ROOT.is_symlink() and HOST_ROOT.resolve() == HOST_ROOT,
                    "system library directory is not canonical")
    destination = HOST_ROOT / FILENAME
    if destination.exists() or destination.is_symlink():
        validate_library(destination)
    for name in LINK_NAMES:
        path = HOST_ROOT / name
        if path.exists() or path.is_symlink():
            context.require(path.is_symlink() and os.readlink(path) == FILENAME
                            and path.resolve() == destination,
                            "refusing to replace an unrelated system ORT object")


def verify_probe_elf(text):
    context.require(re.search(r"\(NEEDED\).*\[libonnxruntime\.so\.1\]", text) is not None,
                    "probe is not directly linked to libonnxruntime.so.1")
    context.require("(RPATH)" not in text and "(RUNPATH)" not in text,
                    "loader probe must not use an embedded library search path")


def loaded_path(text):
    lines = re.findall(r"^loaded=(.+)$", text, re.MULTILINE)
    context.require(len(lines) == 1, "probe did not identify exactly one loaded ORT library")
    path = Path(lines[0])
    context.require(path.is_absolute() and path.resolve() == HOST_ROOT / FILENAME,
                    "empty-environment probe loaded a different ORT library")
    validate_library(path.resolve())
    return str(path.resolve())


def cache_binding(text):
    paths = re.findall(r"^\s*libonnxruntime\.so\.1\s+\([^\n]+\)\s+=>\s+(\S+)\s*$", text, re.MULTILINE)
    expected = HOST_ROOT / "libonnxruntime.so.1"
    context.require(str(expected) in paths and expected.resolve() == HOST_ROOT / FILENAME,
                    "loader cache does not expose the pinned system ORT link")
    return {"soname": "libonnxruntime.so.1", "path": str(expected), "resolved_path": str(expected.resolve())}


def prepare_system_ort():
    before = guard()
    receipt_path = OUTPUT / "system-ort-preparation.json"
    context.require(not receipt_path.exists(), "prior system ORT evidence must not be overwritten")
    receipt = {"schema_version": 1, "scope": "CI environment preparation only; not a Q acceptance gate",
               "before_identity": before, "commands": [], "passed": False}
    context.write(receipt_path, receipt)

    def run(label, command, *, require_success=True):
        log_path = OUTPUT / ("system-ort-" + label + ".log")
        entry = {"phase": label, "command": command, "log": str(log_path), "started_at": context.utc()}
        receipt["commands"].append(entry)
        context.write(receipt_path, receipt)
        try:
            with log_path.open("x") as log:
                result = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=log,
                                        stderr=subprocess.STDOUT, env=context.stable_environment(),
                                        timeout=120, check=False)
            entry["returncode"] = result.returncode
        except BaseException as error:
            entry["execution_error"] = repr(error)
            raise
        finally:
            entry["ended_at"] = context.utc()
            if log_path.exists():
                entry["log_sha256"] = context.sha(log_path.read_bytes())
            context.write(receipt_path, receipt)
        if require_success:
            context.require(result.returncode == 0, f"system ORT preparation failed: {label}, exit {result.returncode}")
        return log_path.read_text(errors="replace")

    failure = None
    final_identity = None
    try:
        package = validate_package()
        receipt["package"] = package
        validate_destinations()
        source = OUTPUT / "ort-loader-probe.c"
        binary = OUTPUT / "ort-loader-probe"
        context.require(not source.exists() and not binary.exists(), "prior loader probe must not be replaced")
        source.write_text(PROBE_SOURCE)
        run("compile-probe", ["/usr/bin/cc", "-std=c11", "-Wall", "-Wextra", "-Werror", "-O0", "-g0",
                              str(source), "-L" + str(SOURCE_LIBRARY.parent), "-lonnxruntime", "-ldl", "-o", str(binary)])
        verify_probe_elf(run("probe-elf", ["/usr/bin/readelf", "-d", str(binary)]))
        run("cache-before", ["/usr/sbin/ldconfig", "-p"])
        run("empty-environment-before", ["/usr/bin/env", "-i", str(binary)], require_success=False)
        context.require(guard() == before, "source identity changed before system ORT mutation")
        validate_library(SOURCE_LIBRARY)
        validate_destinations()
        destination = HOST_ROOT / FILENAME
        if not destination.exists():
            run("install", ["/usr/bin/sudo", "-n", "install", "-m", "0644", "--", str(SOURCE_LIBRARY), str(destination)])
        validate_library(destination)
        for name in LINK_NAMES:
            path = HOST_ROOT / name
            if not path.is_symlink():
                run("link-" + name, ["/usr/bin/sudo", "-n", "ln", "-s", "--", FILENAME, str(path)])
        validate_destinations()
        run("refresh-cache", ["/usr/bin/sudo", "-n", "ldconfig"])
        cached = cache_binding(run("cache-after", ["/usr/sbin/ldconfig", "-p"]))
        loaded = loaded_path(run("empty-environment-after", ["/usr/bin/env", "-i", str(binary)]))
        final_identity = {"version": VERSION, "path": str(destination), "sha256": PINNED_SHA256,
                          "bytes": PINNED_BYTES, "links": {name: os.readlink(HOST_ROOT / name) for name in LINK_NAMES},
                          "loader_cache": cached, "empty_environment_loaded_path": loaded,
                          "probe_requires_soname": "libonnxruntime.so.1", "probe_has_rpath_or_runpath": False,
                          "package": package}
        receipt["installed_identity"] = final_identity
    except BaseException as error:
        failure = error
        receipt["error"] = repr(error)
    try:
        receipt["after_identity"] = guard()
        context.require(receipt["after_identity"] == before, "source identity changed during system ORT preparation")
    except BaseException as error:
        receipt["post_identity_error"] = repr(error)
        failure = failure or error
    receipt["passed"] = failure is None
    context.write(receipt_path, receipt)
    if failure is not None:
        raise failure
    return final_identity
