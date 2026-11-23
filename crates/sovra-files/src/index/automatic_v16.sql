-- One resumable job per currently available file; old snapshots are invalidated
-- in the catalogue transaction, including revocation, rename and replacement.
CREATE TABLE IF NOT EXISTS index_auto_files (
    file_id TEXT NOT NULL UNIQUE REFERENCES index_files(id) ON DELETE CASCADE,
    next_ordinal INTEGER DEFAULT 0, byte_offset INTEGER NOT NULL DEFAULT 0, kind TEXT, area TEXT
);
CREATE INDEX IF NOT EXISTS auto_pending ON index_auto_files(next_ordinal) WHERE next_ordinal IS NOT NULL;
CREATE TABLE IF NOT EXISTS index_auto_findings (
    id INTEGER PRIMARY KEY, file_id TEXT NOT NULL REFERENCES index_auto_files(file_id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL, line INTEGER NOT NULL, category TEXT NOT NULL, detail TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS auto_finding_file ON index_auto_findings(file_id);
CREATE INDEX IF NOT EXISTS auto_finding_category ON index_auto_findings(category,id);
CREATE TABLE IF NOT EXISTS index_auto_counts (facet TEXT NOT NULL, label TEXT NOT NULL, n INTEGER NOT NULL, PRIMARY KEY(facet,label));
CREATE TABLE IF NOT EXISTS index_auto_state (singleton INTEGER PRIMARY KEY, through_row INTEGER NOT NULL, last_old_row INTEGER NOT NULL, revision INTEGER NOT NULL);
INSERT OR IGNORE INTO index_auto_state SELECT 1,0,coalesce(max(rowid),0),1 FROM index_files;
CREATE TRIGGER IF NOT EXISTS auto_file_added AFTER INSERT ON index_files WHEN new.available=1 BEGIN
    INSERT INTO index_auto_files(file_id) VALUES(new.id);
    UPDATE index_auto_state SET revision=revision+1;
END;
CREATE TRIGGER IF NOT EXISTS auto_file_changed AFTER UPDATE OF path,available,current_version ON index_files
WHEN old.path IS NOT new.path OR old.available IS NOT new.available OR old.current_version IS NOT new.current_version BEGIN
    DELETE FROM index_auto_files WHERE file_id=old.id;
    INSERT INTO index_auto_files(file_id) SELECT new.id WHERE new.available=1;
    UPDATE index_auto_state SET revision=revision+1;
END;
CREATE TRIGGER IF NOT EXISTS auto_file_classified AFTER UPDATE OF kind,area ON index_auto_files WHEN old.kind IS NULL AND new.kind IS NOT NULL BEGIN
    INSERT INTO index_auto_counts VALUES('kind',new.kind,1) ON CONFLICT DO UPDATE SET n=n+1;
    INSERT INTO index_auto_counts SELECT 'area',new.area,1 WHERE new.area IS NOT NULL ON CONFLICT DO UPDATE SET n=n+1;
END;
CREATE TRIGGER IF NOT EXISTS auto_file_removed AFTER DELETE ON index_auto_files BEGIN
    UPDATE index_auto_counts SET n=n-1 WHERE (facet='kind' AND label=old.kind) OR (facet='area' AND label=old.area);
    UPDATE index_auto_state SET revision=revision+1;
END;
CREATE TRIGGER IF NOT EXISTS auto_finding_added AFTER INSERT ON index_auto_findings BEGIN
    INSERT INTO index_auto_counts VALUES('finding',new.category,1) ON CONFLICT DO UPDATE SET n=n+1;
END;
CREATE TRIGGER IF NOT EXISTS auto_finding_removed AFTER DELETE ON index_auto_findings BEGIN
    UPDATE index_auto_counts SET n=n-1 WHERE facet='finding' AND label=old.category;
END;
CREATE TABLE IF NOT EXISTS index_auto_terms (
    file_id TEXT NOT NULL REFERENCES index_auto_files(file_id) ON DELETE CASCADE,
    facet TEXT NOT NULL, label TEXT NOT NULL, identity TEXT NOT NULL,
    PRIMARY KEY(file_id,facet,label,identity)
);
CREATE TRIGGER IF NOT EXISTS auto_term_added AFTER INSERT ON index_auto_terms BEGIN
    INSERT INTO index_auto_counts VALUES(new.facet,new.label,1) ON CONFLICT DO UPDATE SET n=n+1;
END;
CREATE TRIGGER IF NOT EXISTS auto_term_removed AFTER DELETE ON index_auto_terms BEGIN
    UPDATE index_auto_counts SET n=n-1 WHERE facet=old.facet AND label=old.label;
END;
PRAGMA user_version=16;
