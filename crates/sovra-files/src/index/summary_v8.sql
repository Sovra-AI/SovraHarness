CREATE TABLE index_summary_configurations (
    id INTEGER PRIMARY KEY AUTOINCREMENT, plan TEXT NOT NULL,
    enabled INTEGER NOT NULL, created INTEGER NOT NULL, calls_used INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE index_summary_scopes (
    configuration_id INTEGER NOT NULL REFERENCES index_summary_configurations(id),
    ordinal INTEGER NOT NULL,
    root_id TEXT REFERENCES index_roots(id) ON DELETE SET NULL,
    PRIMARY KEY(configuration_id,ordinal)
);
CREATE INDEX summary_scope_root ON index_summary_scopes(root_id);
CREATE TABLE index_summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    configuration_id INTEGER NOT NULL REFERENCES index_summary_configurations(id),
    kind TEXT NOT NULL, level INTEGER NOT NULL, signature TEXT NOT NULL,
    created INTEGER NOT NULL, invalidated INTEGER, reason TEXT, details TEXT NOT NULL,
    search_text TEXT NOT NULL
);
CREATE UNIQUE INDEX summary_current_signature ON index_summaries(configuration_id,signature) WHERE invalidated IS NULL;
CREATE INDEX summary_configuration ON index_summaries(configuration_id,id);
CREATE TABLE index_summary_attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    configuration_id INTEGER NOT NULL REFERENCES index_summary_configurations(id),
    signature TEXT NOT NULL, state TEXT NOT NULL, created INTEGER NOT NULL,
    result_id INTEGER REFERENCES index_summaries(id), error TEXT
);
CREATE INDEX summary_attempt_configuration ON index_summary_attempts(configuration_id,id);
CREATE TABLE index_summary_edges (
    parent INTEGER NOT NULL REFERENCES index_summaries(id),
    child INTEGER NOT NULL REFERENCES index_summaries(id),
    PRIMARY KEY(parent,child)
);
CREATE INDEX summary_edge_child ON index_summary_edges(child,parent);
-- The deduplicated leaf closure makes invalidation independent of tree depth
-- and overlapping soft clusters, without traversing every parent at scan time.
CREATE TABLE index_summary_sources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    summary_id INTEGER NOT NULL REFERENCES index_summaries(id),
    root_id TEXT REFERENCES index_roots(id) ON DELETE SET NULL,
    file_id TEXT REFERENCES index_files(id) ON DELETE SET NULL,
    version_id TEXT REFERENCES index_versions(id) ON DELETE SET NULL,
    ordinal INTEGER NOT NULL,
    UNIQUE(summary_id,root_id,file_id,version_id,ordinal)
);
CREATE INDEX summary_source_file ON index_summary_sources(file_id,summary_id);
CREATE INDEX summary_source_root ON index_summary_sources(root_id,summary_id);
CREATE INDEX summary_source_version ON index_summary_sources(version_id,summary_id);
CREATE TRIGGER summary_changed_source AFTER UPDATE OF current_version,available,path ON index_files
WHEN old.current_version IS NOT new.current_version OR old.available IS NOT new.available OR old.path IS NOT new.path
BEGIN
    UPDATE index_summaries SET invalidated=CAST(strftime('%s','now') AS INTEGER),reason='"source_changed"'
    WHERE invalidated IS NULL AND id IN (SELECT summary_id FROM index_summary_sources WHERE file_id=old.id);
END;
CREATE TRIGGER summary_deleted_source BEFORE DELETE ON index_files BEGIN
    UPDATE index_summaries SET invalidated=CAST(strftime('%s','now') AS INTEGER),reason='"source_changed"'
    WHERE invalidated IS NULL AND id IN (SELECT summary_id FROM index_summary_sources WHERE file_id=old.id);
END;
CREATE TRIGGER summary_revoked_scope BEFORE DELETE ON index_roots BEGIN
    UPDATE index_summary_configurations SET enabled=0
    WHERE id IN (SELECT configuration_id FROM index_summary_scopes WHERE root_id=old.id);
    UPDATE index_summaries SET invalidated=CAST(strftime('%s','now') AS INTEGER),reason='"grant_removed"'
    WHERE invalidated IS NULL AND configuration_id IN (SELECT configuration_id FROM index_summary_scopes WHERE root_id=old.id);
END;
PRAGMA user_version=8;

