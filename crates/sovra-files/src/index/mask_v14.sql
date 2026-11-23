CREATE TABLE IF NOT EXISTS index_masks (
    version_id TEXT PRIMARY KEY REFERENCES index_versions(id) ON DELETE CASCADE,
    width INTEGER NOT NULL, height INTEGER NOT NULL, bits BLOB NOT NULL
);
PRAGMA user_version=14;
