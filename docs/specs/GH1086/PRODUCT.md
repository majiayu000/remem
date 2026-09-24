# Project Alias CLI Product Contract

Status: Current contract
Tracking issue: #1086

## Outcome

A binary install exposes the existing audited project identity aliases. A user
can map a Git worktree to a canonical checkout, see the proposed change before
writing, list active mappings with their actor and reason, and revoke a mapping.
The canonical checkout must already have a `projects` row.

## Commands

- `remem project alias add <worktree-path> --canonical <checkout-path> --actor <name> --reason <text>` previews a single mapping and writes nothing.
- The same command with `--apply` records the alias and its proof in the existing audit tables.
- `remem project alias list` shows active mappings, canonical paths, actor, and reason.
- `remem project alias revoke <worktree-path> --actor <name> --reason <text>` previews a revocation; `--apply` records it.

Paths are resolved to the same Git root keys used by hooks. An add requires a
shared Git commit verified in the target checkout. Conflicting aliases and
missing canonical project rows fail visibly. A revoked alias no longer expands
`search --project` or MCP `search` scope. Historic memories keep their original
project path.

## Outside this change

Automatic worktree discovery or aliasing, capture-key changes, migrations, and
rewriting historical rows.

## Acceptance

The command is in `remem --help`; preview leaves the database unchanged;
apply and revoke each append an audit event; listing shows attribution; a
memory saved with a worktree key appears in CLI and MCP searches for the main
checkout only while its alias is active.
