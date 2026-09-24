# Project Alias CLI Technical Contract

Status: Current contract
Tracking issue: #1086

Use the existing `project_alias` plan preview/apply functions and v082 tables.
The CLI constructs one `ProjectAliasPlanEntry` from the two Git roots and a
commit verified present in the canonical repository, with a digest of the
proof payload and selected-path snapshot. Preview opens the current database
read-only; apply opens the existing current database for writes and rechecks
the proof before committing. An existing canonical `projects` row remains a
precondition of the underlying plan.

The retrieval hit hydration query must expand the requested canonical project
through `project_filter_values`, just like its candidate channels. Otherwise
FTS finds a worktree memory but the final ID load drops it before CLI and MCP
results are rendered.

Adding `project` changes the fingerprinted `remem help` output. The prior help
identity is recorded in the GH969 retirement ledger; existing command names
and arguments remain available. Reverting this staged CLI change restores the
prior parser and help surface before release.

List reads active aliases joined with their latest `activate` event and
canonical project row. Revoke copies the active event's proof binding into a
new `revoke` event, records the new actor/reason, and marks the alias revoked
in one transaction. A missing or already revoked alias is an error. Preview
does not write or migrate the store. Database and proof failures propagate as
nonzero command exits.

Focused tests cover CLI parsing, preview side effects, append-only event
attribution, active search-scope expansion and its disappearance on revoke,
plus proof/collision failures. Run `cargo fmt --check`, `cargo check`, and the
repository's required tests before submission.
