-- v022_topic_segments: Topic Loom 中间层（话题连续性 Phase 1）。
-- 在 session_rollup 阶段把事件范围切成连贯的话题段，介于 observations/
-- session_summaries 与 memories 之间。memories 仍是最终晋升产物。
--
-- 设计要点（Phase 0 Gate 2026-06-02 实测据此定型，见 SPEC-topic-continuity.md §0）：
--   * 并行/交错任务下话题段的事件区间会重叠/嵌套，因此本表不假设区间互斥。
--   * evidence_event_ids（JSON 离散集合）是段↔事件的权威关联。
--   * covered_from/to_event_id 仅为派生 min/max，用于排序与范围查询。
--   * 幂等键 = (session_row_id, topic_key)，由 persist 层处理，不靠 (from,to)。

CREATE TABLE IF NOT EXISTS topic_segments (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER NOT NULL REFERENCES sessions(id),
    project TEXT NOT NULL,
    topic_key TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    status TEXT NOT NULL,
    segment_index INTEGER NOT NULL,
    covered_from_event_id INTEGER NOT NULL,
    covered_to_event_id INTEGER NOT NULL,
    evidence_event_ids TEXT NOT NULL,
    files TEXT,
    confidence REAL NOT NULL DEFAULT 0.75,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

-- trace 聚合主路径：同 project 同 topic_key 按时间排序串成话题时间线。
CREATE INDEX IF NOT EXISTS idx_topic_segments_trace
    ON topic_segments(project_id, topic_key, covered_from_event_id);

-- persist 层幂等查重 + 按 session 回放。
CREATE INDEX IF NOT EXISTS idx_topic_segments_session
    ON topic_segments(session_row_id, topic_key);
