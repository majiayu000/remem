-- v041_content_identity_sha256: migrate durable content identity hashes from
-- legacy unversioned FNV64 to sha256:content-v1:<hex>.
--
-- SQLite has no built-in SHA-256 helper in this binary, so the actual data
-- backfill runs in the Rust post-migration hook for v041.
SELECT 1;
