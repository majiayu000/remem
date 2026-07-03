# @remem-ai/remem

npm wrapper for the `remem` native binary.

`remem` provides persistent project memory for Claude Code and OpenAI Codex
coding-agent sessions. The npm package downloads the matching prebuilt binary
from the GitHub Release during installation.

## Install

```bash
npm install -g @remem-ai/remem
remem install
```

`remem install` auto-detects existing Claude Code and Codex CLI config
directories. Use `--target codex`, `--target claude`, or `--target all` when
you need to force a specific integration.

Supported platforms:

- macOS arm64 / x64
- Linux arm64 / x64

Environment variables:

- `REMEM_NPM_SKIP_DOWNLOAD=1`: skip binary download during npm install
- `REMEM_NPM_BINARY=/path/to/remem`: run an explicit existing binary

See the product site and main project README for Claude Code, OpenAI Codex,
Codex CLI, and MCP setup details:

- https://majiayu000.github.io/remem/
- https://github.com/majiayu000/remem
