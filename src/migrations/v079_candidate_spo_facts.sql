-- GH-956: persist extraction-produced SPO facts on memory candidates so the
-- review/auto-promote round-trip through the database keeps them until the
-- promotion transaction writes memory_facts rows.
ALTER TABLE memory_candidates ADD COLUMN facts TEXT;
