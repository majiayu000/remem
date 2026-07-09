-- v065_preference_reinforcement: canonical machine-checkable eligibility flag
-- for preference reinforcement state.
--
-- v062 created memory_preference_reinforcements with the raw reinforcement
-- count but nothing populated it and it carried no compile-eligibility signal.
-- SP671-T3 wires the memory-candidate apply path to persist reinforcement
-- counts and records here whether the preference text deterministically yields
-- a v1 predicate (command_regex / commit_trailer_forbidden). The rule compiler
-- reads this flag as canonical eligibility state and still re-derives the
-- predicate from the source memory text so the compiled rule never drifts.
ALTER TABLE memory_preference_reinforcements
    ADD COLUMN machine_checkable INTEGER NOT NULL DEFAULT 0
    CHECK (machine_checkable IN (0, 1));

CREATE INDEX IF NOT EXISTS idx_memory_preference_reinforcements_eligible
    ON memory_preference_reinforcements(machine_checkable, reinforcement_count DESC);
