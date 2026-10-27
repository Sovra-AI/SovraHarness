CREATE TABLE index_scans (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id TEXT NOT NULL REFERENCES index_roots(id) ON DELETE CASCADE,
    started INTEGER NOT NULL, finished INTEGER, status TEXT NOT NULL,
    extractor_revision TEXT NOT NULL
);
ALTER TABLE index_versions ADD COLUMN extractor_revision TEXT;
ALTER TABLE index_versions ADD COLUMN generated_by INTEGER REFERENCES index_scans(id);
CREATE TABLE index_observations (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id TEXT NOT NULL REFERENCES index_files(id) ON DELETE CASCADE,
    scan_id INTEGER REFERENCES index_scans(id),
    version_id TEXT REFERENCES index_versions(id) ON DELETE CASCADE,
    path TEXT NOT NULL, identity TEXT NOT NULL, present INTEGER NOT NULL,
    byte_size INTEGER, modified INTEGER, created INTEGER,
    extraction TEXT NOT NULL, note TEXT,
    recorded INTEGER NOT NULL, observed INTEGER,
    origin TEXT NOT NULL CHECK(origin IN ('scan','legacy_import')),
    replaces INTEGER REFERENCES index_observations(sequence)
);
CREATE INDEX observation_file ON index_observations(file_id,sequence DESC);
CREATE INDEX observation_path ON index_observations(path,file_id);
CREATE INDEX observation_version ON index_observations(version_id);
PRAGMA user_version=4;

