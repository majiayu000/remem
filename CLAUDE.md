# remem Claude Code Instructions

Claude Code should follow the shared repository router in `AGENTS.md`. This file exists only as the Claude-specific entrypoint so the Codex and Claude instructions do not drift.

## Non-Negotiables

- remem 的目标是做最强的 Claude Code / Codex 记忆系统，不是最便宜的。
- Do not remove LLM extraction or automatic capture to save cost.
- Do not rely on Claude voluntarily calling `save_memory`; automatic hook capture is the primary path.
- Before using old spec files as requirements, read `docs/specs/README.md`. Most historical specs have already been implemented, superseded, or absorbed.

## Commands

```bash
cargo fmt --check
cargo check
cargo test
```

For plugin/runtime changes, also run the relevant Node and version-sync checks listed in `AGENTS.md`.
