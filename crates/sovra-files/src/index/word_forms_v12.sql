-- Keep the original literal index for identifiers and inventory queries. The
-- second FTS index stores postings only and reuses the exact same chunk bodies.
CREATE VIRTUAL TABLE IF NOT EXISTS index_word_forms USING fts5(
    body, content='index_chunks', content_rowid='rowid', tokenize='porter unicode61'
);
CREATE TRIGGER IF NOT EXISTS word_forms_insert AFTER INSERT ON index_chunks BEGIN
    INSERT INTO index_word_forms(rowid,body) VALUES (new.rowid,new.body);
END;
CREATE TRIGGER IF NOT EXISTS word_forms_delete AFTER DELETE ON index_chunks BEGIN
    INSERT INTO index_word_forms(index_word_forms,rowid,body) VALUES ('delete',old.rowid,old.body);
END;
INSERT INTO index_word_forms(index_word_forms) VALUES('rebuild');
PRAGMA user_version=12;
