//! Revision-checked withdrawal of an entire immutable correction history.

use std::collections::HashSet;

use sovra_core::{FactId, MemoryError, MemoryStore as _, Timestamp};
use surrealdb::types::{RecordId, ToSql as _};

use super::{SurrealStore, backend, parse_id};

const STALE: &str = "memory_removal_revision_changed";

impl SurrealStore {
    pub(super) async fn remove_history(
        &self,
        id: &FactId,
        at: Timestamp,
    ) -> Result<(), MemoryError> {
        for _ in 0..3 {
            let revision = self.memory_revision().await?;
            self.exact_fact(id).await?;
            let family = self.correction_family(parse_id(&id.0, "fact")?).await?;
            match self.commit_removal(family, revision, at).await {
                Err(MemoryError::Backend(message)) if message.contains(STALE) => {}
                result => return result,
            }
        }
        Err(MemoryError::Backend(
            "This detail changed during removal. Please retry removing it.".into(),
        ))
    }

    async fn correction_family(&self, anchor: RecordId) -> Result<Vec<RecordId>, MemoryError> {
        let mut seen = HashSet::from([anchor.to_sql()]);
        let mut family = vec![anchor];
        let mut offset = 0;
        // Follow indexed edges in bounded reads outside the write transaction.
        // There is no depth cutoff that could silently leave older versions active.
        while offset < family.len() {
            let end = (offset + 64).min(family.len());
            let adjacent: Vec<Vec<RecordId>> = self.query(
                "SELECT VALUE array::union(->supersedes->fact.id, <-supersedes<-fact.id) FROM $frontier;",
                ("frontier", family[offset..end].to_vec()),
            ).await?.take(0).map_err(|error| backend(&error))?;
            offset = end;
            for id in adjacent.into_iter().flatten() {
                if seen.insert(id.to_sql()) {
                    family.push(id);
                }
            }
        }
        Ok(family)
    }

    async fn commit_removal(
        &self,
        family: Vec<RecordId>,
        revision: u64,
        at: Timestamp,
    ) -> Result<(), MemoryError> {
        let statement = ["BEGIN TRANSACTION;
             IF meta:memory_facts.revision != $revision { THROW 'memory_removal_revision_changed'; };
             UPDATE meta:memory_facts SET revision += 1;
             UPDATE meta:remembering SET revision += 1;
             LET $sources = array::union(
                 (SELECT VALUE out FROM derived_from WHERE in IN $family),
                 (SELECT VALUE episode FROM memory_job WHERE fact_ids CONTAINSANY $family));
             ", super::sources::SOURCE_VERSION_FAMILY, "
             UPDATE $family SET stopped_at = stopped_at ?? $at,
                 invalid_at = invalid_at ?? $at, expired_at = expired_at ?? $at;
             UPDATE $version_sources SET raw_withdrawn_at = raw_withdrawn_at ?? $at;
             UPDATE memory_job SET state = 'cancelled', token = NONE
                 WHERE episode IN $version_sources AND state IN ['pending', 'processing', 'failed'];
             COMMIT TRANSACTION;"].concat();
        self.query(
            &statement,
            ("family", family, "revision", revision, "at", at.millis()),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sovra_core::{MemoryCorrection, MemoryListQuery, MemoryNote};

    #[tokio::test]
    async fn a_correction_during_family_collection_rejects_the_entire_stale_removal() {
        let store = SurrealStore::open(crate::StoreConfig::in_memory())
            .await
            .unwrap();
        let first = store
            .remember_note(MemoryNote {
                text: "Parcel destination is Oslo".into(),
            })
            .await
            .unwrap();
        let revision = store.memory_revision().await.unwrap();
        let family = store
            .correction_family(parse_id(&first.0, "fact").unwrap())
            .await
            .unwrap();
        let second = store
            .correct_memory(MemoryCorrection {
                target: first.clone(),
                text: "Parcel destination is Tallinn".into(),
            })
            .await
            .unwrap();
        let error = store
            .commit_removal(family, revision, Timestamp(i64::MAX))
            .await
            .unwrap_err();
        assert!(error.to_string().contains(STALE), "{error}");
        for id in [&first, &second] {
            let detail = store.memory_fact(id).await.unwrap();
            assert!(
                detail.stopped_at.is_none(),
                "a stale removal must make no partial writes"
            );
            assert!(
                store
                    .review_memory_source(&detail.source.unwrap())
                    .await
                    .unwrap()
                    .metadata
                    .revoked_at
                    .is_none()
            );
        }
        store
            .remove_memory(&first, Timestamp(i64::MAX))
            .await
            .unwrap();
        assert_eq!(
            store
                .list_memories(MemoryListQuery::default())
                .await
                .unwrap()
                .facts,
            Vec::new()
        );
        assert!(
            store
                .memory_fact(&second)
                .await
                .unwrap()
                .stopped_at
                .is_some()
        );
    }
}
