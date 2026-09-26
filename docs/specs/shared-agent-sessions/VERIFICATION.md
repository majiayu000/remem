# Migration verification

## Current closure status (2026-09-26)

The full local preflight at `2aac8286` passed all 29 checks, including the
production Cargo suite (4,016 passed, zero failed, one ignored). Exact-head
CI run [36253853909](https://github.com/majiayu000/remem/actions/runs/36253853909)
then found two eval-only stale fingerprint checks: the graph-decision SQL
bundle omitted registered migration v093, and its checked-in report still
bound older implementation inputs. Source producer `f97e85e3` adds the v093
SQL and schema invariant to the fingerprint inputs and regenerates the report.
All six focused graph-decision fingerprint tests, formatting and all-target
Clippy passed. Full final preflight, eval tests and exact-head CI remain gates.

Native Actions run [36260044237](https://github.com/majiayu000/remem/actions/runs/36260044237)
passed four native targets and its aggregate. Its clean producer
`f97e85e30619e1be34d980fe4d019415bc7f65fd` binds production-input tree
`21e90c35def22f407557ba904809d58e7e97b2ae06e6706f21aeef301ce40b1e`.
Every target reports 20 recomputed cases and zero policy failures. Downloaded
receipts and all 480 payload hashes were checked, and both the raw four-target
root and the canonically relocated root passed independent verification with
all four targets current and release.ready true. Only manifest, report and run
path references changed during relocation; other bytes match the CI bundles.
These technical evidence results do not constitute a merge or publication.
The sections below retain earlier validation history; their producer commits
and test totals do not certify the current head.

The deliberately broken v071 fixture omits v093 together with its missing
v071 dependency. Other fixture corrections open the isolated encrypted CLI
database with its generated temporary key, and use an octal Unix permission
literal. The temporary key has
a distinct binding so assertion diagnostics continue to print the host override
name. No runtime behavior or real memory data changes in these corrections.
The root-test sandbox also uses an atomic sequence: timestamp-only names can
collide between parallel tests, allowing one fixture's cleanup to delete another.

The reported SessionStart Cargo exit 101 in the old CI log was the expected
mocked failure inside a passing runner unit test. The actual CI failure was
Clippy rejecting the non-octal permission literal.

Source producer `f789fee9a9dfc732dcdc78774edbeff74ab282be` passed 160 migration
checks and all four parallel CLI root checks, plus formatting, locked Cargo
check, migration-concern and version-sync guards. The current debug executable
passed isolated install dry-run/install/status/context; doctor reported only the
expected missing capture heartbeat on a fresh store. SessionStart smoke emitted
225 bytes on the first invocation and zero on its duplicate.

Native Actions run [36246237376](https://github.com/majiayu000/remem/actions/runs/36246237376)
passed all four targets and the aggregate. Every target binds that clean source
producer and production-input tree
`34175f433c0180c3f7bc959c36de4e3e18b40cc77e483bc1e33ec363754139a0`,
with 20 recomputed cases and zero policy failures. Import checked each receipt,
report digest and all 120 payload files per target; canonical path relocation
preserved payload bytes. Independent local verification of the staged complete
matrix passed with all four current targets and no missing/stale targets.

The evidence/docs-only import retains that production-input tree. Final full
preflight, production/eval tests and exact-head CI are separate merge gates;
use the completed checks on [PR #1091](https://github.com/majiayu000/remem/pull/1091)
and the integration closure record for their outcomes. Earlier counts below
remain historical and must not be substituted for those final gates.

## Private snapshot parity

2026-09-25, original projector from remem 7f4e144f versus shared-library adapter
(core a88ed07). Deterministically selected every eighth sorted Codex JSONL file
and all Claude JSONL files from the pre-existing frozen local snapshot.

- 952 files, 290,572 physical records.
- 33,898 projected user/assistant messages.
- Zero malformed JSON records; zero role/text/timestamp differences.
- The frozen private snapshot, detailed usage reports and associated caches were
  removed after the cross-repository comparisons; original user data was unchanged.
- Temporary comparison harness removed after execution. No transcript contents
  or original paths were added to Git. Comparison ran read-only, without a DB.
- This is sampled projection parity, not a claim about every Codex file or
  new source-classification labels. Synthetic tests separately cover captured
  byte boundaries, ordinal/empty-row preservation, UTF-8 errors and discovery.

## Expected classification changes

Native `source` now participates in Codex mode projection. `exec` plus Desktop
maps to the existing unattended label; native subagent evidence wins over
originator, and IDE maps to interactive. This label does not establish whether
a human initiated a run. First session ID/cwd/branch and raw identity stay local.

## Delivery gates

Runtime verification completed with the full-run outcomes and final targeted
rechecks distinguished below. Focused tests passed: 8 raw projection/
framing, 42 ingestion/identity, 23 Git evidence, 28 raw archive, 25 reconciliation,
and 2 isolated CLI root tests (128 total). The CLI tests check both valid native
root overrides, both empty override errors, and no database creation on invalid
configuration.

The consumer compiled against registry agent-sessions 0.2.0 with checksum
`741368addca6a3758a911dc5b0871df029863c67d2d2008788662586ed4748c3`.
No agent-sessions path patch remains. Registry resolution is verified locally;
remote CI, merge and the Remem release are separate gates.

## Validation environment

The repository public-surface guard requires Rust 1.97.0; it was installed and
the prior unset rustup default was retained. The historical security benchmark
producer commit `7b3c13c7795eafc4ebc55402848552986ed13ca9` was fetched to resolve
its source-tree attestation. An ignored target/doc link exposes the shared Cargo
rustdoc output to the existing guard. The unchanged REST declaration scan took
about nine minutes on this machine.

The spec-only commit dbf45024 full local preflight passed every gate except the
production test run (4000 passed, 2 failed, 1 ignored). The runner had imposed one
shared REMEM_CONFIG across per-test data directories, allowing an enabled rule
setting to reach two disabled-config cases. A controlled reproduction failed
with that shared setting and passed both cases when REMEM_CONFIG was unset.
The runner now clears inherited REMEM settings and isolates HOME and data while
letting each fixture own its configuration. The original full exit 1 is retained
as evidence; it is not reported as a green run. Fresh remote CI verifies the
spec branch; the complete local suite will run on the implementation branch.

## Complete production test run

At implementation commit b6fe0016, the corrected isolated production test run
completed with 4,078 passed and 6 ignored across 17 unfiltered result blocks;
the library block was 4,008 passed, zero failed, one ignored. Both previously
polluted disabled-config cases passed in the complete run. Formatting, locked
registry check, Clippy, version/documentation guards, native API/SessionStart
smokes and extraction gates passed. The full preflight exit remained 1 because
production_security_e2e correctly rejected the old source-tree-bound report;
all 114 numeric evaluation metrics passed. That source evidence is refreshed
below, with its affected gate recheck tracked separately.

## Four-platform native security evidence

GitHub Actions run 36170892961 generated and verified 20 production-path
adversarial-policy v2 cases on each native target: macOS arm64/x86_64 and Linux
arm64/x86_64. Each row binds clean producer commit
`b6fe001630c963663c252d265cd39cb6bba8d432` and production-input tree
`8e5105e845f8a11bf1c1d259f8a37951b8afa9af436bc43e3aa62610b5f239f1`.
The aggregate and independent local verification passed: 8 manifests/reports,
105 run artifacts and 605 artifact files; all four native targets current,
none missing/stale, zero policy failures.

Import retains the established macOS-arm and Linux-x86 report/artifact names
(`adversarial-policy-v2` and `adversarial-policy-v2-linux-x86_64`) because public
CLI defaults and existing mutation tests bind those locations. The macOS-x86
and Linux-arm rows retain their target-triple names. Only report/run/manifest
path references were relocated; every snapshot, answer, input, evidence,
score and diagnosis payload remained byte-identical to its CI artifact and
was checked against its declared SHA-256. The original bundles and verifier
receipts remain in the external delivery artifacts. No platform identity,
producing commit, source-tree binding or benchmark threshold was rewritten.

The imported complete set was independently verified before copying into the
repository. The evidence paths are outside the production-input pathspec, so
this evidence-only commit preserves the tested implementation tree. The local
standalone macOS run also passed 20 cases, but the committed matrix uses the
four CI-produced rows.

## Final producer after fixture-test alignment

The complete matrix required updating two inventory assertions from the old
6-report/65-run fixture to 8 reports/105 runs/605 artifact files. The existing
closed-target test now explicitly removes one platform manifest before asserting
that release compatibility is unavailable. These three affected tests passed.
Only test source changed in commit
`3523c3b26a123abda5ae43de6da5ca4433548526`; the already-tested production
implementation is unchanged. Because the production-input contract includes
all src bytes, the native matrix was regenerated instead of reusing the earlier
source-tree claim.

GitHub Actions run 36176826911 passed all four native jobs and the aggregate.
The final producer is `3523c3b26a123abda5ae43de6da5ca4433548526`, with tree
`de3ad30bce8295089fce764ddbe493ac1ed43e1f75739ce6900a165f5d6698fa`.
Every row has 20 recomputed runs, zero policy failures and a clean-source
attestation. The same canonical path mapping and payload-byte/hash checks were
applied. Independent local verification of the final imported set passed:
8 manifests, 8 reports, 105 runs, 605 artifact files, four current native targets,
no missing or stale target. Original earlier and final CI bundles remain
separate external audit artifacts. Later fixture corrections supersede this
source-tree certification and require fresh native evidence.

## Final affected-gate rechecks

After evidence commit b214c87d, the final eval-gates run exited 0 with all 114
metrics checked and no regressions. Its local technical ship matrix reported
command_passed, merge_ready and release_ready true. Default-on, cross-host,
coding-outcome and public-superiority claim gates remain false; these results
make no such claims and do not mean a merge or release occurred.

The public-claims checker passed against the final independently verified native
verdict. The three affected fixture tests were repeated against the final
committed evidence: fixture inventory (60.65 s), baseline summary (60.21 s),
and missing-platform negative case (45.47 s), all passed. The earlier full
production suite was not repeated for fixture-only source/evidence updates;
its 4,078 passes remain implementation evidence, with fresh remote CI covering
the final PR head. The original full-preflight exit 1 is preserved in the local
log; its sole stale-security-evidence failure is resolved by these explicit
rechecks, rather than relabeled as an originally green full run.

The integration owner will merge only after the current closure gates pass;
authorization has already been given. Publication is a separate observed action.
