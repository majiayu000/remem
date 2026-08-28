# Installation and upgrades

Use one installation channel and keep one canonical `remem` executable on
`PATH`. After installing or replacing the binary, run `remem install` for the
hosts you use so their hooks and MCP registration point at the current binary.

## Supported channels

| Channel | Install or update the binary |
|---|---|
| Homebrew | `brew install majiayu000/tap/remem` or `brew upgrade remem` |
| Standalone installer | `curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh \| env REMEM_NO_CONFIG=1 sh` |
| npm | `npm install -g @remem-ai/remem` or `npm update -g @remem-ai/remem` |
| Cargo | `cargo install remem-ai --bin remem` |
| GitHub Release | Download the matching archive and checksum from [GitHub Releases](https://github.com/majiayu000/remem/releases/latest) |
| Source build | `cargo build --release` and copy `target/release/remem` onto `PATH` |

Homebrew, the standalone installer, the npm wrapper, and GitHub Releases
provide macOS and Linux x64/arm64 binaries. Windows supports Claude Code and
Codex through Cargo or a source build; the Cursor v1 target is not supported
on Windows.

On macOS ARM, manually copied or source-built binaries need ad-hoc signing:

```bash
codesign -s - -f /path/to/remem
```

The standalone installer verifies the published SHA-256 checksum and performs
this signing step automatically.

## Configure a host

Configure the host after the binary is available:

```bash
remem install --target codex
# or
remem install --target claude
# or
remem install --target all
```

`--target auto` only selects hosts whose configuration directories already
exist. Use an explicit target on a first-time setup so remem can create the
selected configuration files. `--target all` includes every known host and
therefore requires a platform with an approved Cursor renderer. On Windows,
configure Claude Code and Codex separately instead of using `--target all`.
Cursor remains MCP-only in the v1 installer.

For Homebrew, pass the canonical formula binary explicitly if another copy of
`remem` may already be on `PATH`:

```bash
"$(brew --prefix remem)/bin/remem" install --target codex
```

The standalone installer is normally used in two steps so installation and
host configuration stay explicit:

```bash
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | env REMEM_NO_CONFIG=1 sh
~/.local/bin/remem install --target codex
```

## Verify an installation

Restart the configured coding agent, then run:

```bash
remem doctor
remem status
remem search "last decision"
```

`remem doctor` checks the database, encryption key, hooks, MCP registration,
worker, and install-path drift. If it reports multiple binaries, resolve the
PATH conflict and rerun `remem install` with the intended executable.

For incomplete Claude hooks, preview and apply the focused repair:

```bash
remem install --target claude --repair --dry-run
remem install --target claude --repair
```

Repair mode updates Claude hooks only; it does not register MCP, initialize
the runtime store, or create an API token.

## Upgrade an existing installation

Upgrade through the same channel that installed the binary. Avoid leaving a
second manual or package-manager copy elsewhere on `PATH`.

After every binary replacement, rerun host configuration and verification:

```bash
remem install --target codex
# Use --target claude or --target all when appropriate.
remem doctor
```

This step refreshes host-aware hook commands and MCP registration. It is
especially important after a manual copy, source build, or change of install
path.

If moving between installation channels, first use the old binary to preview
and remove its owned host entries. This does not remove the executable:

```bash
/old/path/remem uninstall --dry-run
/old/path/remem uninstall
```

Next, remove the old package or exact manual binary with the command matching
the channel that installed it. Run only one applicable command:

```bash
brew uninstall remem                    # Homebrew
npm uninstall -g @remem-ai/remem        # npm
cargo uninstall remem-ai                # Cargo
rm /exact/path/to/old/remem              # standalone, release, or manual copy
```

Install the new channel, then configure hosts with that channel's canonical
executable:

```bash
/new/path/remem install --target codex
```

Use `--target claude` as well when that host is installed. On supported
macOS/Linux systems, `--target all` can configure every known host at once.

Before downgrading an installation that configured Cursor, run
`remem uninstall --target cursor` with the current version, then install the
older binary and configure the desired targets again.

## Data and uninstall safety

`remem uninstall` removes owned hooks and MCP registration but does not delete
the memory database:

```bash
remem uninstall --dry-run
remem uninstall
```

Memory remains in the configured `REMEM_DATA_DIR`. Back up that directory
before deleting it manually. See the root [README](../README.md#uninstall) for
the concise uninstall contract and [SECURITY.md](../SECURITY.md) for the
security policy.
