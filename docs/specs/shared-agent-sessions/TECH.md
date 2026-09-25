# Shared agent session parsing implementation

Status: Current contract (locally verified; rollout pending)

## Boundaries

- memory/raw_transcript.rs adapts agent_sessions::read_raw_from with unlimited size/line limits to preserve existing accepted inputs. Keep one pending record; bound the input with take(limit), then check captured length after framing so an unterminated short tail releases the preceding pending row before UnexpectedEof with the existing diagnostic text. UTF-8 and CR/LF stripping remain Remem policy.
- project_conversation and tolerant_timestamp_epoch replace duplicate native envelope/text/timestamp decoding. Keep reconciliation classifications and role constants local.
- ingest/sessions.rs maps Roots to optional local scan roots and discover_directory to existing sorted paths and failure collection. CLI callers use a fallible root resolver; the existing public Vec-returning helper keeps its signature and logs invalid configuration at error level; explicit roots remain independently checked.
- git_evidence.rs uses borrowed project_codex_function envelopes, retaining command parsing, tool allowlist, string field requirements, pair removal, successful process markers and resolved commit identity.
- ingest/session_identity.rs uses shared raw framing for its bounded prefix and native origin projection. Remem retains its first session ID/cwd/branch identity strategy and maps Exec to its existing unattended label; IDE remains interactive.
- Version metadata advances together under the repository version-sync contract. No hook configuration or database migration.

## Verification

Existing raw archive, reconciliation, ingestion, identity and Git evidence tests exercise persistence policy. New structural assertions cover mixed content/timestamp precedence, short-capture callback order, exact CRLF/UTF-8 behavior and arbitrary-root discovery. The full local preflight is the final gate; targeted tests precede it. Smoke runs use temporary HOME and REMEM_DATA_DIR. No private transcript fixtures enter Git.

## Rollback

Revert the dependency/adapters as one release. No stored identities or schema are rewritten, so existing databases remain readable. Shared library publication must precede downstream registry verification.
