# Exact Q6 host identity aggregation correction

This support branch only rechecks the immutable artifacts of run
`37558506971`, attempt `1`, original support
`54a9d1a9da509bd2f94a4a86b4677aefb51493fc`, candidate
`4ec561b35932f4317b1c794c7364aad13c90b42f`. It does not rerun any of the eight
candidate gates or change candidate source, bodies, native evidence, execution
helpers, original logs, artifacts, or GitHub conclusions.

Both original group jobs succeeded. The original union failed because it used
different OS hostname strings as proof of independent execution. Both recorded
`runnervm8df0l`. Their authenticated job API assignments instead identify runner
IDs `1000153699` and `1000153700`; the original setup logs record distinct Hosted
Compute Agent Worker UUIDs. Each original host record matches its runner name,
GitHub-hosted environment, Ubuntu 24 image, and job interval. Their execution
intervals overlap for 5,590 seconds.

[GitHub's hosted-runner overview](https://docs.github.com/en/actions/concepts/runners/github-hosted-runners)
and [hosted-runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
describe standard hosted runners as new virtual machines. The pinned jobs use
`ubuntu-24.04`, not the container-based `ubuntu-slim` label. The corrected proof
concerns independent runner VM instances; it makes no claim about physical
hardware separation.

The new workflow runs only on
`audit/acceptance-recheck-20261007-4ec561b3`, with original R3 as the support
commit's sole parent. It checks out original R3 separately and validates all 16
original support-file hashes before importing its unmodified `verify_group`.
Each original group is verified again from the authenticated original ZIP. Its
result must equal the corresponding object in the original failed union.

The wrapper preserves all original strict comparisons: candidate SHA/tree and
ordered parents, frozen bodies and config, original command and helper hashes,
all eight actual passing gates, logs and output hashes, environment, every tool
version including `gh`, private ONNX Runtime, and system-loader proof. Only the
OS-hostname inequality is replaced by the stronger, pinned API/Worker/host/image
and overlap proof. The original union must have exactly the original hostname
error; any other failure remains blocking. The earlier proposed `gh` inventory
exception is not used.

The read-only token is sent solely to the initial `api.github.com` request.
Redirected artifact and job-log downloads use new requests without that token,
restricted to HTTPS Azure Blob endpoints. Neither tokens nor signed redirect
URLs are included in receipts or diagnostic errors. All three original ZIP
sizes and SHA-256 digests are pinned, checked against the original run's artifact
API, and checked again against downloaded bytes. Extraction rejects unsafe,
duplicate, symlink, non-regular, and oversized entries.

Original job HTTP log bytes are retained and hashed. Their entire UTF-8 text
must match the pinned connector-exact text hash after removing at most one
initial BOM and one terminal LF. No other whitespace normalization, decoding
replacement, or partial-log matching is permitted. The Worker UUID is then
checked within that authenticated full text. This explicitly separates original
HTTP bytes from the earlier connector's decoded text representation.

`corrected-union-verdict.json` is a separate, versioned result with its own
orchestration identity. It retains `original_run_conclusion: failure` and
`original_union_passed: false`. The workflow always uploads the evidence
collected so far and the separate verdict, including on recheck failure.
Success requires the complete original failed verdict, all three original ZIPs,
API snapshots, original HTTP job logs, fresh group verification, and file index.
A successful corrected union does not rewrite the original workflow's red
conclusion.

Preparation used 18 focused read-only/synthetic checks and a read-only replay
of the already authenticated original evidence. Those checks are not candidate
test runs or the new remote aggregation result. Only the new workflow's actual
output can provide the separate corrected remote verdict.
