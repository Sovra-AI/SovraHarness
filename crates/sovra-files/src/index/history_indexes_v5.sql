CREATE INDEX scan_root ON index_scans(root_id);
CREATE INDEX version_file ON index_versions(file_id);
CREATE INDEX version_generation ON index_versions(generated_by);
CREATE INDEX observation_scan ON index_observations(scan_id);
CREATE INDEX observation_predecessor ON index_observations(replaces);
PRAGMA user_version=5;

