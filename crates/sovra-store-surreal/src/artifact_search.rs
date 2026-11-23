//! BM25 selects current artifact IDs before copying any content. The body
//! returned for the bounded matches excludes earlier versions and JSON history.
use super::{SurrealStore, backend};
use sovra_core::{
    MemoryError, Timestamp,
    knowledge_analysis::{Excerpt, Language, MAX_ANALYSIS_BYTES},
};
use surrealdb::types::{Object, SurrealValue};

#[derive(SurrealValue)]
struct SearchRow {
    artifact_id: String,
    updated_at: i64,
    search_title: String,
    searchable: String,
}

impl SurrealStore {
    pub(super) async fn artifact_excerpts(
        &self,
        query: &str,
        language: Language,
    ) -> Result<Vec<Excerpt>, MemoryError> {
        if query.trim().is_empty() || query.len() > 2048 {
            return Err(MemoryError::InvalidInput(
                "Enter up to 2,048 bytes of search terms.".into(),
            ));
        }
        let index = if language == Language::English {
            "artifact_text".into()
        } else {
            let lang = language.name();
            let index = format!("artifact_stems_{lang}");
            self.query(&format!("DEFINE ANALYZER IF NOT EXISTS words_{lang} TOKENIZERS blank,class FILTERS lowercase,snowball({lang}); DEFINE INDEX IF NOT EXISTS {index} ON artifact FIELDS searchable FULLTEXT ANALYZER words_{lang} BM25;"),Object::default()).await?;
            index
        };
        let literal = query
            .chars()
            .any(|c| c.is_numeric() || matches!(c, '-' | '_' | '/' | ':' | '\\'));
        let rows:Vec<SearchRow>=self.query(&format!("SELECT artifact_id, updated_at, search_title, searchable, search::score(0) AS rank FROM artifact WITH INDEX {index} WHERE deleted_at IS NONE AND searchable @0@ $query AND (!$literal OR string::contains(string::lowercase(searchable),string::lowercase($query))) ORDER BY rank DESC, updated_at DESC, artifact_id LIMIT 6;"),("query",query.to_owned(),"literal",literal)).await?.take(0).map_err(|e|backend(&e))?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let content = row
                    .searchable
                    .strip_prefix(&row.search_title)
                    .and_then(|rest| rest.strip_prefix('\n'))
                    .unwrap_or(&row.searchable);
                let range = language.excerpt_range(content, query, MAX_ANALYSIS_BYTES);
                Excerpt::artifact_range(
                    row.artifact_id,
                    Timestamp(row.updated_at),
                    row.search_title.clone(),
                    content,
                    range,
                )
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sovra_core::{
        MemoryStore,
        artifacts::{Artifact, ArtifactDraft, ArtifactKind, ArtifactSource},
    };

    #[tokio::test]
    async fn migrates_old_artifacts_and_keeps_only_the_current_version_searchable() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let mut artifact = Artifact::from_draft(
            "work-1".into(),
            ArtifactDraft {
                title: "Field survey".into(),
                kind: ArtifactKind::Note,
                language: String::new(),
                content: "Les maisons. Site-778.".into(),
                source: ArtifactSource::default(),
            },
            Timestamp(12),
        );
        store.save_artifact(&artifact).await.unwrap();
        store
            .query(
                "UPDATE artifact UNSET searchable,search_title; UPDATE meta:schema SET version=13;",
                Object::default(),
            )
            .await
            .unwrap();
        store.apply_schema().await.unwrap();
        let hits = store
            .artifact_excerpts("maison", Language::French)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].wording, artifact.content);
        assert_eq!(
            store
                .artifact_excerpts("Site-778", Language::English)
                .await
                .unwrap()
                .len(),
            1
        );
        artifact.edit("Changed survey", "Oak reading", Timestamp(14));
        store.save_artifact(&artifact).await.unwrap();
        assert_eq!(
            store
                .artifact_excerpts("maison", Language::French)
                .await
                .unwrap(),
            Vec::new()
        );
        store
            .delete_artifact(&artifact.id, Timestamp(16))
            .await
            .unwrap();
        assert_eq!(
            store
                .artifact_excerpts("Oak", Language::English)
                .await
                .unwrap(),
            Vec::new()
        );
    }
    #[tokio::test]
    async fn personal_language_search_honors_existing_permissions_and_withdrawal() {
        use sovra_core::{EpisodeKind, MemoryInput, MemorySearch, NewEpisode};
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let id = store
            .enqueue_memory(MemoryInput {
                key: "lang-1".into(),
                episode: NewEpisode {
                    session_id: "garden".into(),
                    kind: EpisodeKind::UserMessage,
                    content: "Les maisons sont dans la vallée.".into(),
                    occurred_at: Timestamp(1),
                },
            })
            .await
            .unwrap();
        let search = MemorySearch {
            text: "maison".into(),
            ..Default::default()
        };
        let hits = store
            .search_source_language(search.clone(), Timestamp(10), Language::French)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_id, id);
        store.stop_memory_source(&id, Timestamp(11)).await.unwrap();
        assert_eq!(
            store
                .search_source_language(search, Timestamp(12), Language::French)
                .await
                .unwrap(),
            Vec::new()
        );
    }
}
