# Shared agent session parsing implementation

Status: Current contract (implementation in progress)

## Boundaries

- memory/raw_transcript.rs adapts agent_sessions::read_raw_from with unlimited size/line limits to preserve existing accepted inputs. Keep one pending record; bound the input with take(limit), then check captured length after framing so an unterminated short tail releases the preceding pending row before UnexpectedEof with the existing diagnostic text. UTF-8 and CR/LF stripping remain Remem policy.
- project_conversation and tolerant_timestamp_epoch replace duplicate native envelope/text/timestamp decoding. Keep reconciliation classifications and role constants local.
- ingest/sessions.rs maps Roots to required local scan roots when the corresponding CLAUDE_CONFIG_DIR/CODEX_HOME override is present, and optional roots only for inferred defaults and discover_directory to existing sorted paths and failure collection. Default root resolution becomes fallible at both CLI callers; explicit roots remain independently checked.
- git_evidence.rs uses borrowed project_codex_function envelopes, retaining command parsing, tool allowlist, string field requirements, pair removal, successful process markers and resolved commit identity.
- ingest/session_identity.rs uses shared raw framing for its bounded prefix and native origin projection. Remem retains its first session ID/cwd/branch identity strategy and maps Exec to its existing unattended label; IDE remains interactive.
- Preserve the published `default_scan_roots() -> Vec<ScanRoot>` signature as a compatibility wrapper that logs resolver errors. CLI ingest and raw repair use a separate fallible resolver so invalid configuration fails the command.
- v093 adds `raw_session_identities.session_mode_version`, constrained to 0 (legacy, the SQL default) or 1 (shared). New claims explicitly write version 1. The same bounded prefix computes legacy and shared modes. In the existing Phase A transaction, verify host provenance first. For a legacy known mode, conflicting known legacy evidence still fails; known matching legacy evidence permits a one-time transition to a known shared mode. Matching shared evidence can adopt version 1 when legacy evidence is unknown. Unknown shared evidence retains the known stored mode and version 0; insufficient evidence for a conflicting mode remains an error. Version 1 keeps the existing known-to-different-known rejection and unknown-to-known promotion. Classification updates occur even when mtime/size and raw cursor are unchanged, without resetting cursor or raw identity. Batch failure rolls back both mode and version.
- Version metadata advances together under the repository version-sync contract. No hook configuration changes.

## Verification

Existing raw archive, reconciliation, ingestion, identity and Git evidence tests exercise persistence policy. New structural assertions cover mixed content/timestamp precedence, short-capture callback order, exact CRLF/UTF-8 behavior and arbitrary-root discovery. Regression tests cover v092-to-v093 defaults/constraints, one-time reclassification with unchanged cursor, actual host/mode conflicts, batch rollback, and required overrides with an unaffected optional host. The full local preflight is the final gate; targeted tests precede it. Smoke runs use temporary HOME and REMEM_DATA_DIR. No private transcript fixtures enter Git.

## Rollback

Revert the dependency/adapters as one release. The additive classifier column preserves existing IDs, raw messages and cursors. Older SQL readers can ignore the column, and older writers omit it and therefore create legacy-version rows. Schema downgrade is not supported by the migration runner: do not run an older binary against a database upgraded to v093; restore a pre-upgrade backup or ship a forward rollback retaining v093 and its provenance-aware classifier. Reverting source adapters alone is insufficient after classifications have upgraded. Shared library publication must precede downstream registry verification.
