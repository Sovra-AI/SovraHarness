ALTER TABLE index_versions ADD COLUMN partial INTEGER NOT NULL DEFAULT 0 CHECK(partial IN (0,1));
CREATE TABLE index_scan_notices_v10 (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id TEXT NOT NULL REFERENCES index_scan_reports(root_id) ON DELETE CASCADE,
    kind INTEGER NOT NULL CHECK(kind IN (0,1,2,3)),
    path TEXT NOT NULL,
    message TEXT NOT NULL
);
INSERT INTO index_scan_notices_v10 SELECT * FROM index_scan_notices;
DROP TABLE index_scan_notices;
ALTER TABLE index_scan_notices_v10 RENAME TO index_scan_notices;
CREATE INDEX scan_notice_root ON index_scan_notices(root_id,sequence);
CREATE INDEX scan_notice_kind ON index_scan_notices(root_id,kind,sequence);
PRAGMA user_version=10;
