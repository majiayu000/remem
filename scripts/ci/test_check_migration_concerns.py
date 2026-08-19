#!/usr/bin/env python3
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_migration_concerns as concerns


class ExtraRewriteTablesTests(unittest.TestCase):
    def test_same_table_backfill_is_one_concern(self) -> None:
        sql = """
        ALTER TABLE memories ADD COLUMN enrichment_state TEXT;
        UPDATE memories SET enrichment_state = 'pending';
        """
        self.assertEqual(concerns.extra_rewrite_tables(sql), set())

    def test_trigger_body_rewrite_is_schema_not_upgrade_dml(self) -> None:
        sql = """
        ALTER TABLE memories ADD COLUMN enrichment_state TEXT;
        CREATE TRIGGER memories_au AFTER UPDATE ON memories
        BEGIN
            DELETE FROM memory_embeddings WHERE memory_id = new.id;
        END;
        """
        self.assertEqual(concerns.extra_rewrite_tables(sql), set())

    def test_unrelated_pricing_rewrite_is_mixed(self) -> None:
        sql = """
        ALTER TABLE memories ADD COLUMN enrichment_state TEXT;
        UPDATE memories SET enrichment_state = 'deferred';
        UPDATE ai_usage_events SET pricing_source = 'unknown_pricing';
        """
        self.assertEqual(concerns.extra_rewrite_tables(sql), {"ai_usage_events"})

    def test_data_only_rewrite_is_one_concern(self) -> None:
        sql = "UPDATE ai_usage_events SET estimated_cost_usd = 0.0;"
        self.assertEqual(concerns.extra_rewrite_tables(sql), set())

    def test_v083_is_detected_as_mixed(self) -> None:
        sql = (
            concerns.ROOT / "src/migrations/v083_retrieval_enrichment_budget.sql"
        ).read_text(encoding="utf-8")
        self.assertEqual(concerns.extra_rewrite_tables(sql), {"ai_usage_events"})

    def test_v084_is_not_mixed(self) -> None:
        sql = (
            concerns.ROOT / "src/migrations/v084_legacy_pending_bridge_state.sql"
        ).read_text(encoding="utf-8")
        self.assertEqual(concerns.extra_rewrite_tables(sql), set())


class CheckMigrationsTests(unittest.TestCase):
    def test_current_tree_is_clean_with_historical_allowlist(self) -> None:
        self.assertEqual(concerns.check_migrations(), [])

    def test_new_mixed_migration_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            migrations = root / "src" / "migrations"
            migrations.mkdir(parents=True)
            (migrations / "v085_mixed.sql").write_text(
                "ALTER TABLE memories ADD COLUMN x TEXT;\n"
                "UPDATE jobs SET state = 'failed';\n",
                encoding="utf-8",
            )
            errors = concerns.check_migrations(root)
        self.assertTrue(any("v085_mixed.sql" in error and "jobs" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
