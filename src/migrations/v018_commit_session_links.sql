-- v018_commit_session_links: durable traceability between git commits and
-- remem sessions. A commit can be linked to multiple sessions, and one session
-- can produce or observe multiple commits.

CREATE TABLE IF NOT EXISTS git_commits (
    id INTEGER PRIMARY KEY,
    project TEXT NOT NULL,
    repo_path TEXT NOT NULL,
    sha TEXT NOT NULL,
    short_sha TEXT NOT NULL,
    branch TEXT,
    message TEXT,
    authored_at_epoch INTEGER,
    changed_files TEXT NOT NULL DEFAULT '[]',
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    UNIQUE(project, sha)
);

CREATE TABLE IF NOT EXISTS git_commit_sessions (
    commit_id INTEGER NOT NULL REFERENCES git_commits(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    memory_session_id TEXT,
    source TEXT NOT NULL,
    linked_at_epoch INTEGER NOT NULL,
    PRIMARY KEY(commit_id, session_id)
);

CREATE INDEX IF NOT EXISTS idx_git_commits_project_short
    ON git_commits(project, short_sha);
CREATE INDEX IF NOT EXISTS idx_git_commits_project_updated
    ON git_commits(project, updated_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_git_commit_sessions_session
    ON git_commit_sessions(session_id);
CREATE INDEX IF NOT EXISTS idx_git_commit_sessions_memory_session
    ON git_commit_sessions(memory_session_id);
