-- v078_event_capture_projection: give hook-originated compatibility events a
-- canonical capture identity without forcing audit-only event writers to
-- invent one.

ALTER TABLE events
    ADD COLUMN captured_event_id INTEGER
        REFERENCES captured_events(id) ON DELETE SET NULL;

CREATE UNIQUE INDEX idx_events_captured_event
ON events(captured_event_id)
WHERE captured_event_id IS NOT NULL;
