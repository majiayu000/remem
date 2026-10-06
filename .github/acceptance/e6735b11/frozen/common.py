"""Local evidence preparation only; no GitHub writes or receipt rewriting."""
from __future__ import annotations

import hashlib
import json
import subprocess
from pathlib import Path, PurePosixPath

TARGETS = {
    "x86_64-apple-darwin": ("macos", "x86_64"),
    "aarch64-apple-darwin": ("macos", "aarch64"),
    "x86_64-unknown-linux-gnu": ("linux", "x86_64"),
    "aarch64-unknown-linux-gnu": ("linux", "aarch64"),
}
PAYLOAD_LABELS = {"answer", "diagnosis", "reader_input", "remem_db_snapshot", "retrieved_evidence", "score"}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def blob_sha(data):
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def read_json(path):
    return json.loads(Path(path).read_bytes())


def dump(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")
    temporary.replace(path)


def candidate_parents(c):
    parents = c.get("candidate_parents", [c["producer_sha"]])
    require(isinstance(parents, list) and bool(parents), "candidate parents must be a nonempty ordered list")
    require(all(isinstance(parent, str) and len(parent) == 40 and
                all(character in "0123456789abcdef" for character in parent)
                for parent in parents), "candidate parents must be full lowercase Git SHAs")
    require(len(set(parents)) == len(parents), "candidate parents must be unique")
    require(parents[0] == c["producer_sha"], "first candidate parent must be the native producer")
    return list(parents)


def config(path):
    c = read_json(path)
    require(c["schema_version"] == 1, "unsupported configuration schema")
    for key in ("producer_sha", "producer_tree", "source_checkout_sha", "expected_review_head", "stacked_base_sha"):
        require(len(c[key]) == 40 and all(x in "0123456789abcdef" for x in c[key]), key)
    for key in ("production_input_tree_sha256", "production_pathspec_sha256", "suite_sha256"):
        require(len(c[key]) == 64 and all(x in "0123456789abcdef" for x in c[key]), key)
    require(c["workflow_run_attempt"] > 0, "workflow attempt must be explicit")
    candidate_parents(c)
    return c


def git(c, *args):
    return subprocess.check_output(["git", "--no-optional-locks", *args], cwd=c["repo_dir"])


def regular_file(root, relative):
    """Reject archive/file paths escaping the explicitly selected local root."""
    relative = PurePosixPath(relative)
    require(not relative.is_absolute() and ".." not in relative.parts, f"unsafe relative path: {relative}")
    path = Path(root) / str(relative)
    require(path.is_file() and not path.is_symlink(), f"not a regular file: {path}")
    require(Path(root).resolve() in path.resolve().parents, f"escaped root: {path}")
    return path


def manifest_destination(target):
    suffix = "" if target == "aarch64-apple-darwin" else "-" + target
    return f"eval/public/memory/manifests/adversarial-policy-v2{suffix}.json"


def active_manifests(c):
    expected = {manifest_destination(t) for t in TARGETS}
    repo = Path(c["repo_dir"])
    actual = {p.relative_to(repo).as_posix() for p in (repo / "eval/public/memory/manifests").glob("adversarial-policy-v2*.json")}
    indexed = {p for p in git(c, "ls-files", "--", "eval/public/memory/manifests/adversarial-policy-v2*.json").decode().splitlines()}
    require(actual == expected, f"active worktree manifests differ: {actual ^ expected}")
    require(indexed == expected, f"active index manifests differ: {indexed ^ expected}")
    return sorted(actual)


def source_identity(c, expected_head=None, clean=False):
    repo = Path(c["repo_dir"])
    head = git(c, "rev-parse", "HEAD").decode().strip()
    if expected_head:
        require(head == expected_head, f"wrong checkout head: {head}")
    if clean:
        require(not git(c, "status", "--porcelain", "--untracked-files=all"), "checkout must be clean, including untracked files")
    contract = read_json(repo / "eval/production-input-pathspec-v1.json")
    require(not git(c, "diff", "--name-only", "--", *contract["paths"]), "unstaged production inputs")
    require(not git(c, "diff", "--cached", "--name-only", "--", *contract["paths"]), "staged production inputs")
    production = sha(git(c, "ls-files", "-s", "--", *contract["paths"]))
    pathspec = sha(json.dumps(contract, sort_keys=True, separators=(",", ":")).encode())
    suite = sha((repo / "eval/public/memory/suites/adversarial-policy/suite.json").read_bytes())
    require(production == c["production_input_tree_sha256"], "production input identity differs")
    require(pathspec == c["production_pathspec_sha256"], "pathspec identity differs")
    require(suite == c["suite_sha256"], "suite bytes differ")
    return {"sha": head, "tree": git(c, "rev-parse", "HEAD^{tree}").decode().strip(),
            "parents": git(c, "show", "-s", "--format=%P", "HEAD").decode().split(),
            "production_input_tree_sha256": production, "production_pathspec_sha256": pathspec,
            "suite_sha256": suite}
