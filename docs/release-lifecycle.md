# remem Release Lifecycle

This checklist keeps remem's source version, binary assets, package registries,
plugin runtime metadata, and install paths aligned.

## Before Tagging

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
python3 scripts/ci/check_file_size.py
```

## Local Verification

Run focused checks before creating the tag:

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
