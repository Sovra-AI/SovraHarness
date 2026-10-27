CREATE TABLE index_semantic_settings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id TEXT NOT NULL UNIQUE REFERENCES index_roots(id) ON DELETE CASCADE,
    target TEXT NOT NULL, dimensions INTEGER
);
CREATE TABLE index_embeddings (
    setting_id INTEGER NOT NULL REFERENCES index_semantic_settings(id) ON DELETE CASCADE,
    chunk_id TEXT NOT NULL REFERENCES index_chunks(id) ON DELETE CASCADE,
    vector BLOB NOT NULL,
    PRIMARY KEY(setting_id, chunk_id)
);
CREATE INDEX embedding_chunk ON index_embeddings(chunk_id);
PRAGMA user_version=6;
