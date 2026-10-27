ALTER TABLE index_files ADD COLUMN filename TEXT NOT NULL DEFAULT '';
ALTER TABLE index_files ADD COLUMN directory TEXT NOT NULL DEFAULT '';
ALTER TABLE index_files ADD COLUMN extension TEXT NOT NULL DEFAULT '';
ALTER TABLE index_files ADD COLUMN byte_size INTEGER;
ALTER TABLE index_files ADD COLUMN observed INTEGER;
ALTER TABLE index_files ADD COLUMN modified INTEGER;
ALTER TABLE index_files ADD COLUMN created INTEGER;
ALTER TABLE index_files ADD COLUMN extraction TEXT NOT NULL DEFAULT 'metadata_only';
ALTER TABLE index_files ADD COLUMN detail TEXT;
UPDATE index_files SET
    observed=(SELECT observed FROM index_versions WHERE id=current_version),
    modified=(SELECT modified FROM index_versions WHERE id=current_version),
    detail=(SELECT note FROM index_versions WHERE id=current_version),
    extraction=CASE WHEN EXISTS(SELECT 1 FROM index_version_chunks WHERE version_id=current_version) THEN 'text' ELSE 'metadata_only' END;
CREATE TABLE index_state (singleton INTEGER PRIMARY KEY CHECK(singleton=1), revision INTEGER NOT NULL);
INSERT INTO index_state VALUES(1,1);
CREATE INDEX file_catalogue_path ON index_files(root_id,path,id);
CREATE INDEX file_catalogue_modified ON index_files(modified) WHERE available=1;
CREATE TRIGGER catalogue_insert AFTER INSERT ON index_files BEGIN
    UPDATE index_state SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER catalogue_delete AFTER DELETE ON index_files BEGIN
    UPDATE index_state SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER catalogue_update AFTER UPDATE OF path,current_version,available,byte_size,modified,extraction,detail ON index_files
WHEN old.path IS NOT new.path OR old.current_version IS NOT new.current_version OR old.available IS NOT new.available
 OR old.byte_size IS NOT new.byte_size OR old.modified IS NOT new.modified OR old.extraction IS NOT new.extraction OR old.detail IS NOT new.detail
BEGIN
    UPDATE index_state SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER catalogue_root_delete AFTER DELETE ON index_roots BEGIN
    UPDATE index_state SET revision=revision+1 WHERE singleton=1;
END;
PRAGMA user_version=2;
