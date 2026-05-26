# @majiayu000/remem

npm wrapper for the `remem` native binary.

`remem` provides persistent project memory for Claude Code and Codex. The npm
package downloads the matching prebuilt binary from the GitHub Release during
installation.

## Install

```bash
npm install -g @majiayu000/remem
remem install --target codex
```

Supported platforms:

- macOS arm64 / x64
- Linux arm64 / x64

Environment variables:

- `REMEM_NPM_SKIP_DOWNLOAD=1`: skip binary download during npm install
- `REMEM_NPM_BINARY=/path/to/remem`: run an explicit existing binary

See the main project README for Claude Code and Codex setup details:
https://github.com/majiayu000/remem
