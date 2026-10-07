#!/usr/bin/env python3
"""Authenticate original R3 evidence and correct only its hostname identity test.

No candidate gate, build, source edit, or original receipt edit is performed.
"""
from __future__ import annotations
import datetime as dt
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
import zipfile

HERE = Path(__file__).resolve().parent
WORKSPACE = Path("/home/runner/work/remem/remem")
ORIGINAL = WORKSPACE / "orchestration"
ORIGINAL_SUPPORT = ORIGINAL / ".github/acceptance/4ec561b3"
OUTPUT = WORKSPACE / "host-identity-recheck-results"
MAX_DOWNLOAD = 20 * 1024 * 1024


class AuditError(ValueError):
    pass


def require(ok, message):
    if not ok:
        raise AuditError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read(path):
    return json.loads(Path(path).read_bytes())


def save(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2, sort_keys=True)
        handle.write("\n")


def git(path, *args):
    return subprocess.check_output(["git", "--no-optional-locks", *args], cwd=path, text=True).strip()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def http_bytes(api_path, token, opener=None):
    """Bearer is sent only to the initial api.github.com request.

    Blob redirects are manually followed with a new credential-free request.
    Neither signed locations nor the token are returned or written to evidence.
    """
    require(api_path.startswith("/repos/majiayu000/remem/") and "?" not in api_path.split("/repos/", 1)[0], "API path is outside the fixed repository")
    opener = opener or urllib.request.build_opener(NoRedirect())
    url = "https://api.github.com" + api_path
    for redirect in range(3):
        parts = urllib.parse.urlsplit(url)
        require(parts.scheme == "https" and parts.port in (None, 443) and parts.username is None and parts.password is None,
                "download endpoint must be HTTPS without user information")
        if redirect == 0:
            require(parts.hostname == "api.github.com", "unexpected API host")
            headers = {"Authorization": "Bearer " + token, "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"}
        else:
            require(parts.hostname is not None and parts.hostname.endswith(".blob.core.windows.net"), "unexpected signed-download host")
            headers = {}
        request = urllib.request.Request(url, headers=headers)
        try:
            response = opener.open(request, timeout=60)
        except urllib.error.HTTPError as error:
            response = error
        try:
            code = response.getcode()
            if code in (301, 302, 303, 307, 308):
                location = response.headers.get("Location")
                require(bool(location), "redirect has no location")
                url = urllib.parse.urljoin(url, location)
                continue
            require(code == 200, "GitHub evidence download failed")
            data = response.read(MAX_DOWNLOAD + 1)
            require(len(data) <= MAX_DOWNLOAD, "download exceeds the fixed evidence size limit")
            return data
        finally:
            response.close()
    raise AuditError("too many evidence redirects")


def validate_api(policy, run, jobs, artifacts):
    require(run["id"] == policy["run_id"] and run["run_attempt"] == policy["attempt"]
            and run["head_sha"] == policy["original_support_sha"] and run["head_branch"] == policy["original_branch"]
            and run["event"] == "push" and run["status"] == "completed" and run["conclusion"] == "failure"
            and run["repository"]["id"] == policy["repository_id"], "original completed failed-run API binding differs")
    require(jobs["total_count"] == len(jobs["jobs"]) == 3, "original job set is incomplete or unexpected")
    by_id = {j["id"]: j for j in jobs["jobs"]}
    require(set(by_id) == {j["id"] for j in policy["jobs"].values()}, "original job IDs differ")
    result = {}
    for group, expected in policy["jobs"].items():
        job = by_id[expected["id"]]
        require(job["name"] == expected["name"] and job["run_id"] == policy["run_id"]
                and job["run_attempt"] == policy["attempt"] and job["head_sha"] == policy["original_support_sha"]
                and job["status"] == "completed" and job["conclusion"] == ("failure" if group == "union" else "success"),
                "original job identity or actual conclusion differs")
        if group != "union":
            require(job["runner_id"] == expected["runner_id"] and job["runner_name"] == expected["runner_name"]
                    and job["runner_group_id"] == 0 and job["runner_group_name"] == "GitHub Actions"
                    and job["labels"] == ["ubuntu-24.04"], "original standard hosted-runner assignment differs")
        result[group] = job
    require(artifacts["total_count"] == len(artifacts["artifacts"]) == 3, "original artifact set is incomplete or unexpected")
    by_id = {a["id"]: a for a in artifacts["artifacts"]}
    require(set(by_id) == {a["id"] for a in policy["artifacts"].values()}, "original artifact IDs differ")
    for expected in policy["artifacts"].values():
        a = by_id[expected["id"]]
        require(a["name"] == expected["name"] and a["size_in_bytes"] == expected["bytes"]
                and a["digest"] == "sha256:" + expected["sha256"] and a["expired"] is False,
                "original artifact name, size, digest, or expiry differs")
        wr = a["workflow_run"]
        require(wr["id"] == policy["run_id"] and wr["head_sha"] == policy["original_support_sha"]
                and wr["head_branch"] == policy["original_branch"] and wr["repository_id"] == policy["repository_id"]
                and wr["head_repository_id"] == policy["repository_id"], "artifact workflow provenance differs")
    return result


def extract_original(archive, target, expected):
    data = archive.read_bytes()
    require(len(data) == expected["bytes"] and sha(data) == expected["sha256"], "original ZIP bytes or digest differ")
    target.mkdir(parents=True, exist_ok=False)
    files, names, total = [], set(), 0
    with zipfile.ZipFile(archive) as z:
        require(len(z.infolist()) <= 500, "unexpected ZIP entry count")
        for entry in z.infolist():
            relative = PurePosixPath(entry.filename)
            mode = entry.external_attr >> 16
            require(relative.parts and not relative.is_absolute() and ".." not in relative.parts
                    and "\\" not in entry.filename and entry.filename not in names
                    and not stat.S_ISLNK(mode), "unsafe or duplicate original ZIP path")
            names.add(entry.filename)
            path = target.joinpath(*relative.parts)
            if entry.is_dir():
                path.mkdir(parents=True, exist_ok=True)
                continue
            require(not stat.S_IFMT(mode) or stat.S_ISREG(mode), "non-regular original ZIP entry")
            total += entry.file_size
            require(total <= MAX_DOWNLOAD, "expanded original ZIP exceeds evidence limit")
            raw = z.read(entry)
            require(len(raw) == entry.file_size, "original ZIP member size differs")
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("xb") as handle:
                handle.write(raw)
            files.append({"path": relative.as_posix(), "bytes": len(raw), "sha256": sha(raw)})
    return files


def authenticated_worker_log(raw, expected):
    text = raw.decode("utf-8", errors="strict")
    had_bom = text.startswith("\ufeff")
    had_final_lf = text.endswith("\n")
    canonical = text.removeprefix("\ufeff").removesuffix("\n").encode("utf-8")
    require(sha(canonical) == expected["log_text_canonical_sha256"], "entire original job-log content differs")
    matches = re.findall(r"(?m)^\d{4}-\d{2}-\d{2}T\S+Z Worker ID: \{([0-9a-f-]{36})\}$", text)
    require(matches == [expected["worker_uuid"]], "original Hosted Compute Agent Worker UUID differs")
    return {"http_raw_bytes": len(raw), "http_raw_sha256": sha(raw), "canonical_text_sha256": sha(canonical),
            "removed_one_initial_utf8_bom": had_bom, "removed_one_terminal_lf": had_final_lf,
            "worker_uuid": matches[0], "representation_rule": "Strict UTF-8; remove at most one initial BOM and one final LF only. No other whitespace or decoding normalization."}


def corrected_comparison(policy, groups, original_union, jobs, logs):
    require(original_union["passed"] is False
            and original_union["errors"] == [{"group": "aggregate", "error": policy["original_unique_error"]}],
            "original union has another or additional failure")
    require(original_union["candidate_sha"] == policy["candidate_sha"] and original_union["candidate_tree"] == policy["candidate_tree"]
            and original_union["candidate_parents"] == policy["candidate_parents"] and original_union["support_sha"] == policy["original_support_sha"]
            and original_union["run_id"] == str(policy["run_id"]) and original_union["run_attempt"] == str(policy["attempt"]),
            "original union identity differs")
    require(set(groups) == {"eval", "other"} and groups == original_union["groups"], "fresh original group verification differs from original union")
    first, second = groups["eval"], groups["other"]
    for key in ("identity", "environment", "execution_helpers"):
        require(first[key] == second[key], "strict cross-runner source/environment/helper binding differs")
    for key in ("tool_versions", "ort_library", "system_ort"):
        require(first["host"][key] == second["host"][key], "strict cross-runner toolchain/library differs")
    required = {"eval_library_and_e2e", "default_check", "public_claims_self_test", "committed_root_verifier",
                "committed_root_eval_gates", "consumer_report_path_regression", "snapshot_mutation_regression", "full_preflight"}
    names = [g["name"] for group in groups.values() for g in group["gates"]]
    require(len(names) == len(set(names)) == 8 and set(names) == required, "complete original eight-gate union differs")
    records = {}
    for name, group in groups.items():
        host, job = group["host"], jobs[name]
        require(host["hostname"] == policy["expected_hostname"] and host["runner_name"] == job["runner_name"]
                and host["runner_environment"] == "github-hosted" and host["runner_image"] == policy["image"]
                and host["runner_image_version"] == policy["image_version"] and host["job"] == "acceptance",
                "original host context is not bound to the expected hosted runner")
        record = authenticated_worker_log(logs[name], policy["jobs"][name])
        start, end = (dt.datetime.fromisoformat(job[k].replace("Z", "+00:00")) for k in ("started_at", "completed_at"))
        captured = dt.datetime.fromisoformat(host["recorded_at"])
        require(start < captured < end, "host recording time is outside the actual job interval")
        records[name] = {**record, "job_id": job["id"], "runner_id": job["runner_id"], "runner_name": job["runner_name"],
                         "runner_group_id": job["runner_group_id"], "runner_group_name": job["runner_group_name"],
                         "labels": job["labels"], "hostname": host["hostname"], "image": host["runner_image"],
                         "image_version": host["runner_image_version"], "started_at": job["started_at"], "completed_at": job["completed_at"]}
    require(records["eval"]["runner_id"] != records["other"]["runner_id"]
            and records["eval"]["runner_name"] != records["other"]["runner_name"]
            and records["eval"]["worker_uuid"] != records["other"]["worker_uuid"], "independent hosted-runner assignments are not demonstrated")
    start = max(dt.datetime.fromisoformat(jobs[g]["started_at"].replace("Z", "+00:00")) for g in groups)
    end = min(dt.datetime.fromisoformat(jobs[g]["completed_at"].replace("Z", "+00:00")) for g in groups)
    require(start < end, "original runner execution intervals do not overlap")
    return {"hosted_runner_instances": records, "overlap_seconds": (end - start).total_seconds(),
            "proof_scope": "Two independent GitHub-hosted standard Ubuntu runner VM instances; not a claim about distinct physical hardware.",
            "all_original_tool_versions_including_gh_equal": True, "hostname_preserved_as_nonunique_inventory": policy["expected_hostname"]}


def verify_execution_context(policy):
    require(os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted"
            and os.environ.get("GITHUB_SERVER_URL") == "https://github.com"
            and os.environ.get("GITHUB_REPOSITORY") == policy["repo"]
            and os.environ.get("GITHUB_EVENT_NAME") == "push" and os.environ.get("GITHUB_REF") == "refs/heads/" + policy["recheck_branch"],
            "unexpected aggregation execution host or trigger")
    require(Path(os.environ["GITHUB_WORKSPACE"]).resolve() == WORKSPACE
            and HERE == WORKSPACE / "recheck/.github/acceptance-recheck/4ec561b3", "unexpected aggregation checkout path")
    current = git(WORKSPACE / "recheck", "rev-parse", "HEAD")
    require(current == os.environ["GITHUB_SHA"] and git(WORKSPACE / "recheck", "show", "-s", "--format=%P", "HEAD") == policy["original_support_sha"],
            "aggregation support must have original R3 as its sole parent")
    changes = git(WORKSPACE / "recheck", "diff", "--name-status", policy["original_support_sha"], "HEAD").splitlines()
    require(changes and all(x.startswith("A\t") and (x.split("\t")[1].startswith(".github/acceptance-recheck/4ec561b3/")
            or x.split("\t")[1] == ".github/workflows/q6-host-identity-recheck.yml") for x in changes), "aggregation support changed an original file")
    require(git(ORIGINAL, "rev-parse", "HEAD") == policy["original_support_sha"]
            and git(ORIGINAL, "rev-parse", "HEAD^{tree}") == policy["original_support_tree"], "original R3 checkout differs")
    for item in policy["original_support_files"]:
        raw = (ORIGINAL / item["path"]).read_bytes()
        require(len(raw) == item["bytes"] and sha(raw) == item["sha256"], "original R3 support file changed")
    return current


def main():
    OUTPUT.mkdir(parents=True, exist_ok=False)
    policy = read(HERE / "policy.json")
    verdict = {"policy_version": policy["policy_version"], "policy_sha256": sha((HERE / "policy.json").read_bytes()),
               "wrapper_sha256": sha(Path(__file__).read_bytes()), "candidate_sha": policy["candidate_sha"],
               "original_support_sha": policy["original_support_sha"], "original_run_id": policy["run_id"], "original_attempt": policy["attempt"],
               "original_run_conclusion": "failure", "original_union_passed": False, "corrected_union_passed": False,
               "scope": "Separate authenticated aggregation recheck of original remote Q6 gate evidence; no gate was rerun and no original failure was rewritten."}
    phase = "execution-context"
    try:
        verdict["recheck_support_sha"] = verify_execution_context(policy)
        verdict["recheck_run_id"] = os.environ["GITHUB_RUN_ID"]
        verdict["recheck_attempt"] = os.environ["GITHUB_RUN_ATTEMPT"]
        token = os.environ["GITHUB_TOKEN"]
        require(bool(token), "read-only Actions token is required")
        root = "/repos/" + policy["repo"]
        api = {}
        phase = "original-api-authentication"
        for name, suffix in {"run": f'/actions/runs/{policy["run_id"]}', "jobs": f'/actions/runs/{policy["run_id"]}/jobs?per_page=100',
                             "artifacts": f'/actions/runs/{policy["run_id"]}/artifacts?per_page=100'}.items():
            raw = http_bytes(root + suffix, token)
            (OUTPUT / ("original-api-" + name + ".json")).write_bytes(raw)
            api[name] = json.loads(raw)
        jobs = validate_api(policy, api["run"], api["jobs"], api["artifacts"])
        phase = "original-artifact-authentication"
        downloads = {}
        for group, expected in policy["artifacts"].items():
            raw = http_bytes(root + f'/actions/artifacts/{expected["id"]}/zip', token)
            archive = OUTPUT / (group + ".zip")
            archive.write_bytes(raw)
            files = extract_original(archive, OUTPUT / group, expected)
            downloads[group] = {**expected, "downloaded_sha256": sha(raw), "downloaded_bytes": len(raw), "files": files}
        save(OUTPUT / "authenticated-downloads.json", downloads)
        logs = {}
        phase = "original-job-log-authentication"
        for group in ("eval", "other"):
            raw = http_bytes(root + f'/actions/jobs/{policy["jobs"][group]["id"]}/logs', token)
            (OUTPUT / ("original-job-" + group + ".log")).write_bytes(raw)
            logs[group] = raw
            save(OUTPUT / ("job-log-authentication-" + group + ".json"), authenticated_worker_log(raw, policy["jobs"][group]))
        union_file = OUTPUT / "union/union-verdict.json"
        require(sha(union_file.read_bytes()) == policy["original_union_sha256"], "original failed union bytes differ")
        original_union = read(union_file)
        phase = "unchanged-original-group-verification"
        sys.path.insert(0, str(ORIGINAL_SUPPORT))
        import verify_results as original
        groups = {g: original.verify_group(OUTPUT / g, g, str(policy["run_id"]), str(policy["attempt"]), policy["original_support_sha"]) for g in ("eval", "other")}
        save(OUTPUT / "fresh-original-group-verification.json", groups)
        groups = read(OUTPUT / "fresh-original-group-verification.json")
        phase = "strict-corrected-hosted-runner-comparison"
        verdict["host_identity"] = corrected_comparison(policy, groups, original_union, jobs, logs)
        verdict["groups"] = groups
        verdict["original_union_sha256"] = sha(union_file.read_bytes())
        verdict["original_errors"] = original_union["errors"]
        verdict["corrected_union_passed"] = True
    except Exception as error:
        # HTTP exception strings may contain signed URLs. Never serialize them.
        verdict["error"] = {"phase": phase, "type": type(error).__name__,
                            "message": str(error) if isinstance(error, AuditError) else "Evidence recheck failed; inspect preserved stage files without exposing credentials or signed locations."}
    save(OUTPUT / "corrected-union-verdict.json", verdict)
    entries = [{"path": p.relative_to(OUTPUT).as_posix(), "bytes": p.stat().st_size, "sha256": sha(p.read_bytes())}
               for p in sorted(OUTPUT.rglob("*")) if p.is_file()]
    save(OUTPUT / "recheck-artifact-index.json", {"files": entries, "candidate_sha": policy["candidate_sha"], "original_run_id": policy["run_id"]})
    print(json.dumps({"corrected_union_passed": verdict["corrected_union_passed"], "original_run_conclusion": "failure", "phase": phase}))
    return 0 if verdict["corrected_union_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
