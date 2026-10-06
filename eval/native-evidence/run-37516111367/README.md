# Native adversarial-policy v2 evidence import

These files preserve the original verifier receipts and archive identities from
[workflow run 37516111367](https://github.com/majiayu000/remem/actions/runs/37516111367).
The four native target bundles were produced by clean source commit
`2c153b704597ee09eef9606f01167d25521174a6`. Each target contains 20 generated runs
with actual SQLite snapshots and task-bound worker dispatch state.

`import.json` records the authenticated GitHub artifact identities, archive
SHA-256 values, unchanged receipt digests, and manifest source-to-destination
mapping. Public run/report/payload bytes are copied without modification.
The macOS ARM64 target manifest is copied byte-for-byte to the required generic
`eval/public/memory/manifests/adversarial-policy-v2.json` template path. The old
Linux x86_64 alias is removed; only four v2 target manifests remain active.
Unreferenced older payloads retain their historical identities.

Original row receipts describe the producer's actual paths and are never edited
to claim they consumed the renamed generic path. A verifier run on the final
review commit records its own consumed paths and source-equivalence result.
Native security evidence does not establish live-model or coding-outcome
improvements. GH931 and the other research/rollout acceptance gates remain
independent; this import authorizes no release or default-policy change.

The unchanged `authority-verdict.json` is the original aggregate result from
that same native run: 80 recomputed security cases, four current targets, zero
policy failures and `release.ready=true` for the closed security target set.
GH931 remains `INSUFFICIENT`. This producer result does not substitute for
the final review commit's own verifier or complete acceptance gates.
