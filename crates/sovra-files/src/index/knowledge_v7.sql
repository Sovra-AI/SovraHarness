CREATE TABLE index_knowledge_proposals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    fingerprint TEXT NOT NULL UNIQUE, state TEXT NOT NULL,
    created INTEGER NOT NULL, details TEXT NOT NULL
);
CREATE INDEX knowledge_proposal_state ON index_knowledge_proposals(state,id);
CREATE TABLE index_knowledge_evidence (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    proposal_id INTEGER NOT NULL REFERENCES index_knowledge_proposals(id),
    ordinal INTEGER NOT NULL,
    root_id TEXT REFERENCES index_roots(id) ON DELETE SET NULL,
    file_id TEXT REFERENCES index_files(id) ON DELETE SET NULL,
    version_id TEXT REFERENCES index_versions(id) ON DELETE SET NULL,
    snapshot TEXT NOT NULL, UNIQUE(proposal_id,ordinal)
);
CREATE INDEX knowledge_evidence_root ON index_knowledge_evidence(root_id);
CREATE INDEX knowledge_evidence_file ON index_knowledge_evidence(file_id);
CREATE INDEX knowledge_evidence_version ON index_knowledge_evidence(version_id);
CREATE TABLE index_knowledge_entities (id TEXT PRIMARY KEY, details TEXT NOT NULL);
CREATE TABLE index_knowledge_entity_sources (
    entity_id TEXT NOT NULL REFERENCES index_knowledge_entities(id),
    evidence_id INTEGER NOT NULL REFERENCES index_knowledge_evidence(id),
    PRIMARY KEY(entity_id,evidence_id)
);
CREATE INDEX knowledge_entity_source ON index_knowledge_entity_sources(evidence_id);
CREATE TABLE index_knowledge_relations (
    id TEXT PRIMARY KEY,
    proposal_id INTEGER NOT NULL REFERENCES index_knowledge_proposals(id),
    subject TEXT REFERENCES index_knowledge_entities(id),
    object TEXT REFERENCES index_knowledge_entities(id),
    kind TEXT NOT NULL, created INTEGER NOT NULL, expired INTEGER, details TEXT NOT NULL
);
CREATE INDEX knowledge_relation_subject ON index_knowledge_relations(subject,created);
CREATE INDEX knowledge_relation_object ON index_knowledge_relations(object,created);
CREATE INDEX knowledge_relation_proposal ON index_knowledge_relations(proposal_id);
CREATE INDEX knowledge_relation_time ON index_knowledge_relations(created,id);
CREATE TABLE index_knowledge_relation_sources (
    relation_id TEXT NOT NULL REFERENCES index_knowledge_relations(id),
    evidence_id INTEGER NOT NULL REFERENCES index_knowledge_evidence(id),
    PRIMARY KEY(relation_id,evidence_id)
);
CREATE INDEX knowledge_relation_source ON index_knowledge_relation_sources(evidence_id);
CREATE TRIGGER knowledge_revoke_pending BEFORE DELETE ON index_roots BEGIN
    UPDATE index_knowledge_proposals SET state='"rejected"'
    WHERE state='"pending"' AND EXISTS (
        SELECT 1 FROM json_each(details,'$.required_roots') WHERE value=old.id
    );
END;
PRAGMA user_version=7;
