# remem Release Lifecycle

This checklist keeps remem's source version, binary assets, package registries,
plugin runtime metadata, and install paths aligned.

## Before Auto Tagging

- [ ] `Cargo.toml`, `Cargo.lock`, `plugins/remem/.codex-plugin/plugin.json`,
      `plugins/remem/runtimes/remem-releases.json`, and
      `npm/remem/package.json` use the same source version.
- [ ] `CHANGELOG.md` has an `Unreleased` section that mentions the current
      source version, or a top section for the release version being tagged.
- [ ] If the version is not published yet,
      `plugins/remem/runtimes/remem-releases.json` uses
      `state: "unreleased"`, has `assets: {}`, and does not set `base_url`.
- [ ] README install commands and distribution badges still point to live
      channels: Homebrew, GitHub Releases, crates.io, npm, and source build.
- [ ] Public surface checks pass:

```bash
python3 scripts/ci/check_plugin_version_sync.py
python3 scripts/ci/check_public_surface.py
python3 scripts/ci/check_public_claims.py
python3 scripts/ci/check_file_size.py
```

## Automatic Tagging

`Auto Release` is the only default path for creating release tags. After `CI`
passes on `main`, `.github/workflows/auto-release.yml` verifies that:

- required release secrets are present (`CRATES_IO_TOKEN` and `NPM_TOKEN`)
- the checked-out commit is the exact `main` commit that passed CI
- release metadata is synchronized with `Cargo.toml`
- `plugins/remem/runtimes/remem-releases.json` marks the current version as
  `state: "unreleased"` with empty `assets`
- the matching `vX.Y.Z` tag does not already point at another commit
- the source version is newer than the latest existing semver release tag

When all gates pass, the workflow creates the annotated `vX.Y.Z` tag and then
dispatches `.github/workflows/release.yml` against that tag ref. The explicit
dispatch is required because GitHub suppresses downstream workflow triggers
from tags created with the default `GITHUB_TOKEN`.

The release workflow builds assets, creates the GitHub Release, publishes
crates.io, and publishes npm. It also accepts manual `workflow_dispatch`, but
only when the selected ref is a `v*` tag.

Manual tag pushes remain a break-glass fallback only. Prefer rerunning
`Auto Release` with `workflow_dispatch` after fixing the failed gate.

## Public Claim Policy

README, changelog, release notes, and release-adjacent docs must link public
benchmark claims to committed report artifacts and the claim level they satisfy.
Until the linked artifacts pass the relevant gate, use directional wording only.

Allowed claim levels:

| Level | Claim | Required evidence |
|---|---|---|
| 1 | Reproducible local memory benchmark | A memory-system report from a clean checkout, full reproduction commands, and a passing artifact verifier. |
| 2 | Coding-agent outcome improvement | The #385 `no_memory` / `remem` / `curated_file` matrix on the same task set, at least three runs per condition, positive remem delta versus `no_memory`, reported token/turn/wall-time regressions, and the coding outcome stop-loss gate. |
| 3 | Public SOTA claim | A public benchmark comparison using the same model, budget, harness, and published artifacts; wording must name the benchmark and condition instead of generalizing to all long-term memory or all coding agents. |

The coding outcome stop-loss gate applies to README, release, marketing, and
roadmap wording that says remem improves coding-agent outcomes, beats a
maintained context file, or is broadly superior for coding workflows. The gate
passes only when all of these are true:

- remem beats `no_memory` on resolved rate by at least 10 percentage points, or
  by a statistically credible positive bootstrap interval.
- remem is not worse than `curated_file` by more than 3 percentage points.
- remem total token cost is at most `curated_file + 20%`, unless the report
  justifies a higher cost with a higher resolved rate.
- stale-memory-caused failures stay under 2% of runs.
- privacy and non-retention leak rate is 0 on the adversarial suite.
- All linked artifacts reproduce from a clean checkout.

If `curated_file` ties or beats remem with lower cost and no material usability
downside, record the stop-loss signal in the M6 roadmap before strengthening
release wording. The next slice should focus on ergonomics, export/import,
human-maintained memory workflows, and context-file integration.

Current public baseline: `eval/public/reports/baseline.md` and
`eval/public/reports/baseline.json` are `directional_only_no_public_claim`. They
do not support SOTA, broad superiority, or coding-task superiority wording.
CI enforces this boundary with:

```bash
python3 scripts/ci/check_public_claims.py
```

## Local Verification

Run focused checks before release approval:

```bash
sh -n install.sh
node --test plugins/remem/scripts/remem-runtime.test.js npm/remem/scripts/install.test.js
cargo fmt --check
cargo check --locked
```

Run the full suite before submission or release approval:

```bash
cargo test
```

Package dry runs:

```bash
cargo publish --dry-run --locked
npm pack ./npm/remem --pack-destination "$(mktemp -d)"
```

## Release Workflow Contract

The GitHub release workflow must produce:

- `remem-darwin-arm64.tar.gz`
- `remem-darwin-x64.tar.gz`
- `remem-linux-arm64.tar.gz`
- `remem-linux-x64.tar.gz`
- `SHA256SUMS`
- release-hosted `remem-releases.json` with exact sha256 values for all four
  platform archives

The npm wrapper downloads from the release-hosted manifest for its exact package
version. Do not publish npm until the GitHub Release assets are available.

## Post-Release Verification

After the tag workflow finishes:

```bash
curl -fsSI https://github.com/majiayu000/remem/releases/download/vX.Y.Z/remem-releases.json
npm view @remem-ai/remem version
cargo search remem-ai --limit 1
```

Clean install smoke:

```bash
install_dir="$(mktemp -d)"
REMEM_VERSION=vX.Y.Z REMEM_NO_CONFIG=1 REMEM_INSTALL_DIR="$install_dir" sh ./install.sh
"$install_dir/remem" --version
```

On macOS ARM, also verify ad-hoc signing after install:

```bash
codesign -dv "$install_dir/remem"
```

Then run:

```bash
remem install --target codex
remem doctor
remem search "last decision"
```

Do not announce a release until the GitHub Release, crates.io package, npm
package, and clean install smoke all agree on the same version.
