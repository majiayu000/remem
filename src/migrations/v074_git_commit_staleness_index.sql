-- v074_git_commit_staleness_index: make source-anchor staleness lookups scale
-- with commits newer than the anchor instead of the full project history
-- (GH-948).

CREATE INDEX idx_git_commits_project_commit_epoch
ON git_commits(
    project,
    COALESCE(authored_at_epoch, updated_at_epoch, created_at_epoch) DESC,
    id DESC
);

-- changed_files remains the captured JSON payload. This relation is derived,
-- migration-managed lookup data so readers do not repeatedly decode that JSON.
-- Paths remain byte-for-byte unchanged: readers apply the existing Rust path
-- normalization so historical "./", absolute, and whitespace variants keep
-- their current overlap semantics.
CREATE TABLE git_commit_files (
    commit_id INTEGER NOT NULL
        REFERENCES git_commits(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    PRIMARY KEY(commit_id, path)
) WITHOUT ROWID;

CREATE TRIGGER git_commits_validate_changed_files_insert
BEFORE INSERT ON git_commits
BEGIN
    SELECT CASE
        WHEN json_valid(NEW.changed_files) = 0
        THEN RAISE(
            ABORT,
            'git_commits.changed_files must be a JSON array of strings'
        )
    END;
    SELECT CASE
        WHEN json_type(NEW.changed_files) != 'array'
        THEN RAISE(
            ABORT,
            'git_commits.changed_files must be a JSON array of strings'
        )
    END;
    SELECT CASE
        WHEN EXISTS (
            SELECT 1
            FROM json_each(NEW.changed_files) AS changed_file
            WHERE changed_file.type != 'text'
        )
        THEN RAISE(
            ABORT,
            'git_commits.changed_files must be a JSON array of strings'
        )
    END;
END;

CREATE TRIGGER git_commits_validate_changed_files_update
BEFORE UPDATE OF changed_files ON git_commits
BEGIN
    SELECT CASE
        WHEN json_valid(NEW.changed_files) = 0
        THEN RAISE(
            ABORT,
            'git_commits.changed_files must be a JSON array of strings'
        )
    END;
    SELECT CASE
        WHEN json_type(NEW.changed_files) != 'array'
        THEN RAISE(
            ABORT,
            'git_commits.changed_files must be a JSON array of strings'
        )
    END;
    SELECT CASE
        WHEN EXISTS (
            SELECT 1
            FROM json_each(NEW.changed_files) AS changed_file
            WHERE changed_file.type != 'text'
        )
        THEN RAISE(
            ABORT,
            'git_commits.changed_files must be a JSON array of strings'
        )
    END;
END;

CREATE TRIGGER git_commits_sync_files_insert
AFTER INSERT ON git_commits
BEGIN
    INSERT OR IGNORE INTO git_commit_files(commit_id, path)
    SELECT NEW.id, CAST(changed_file.value AS TEXT)
    FROM json_each(NEW.changed_files) AS changed_file;
END;

CREATE TRIGGER git_commits_sync_files_update
AFTER UPDATE OF changed_files ON git_commits
WHEN OLD.changed_files IS NOT NEW.changed_files
BEGIN
    DELETE FROM git_commit_files WHERE commit_id = NEW.id;
    INSERT OR IGNORE INTO git_commit_files(commit_id, path)
    SELECT NEW.id, CAST(changed_file.value AS TEXT)
    FROM json_each(NEW.changed_files) AS changed_file;
END;

CREATE TRIGGER git_commits_sync_files_delete
AFTER DELETE ON git_commits
BEGIN
    DELETE FROM git_commit_files WHERE commit_id = OLD.id;
END;
