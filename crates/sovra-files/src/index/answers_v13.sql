CREATE TABLE IF NOT EXISTS index_saved_answers (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id TEXT NOT NULL REFERENCES index_roots(id) ON DELETE CASCADE,
    file_id TEXT NOT NULL, version_id TEXT NOT NULL, ordinal INTEGER NOT NULL, snapshot TEXT NOT NULL,
    UNIQUE(root_id,file_id,version_id,ordinal)
);
PRAGMA user_version=13;
