CREATE TABLE index_scan_reports (
    root_id TEXT PRIMARY KEY REFERENCES index_roots(id) ON DELETE CASCADE,
    scan_id INTEGER NOT NULL REFERENCES index_scans(id)
);
CREATE TABLE index_scan_notices (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id TEXT NOT NULL REFERENCES index_scan_reports(root_id) ON DELETE CASCADE,
    kind INTEGER NOT NULL CHECK(kind IN (0,1,2)),
    path TEXT NOT NULL,
    message TEXT NOT NULL
);
CREATE INDEX scan_notice_root ON index_scan_notices(root_id,sequence);
CREATE INDEX scan_notice_kind ON index_scan_notices(root_id,kind,sequence);
PRAGMA user_version=9;
