# Native evidence from run 37657526048

Source producer: b0585181ef3c9d179abef10b4af36764d08f456a, tree 6ce9ccb06600a2998f63d1035442d7ea3936e7c1.
The [native workflow run](https://github.com/majiayu000/remem/actions/runs/37657526048)
completed all four target jobs and its aggregate on attempt 1.

All five original artifact ZIPs are retained unchanged with their API metadata,
sizes and SHA-256 digests in import.json. The four row receipts and aggregate
receipt are original downloaded bytes. They bind the native target-qualified
paths; they are not relabeled as receipts for the mechanically relocated paths.

The import preserves the existing committed layout: macOS ARM uses
adversarial-policy-v2, Linux x64 uses adversarial-policy-v2-linux-x86_64;
macOS x64 and Linux ARM retain their target-qualified names. Only JSON path
references in manifests, reports and run records may change during relocation,
and reversing the mapping must recover the exact native bytes. All 480 payloads,
including 80 SQLite snapshots, are copied byte for byte. Producer SHA, source
fingerprint, platform and suite identities remain unchanged.

import.json records all 568 mappings and the actual static authentication checks.
The producer's original aggregate records 80 actual security replays with zero
policy failures. Static import authentication does not substitute for executing
the Rust verifier against the clean evidence-only candidate. That candidate's
actual verifier, full-preflight and final-head CI results are recorded separately
in its pull request; no preflight success or merge/release approval is implied here.

These deterministic offline fixtures do not establish live-model extraction
quality or the separate GH931 research claims.
