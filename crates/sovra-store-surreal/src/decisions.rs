//! Bounded decision summaries and atomic, revision-checked record persistence.

use sovra_core::MemoryError;
use sovra_core::decisions::{DECISION_PAGE_SIZE, DecisionRecord, DecisionSummary};
use surrealdb::types::{RecordId, SurrealValue};

use crate::{SurrealStore, backend, entity_key};

#[derive(SurrealValue)]
struct DecisionRow {
    decision_id: String,
    revision: i64,
    summary_json: String,
    record_json: String,
}

impl SurrealStore {
    pub(crate) async fn decision_list(
        &self,
        offset: usize,
    ) -> Result<Vec<DecisionSummary>, MemoryError> {
        let rows: Vec<String> = self.query(
            "SELECT VALUE summary_json FROM decision ORDER BY decision_id LIMIT $limit START $offset;",
            ("limit", i64::try_from(DECISION_PAGE_SIZE + 1).unwrap_or(101), "offset", i64::try_from(offset).map_err(|_| MemoryError::InvalidInput("Decision page is too large.".into()))?),
        ).await?.take(0).map_err(|error| backend(&error))?;
        rows.into_iter()
            .map(|json| {
                serde_json::from_str(&json).map_err(|error| {
                    MemoryError::Backend(format!("Read decision summary: {error}"))
                })
            })
            .collect()
    }

    pub(crate) async fn decision_get(
        &self,
        id: &str,
    ) -> Result<Option<DecisionRecord>, MemoryError> {
        let json: Option<String> = self
            .query(
                "SELECT VALUE record_json FROM ONLY $record;",
                ("record", RecordId::new("decision", entity_key(id))),
            )
            .await?
            .take(0)
            .map_err(|error| backend(&error))?;
        json.map(|json| {
            serde_json::from_str(&json)
                .map_err(|error| MemoryError::Backend(format!("Read decision: {error}")))
        })
        .transpose()
    }

    pub(crate) async fn decision_save(
        &self,
        record: &DecisionRecord,
        expected: u64,
    ) -> Result<(), MemoryError> {
        let invalid = |message: String| MemoryError::InvalidInput(message);
        if expected.checked_add(1) != Some(record.revision) {
            return Err(invalid("Decision revision is invalid.".into()));
        }
        let summary = record
            .summary()
            .map_err(|error| invalid(error.to_string()))?;
        record
            .current()
            .map_err(|error| invalid(error.to_string()))?
            .draft
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        let row = DecisionRow {
            decision_id: record.id.clone(),
            revision: i64::try_from(record.revision)
                .map_err(|_| invalid("Decision revision is too large.".into()))?,
            summary_json: serde_json::to_string(&summary)
                .map_err(|error| MemoryError::Backend(format!("Save decision summary: {error}")))?,
            record_json: serde_json::to_string(record)
                .map_err(|error| MemoryError::Backend(format!("Save decision: {error}")))?,
        };
        self.query(
            "BEGIN TRANSACTION;
             LET $current = (SELECT VALUE revision FROM ONLY $record);
             IF ($expected = 0 AND $current != NONE) OR ($expected > 0 AND $current != $expected) { THROW 'This decision changed. Reopen it before trying again.'; };
             UPSERT $record CONTENT $data;
             COMMIT TRANSACTION;",
            ("record", RecordId::new("decision", entity_key(&record.id)), "expected", i64::try_from(expected).map_err(|_| invalid("Decision revision is too large.".into()))?, "data", row),
        ).await?;
        Ok(())
    }
}
