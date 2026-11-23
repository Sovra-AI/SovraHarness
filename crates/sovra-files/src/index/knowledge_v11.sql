CREATE INDEX IF NOT EXISTS knowledge_predicate_time ON index_knowledge_relations(kind,created DESC,id);
CREATE INDEX IF NOT EXISTS knowledge_subject_predicate ON index_knowledge_relations(subject,kind,created DESC,id);
CREATE INDEX IF NOT EXISTS knowledge_object_predicate ON index_knowledge_relations(object,kind,created DESC,id);
PRAGMA user_version=11;
