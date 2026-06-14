-- v043_graph_candidate_prompt_memory_refs: persist the exact memory refs
-- shown to graph candidate extraction so conflict approvals can fail closed.

ALTER TABLE graph_candidates
ADD COLUMN prompt_memory_ref_ids TEXT;
