CREATE TABLE IF NOT EXISTS index_roots (
    id TEXT PRIMARY KEY, path TEXT NOT NULL UNIQUE, identity TEXT NOT NULL,
    generation INTEGER NOT NULL DEFAULT 0, details TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS index_files (
    id TEXT PRIMARY KEY, root_id TEXT NOT NULL REFERENCES index_roots(id) ON DELETE CASCADE,
    path TEXT NOT NULL, identity TEXT NOT NULL, current_version TEXT,
    generation INTEGER NOT NULL, available INTEGER NOT NULL DEFAULT 0,
    UNIQUE(root_id,path)
);
CREATE INDEX IF NOT EXISTS file_identity ON index_files(root_id,identity);
CREATE TABLE IF NOT EXISTS index_versions (
    id TEXT PRIMARY KEY, file_id TEXT NOT NULL REFERENCES index_files(id) ON DELETE CASCADE,
    digest TEXT NOT NULL, mime TEXT NOT NULL, note TEXT, observed INTEGER NOT NULL,
    modified INTEGER
);
CREATE TABLE IF NOT EXISTS index_chunks (
    rowid INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, body TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS index_fts USING fts5(body,content='index_chunks',content_rowid='rowid',tokenize='unicode61');
CREATE TRIGGER IF NOT EXISTS index_chunks_insert AFTER INSERT ON index_chunks BEGIN
    INSERT INTO index_fts(rowid,body) VALUES(new.rowid,new.body);
END;
CREATE TRIGGER IF NOT EXISTS index_chunks_delete AFTER DELETE ON index_chunks BEGIN
    INSERT INTO index_fts(index_fts,rowid,body) VALUES('delete',old.rowid,old.body);
END;
CREATE TABLE IF NOT EXISTS index_version_chunks (
    version_id TEXT NOT NULL REFERENCES index_versions(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL, chunk_id TEXT NOT NULL REFERENCES index_chunks(id),
    start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
    PRIMARY KEY(version_id,ordinal)
);
CREATE INDEX IF NOT EXISTS version_chunk_id ON index_version_chunks(chunk_id);
PRAGMA user_version=1;
