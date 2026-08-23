-- v089_supplemental_local_copy_receipt: bind the exact local-copy outcome to
-- supplemental activation receipts without rewriting prior immutable rows.

ALTER TABLE memory_activation_requests
    ADD COLUMN local_copy_status TEXT CHECK (
        local_copy_status IS NULL OR local_copy_status IN ('saved', 'disabled')
    );
ALTER TABLE memory_activation_requests
    ADD COLUMN local_copy_path TEXT CHECK (
        local_copy_path IS NULL OR (
            typeof(local_copy_path) = 'text'
            AND length(trim(local_copy_path)) > 0
            AND instr(local_copy_path, char(0)) = 0
        )
    );
ALTER TABLE memory_activation_requests
    ADD COLUMN local_copy_saved_at TEXT CHECK (
        local_copy_saved_at IS NULL OR (
            typeof(local_copy_saved_at) = 'text'
            AND length(trim(local_copy_saved_at)) > 0
            AND instr(local_copy_saved_at, char(0)) = 0
        )
    );
ALTER TABLE memory_activation_requests
    ADD COLUMN local_copy_sha256 TEXT CHECK (
        local_copy_sha256 IS NULL OR (
            typeof(local_copy_sha256) = 'text'
            AND length(local_copy_sha256) = 64
            AND local_copy_sha256 NOT GLOB '*[^0-9a-f]*'
        )
    );

CREATE TRIGGER memory_activation_requests_local_copy_receipt_insert
BEFORE INSERT ON memory_activation_requests
WHEN NOT (
    (
        NEW.route_kind <> 'supplemental_save'
        AND NEW.local_copy_status IS NULL
        AND NEW.local_copy_path IS NULL
        AND NEW.local_copy_saved_at IS NULL
        AND NEW.local_copy_sha256 IS NULL
    )
    OR (
        NEW.route_kind = 'supplemental_save'
        AND NEW.local_copy_status IS 'disabled'
        AND NEW.local_copy_path IS NULL
        AND NEW.local_copy_saved_at IS NULL
        AND NEW.local_copy_sha256 IS NULL
    )
    OR (
        NEW.route_kind = 'supplemental_save'
        AND NEW.local_copy_status IS 'saved'
        AND NEW.local_copy_path IS NOT NULL
        AND NEW.local_copy_saved_at IS NOT NULL
        AND NEW.local_copy_sha256 IS NOT NULL
    )
)
BEGIN
    SELECT RAISE(ABORT, 'invalid supplemental local-copy receipt');
END;
