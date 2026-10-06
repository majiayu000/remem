# Exact Q4 acceptance on isolated GitHub Actions hosts

This support branch runs the already reviewed acceptance commands on immutable
candidate `e6735b1165bd92e47333ad7b322640791ecbe689` (Q4). It is orchestration
only and is not a replacement source tree or a new native evidence producer.
It must not be merged into the review PR.

The support commit R has Q4 as its sole parent. Only a push to
`audit/acceptance-runner-20261007-e6735b11` starts this workflow. The branch is
first created at Q4 and then advanced to R. It is not opened as a PR. Workflow
permissions are `contents: read`.

## Exact bindings

Q4 tree: `9ab4b0e7b337d5cf088a3a2a633ae51cf8b62357`.
Ordered Q4 parents:

1. Native producer P5: `c144b36a39415a28cd761d48b5ff78c88a6dfb10`.
2. Concurrent evidence history: `70b2de0d414a1adeaf2623e00843bdf1ee6d35dd`.

The intended PR body remains SHA256
`4ab1ccdc4ada142ca0a480e7450b26f99ee4ed4ec21e9a528fc212c3f5665c73`.
The original config, two intended bodies, publication records, and three
execution helpers are copied byte for byte. `input-hashes.json` identifies each
original file. The P5 native run and its authenticated evidence remain the
authority for native production. No SQLite payload or ZIP is copied into R.

## Two independent execution hosts

Both `ubuntu-24.04` matrix jobs check out R under `orchestration/` and the exact
Q4 SHA under `candidate/`. Only Q4 is used as a Cargo or gate working directory.
The full history is fetched for the original base-S checks. Each job has its
own physical machine, Cargo target and temporary directory. The logical paths
are identical so the two original ledgers can be compared exactly.

The original configuration is adapted only for these absolute host paths:
`repo_dir`, `output_dir`, `pr_body_file`, `epic_body_file`, `cargo_bin_dir`,
`python_deps_dir`, `cargo_target_dir`, and `ort_lib_dir`. Every other value must
remain unchanged. The original and adapted bytes and the full allowlisted diff
are preserved in each artifact. Source, tree, ordered parents, producer, native
run, production fingerprint, suite, base, body bytes and test thread count stay
fixed. Both hosts use an explicit common PATH; actual tool locations and
versions are recorded and compared.

Preparation installs Rust 1.97.0 with Clippy/rustfmt, the Q4 Python requirements,
and ONNX Runtime 1.24.2. The private ONNX directory has the same `.so`/`.so.1`
links as the reviewed local environment. `cargo fetch --locked` runs before
the original helpers set `CARGO_NET_OFFLINE=true`. No shared build cache is
restored. Available space is measured after dependencies and before every
gate batch; at least 20 GiB must remain on the source, target and temporary
filesystems. Only if capacity is insufficient, an authenticated GitHub-hosted
VM may remove the unused preinstalled `/usr/local/lib/android` directory.
The exact Q/R identities and workspace are rechecked first; no other cleanup
path is accepted. The insufficient before measurement, exact command/exit and
after measurement are preserved. Capacity must still reach 20 GiB before the
batch starts. This cleanup cannot run in the shared local container or a
self-hosted runner. `/usr/bin/python3 -m pip` is probed explicitly; if missing,
`python3-pip` is installed only on the same authenticated disposable CI VM.

## Complete acceptance coverage

| Host group | Original gate names |
| --- | --- |
| `eval` | `eval_library_and_e2e` |
| `other` | `default_check`, `public_claims_self_test`, `committed_root_verifier`, `committed_root_eval_gates`, `consumer_report_path_regression`, `snapshot_mutation_regression`, `full_preflight` |

The other group's first six gates use the original `--continue-after-failure`
behavior. The final full preflight runs after a fresh capacity check even if
an earlier gate failed. Every original exit status is preserved and any
failure keeps the group unsuccessful. The separately available
`production_integration_doc` gate is not duplicated: the unmodified full
preflight executes the complete production, integration and doc suite with
`--no-default-features --features local-onnx -- --test-threads 4`.

No adapter modifies the original helpers, gate commands or committed
preflight. There is no `--fast`, body-check skip or test reduction. The original
preflight retains its own TemporaryDirectory cleanup. Its complete stdout is
kept, together with all output files that the original commands persist; no
deleted temporary JSON is reconstructed or claimed as retained.

## Failure and evidence handling

Each execution job has a 360-minute limit. The gate step is bounded at
330 minutes to leave time for failure collection and upload. Collection and
artifact upload run with `always()`. Logs, reports, original inputs, config
diffs, pre/post exact identities, helper freezes, environment layout, actual
host provenance and resource measurements are uploaded. Every retained file
has its original bytes covered by an artifact index. Interrupted work remains
failed or incomplete rather than being recorded as passed.

The aggregate job downloads both artifacts from the same run attempt. It
requires all eight gates exactly once, every command and exit status, original
file/log/report hashes, identical candidate/body/config/helper/environment
bindings, and distinct host identities. It checks actual nonzero test activity
for the two exact regressions, eval library/e2e, and the complete production
suite. Missing, duplicated, skipped, failed or empty execution cannot produce
a successful verdict.

The resulting evidence is **remote actual-Q acceptance on GitHub Actions**.
It must not be reported as successful execution in the shared local container.
The current files make no advance claim that Q4's gates or final PR CI pass.
