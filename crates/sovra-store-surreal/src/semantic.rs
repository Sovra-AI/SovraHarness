//! Isolated vector indexes per consent generation; facts and older indexes stay intact.

use sovra_core::{
    FactId, MEMORY_EMBEDDING_BATCH, MEMORY_SEMANTIC_CANDIDATES, MEMORY_SIMILARITY_FLOOR,
    MemoryEmbeddingInput, MemoryError, MemoryPolicy, MemorySearch, MemorySemanticHit,
    MemorySemanticSetting, MemorySemanticState, MemoryStore as _, MemoryVector, Timestamp,
    file_index::semantic::{EmbeddingVector, MAX_EMBEDDING_TEXT_BYTES},
    models::{ModelDestination, ProviderKind},
    validate_memory_vectors,
};
use surrealdb::types::{RecordId, SurrealValue, ToSql as _};

use super::{
    SurrealStore, backend, parse_id,
    schema::{self, semantic_table_name as table},
};

fn destination(target: &ModelDestination) -> Result<String, MemoryError> {
    let json = serde_json::to_string(target)
        .map_err(|error| MemoryError::InvalidInput(error.to_string()))?;
    if json.len() > 8192
        || target.key.model_id.trim().is_empty()
        || target.key.connection_id.trim().is_empty()
        || target.base_url.trim().is_empty()
        || !matches!(
            target.provider,
            ProviderKind::Ollama | ProviderKind::OpenAiCompatible
        )
    {
        return Err(MemoryError::InvalidInput(
            "Choose a supported search helper in Models.".into(),
        ));
    }
    Ok(json)
}

impl SurrealStore {
    pub(super) async fn semantic_state(&self) -> Result<MemorySemanticState, MemoryError> {
        let row: Option<StateRow> = self
            .query(
                "SELECT generation, target_json, dimension FROM ONLY meta:memory_semantic;",
                surrealdb::types::Object::default(),
            )
            .await?
            .take(0)
            .map_err(|error| backend(&error))?;
        let row =
            row.ok_or_else(|| MemoryError::Backend("Missing semantic memory metadata.".into()))?;
        let target = row
            .target_json
            .map(|json| {
                serde_json::from_str(&json).map_err(|error| {
                    MemoryError::Backend(format!("Invalid semantic memory destination: {error}"))
                })
            })
            .transpose()?;
        Ok(MemorySemanticState {
            generation: row.generation,
            setting: target.map(|target| MemorySemanticSetting {
                generation: row.generation,
                target,
                dimension: row.dimension,
            }),
        })
    }

    pub(super) async fn configure_semantics(
        &self,
        target: Option<ModelDestination>,
    ) -> Result<MemorySemanticState, MemoryError> {
        let json = target.as_ref().map(destination).transpose()?;
        let previous = self.semantic_state().await?;
        let generation = previous
            .generation
            .checked_add(1)
            .filter(|value| *value <= i64::MAX.unsigned_abs())
            .ok_or_else(|| {
                MemoryError::InvalidInput("Semantic memory generation exhausted.".into())
            })?;
        let schema = target
            .as_ref()
            .map_or_else(String::new, |_| schema::semantic_table(generation));
        self.query(&format!("BEGIN TRANSACTION;
            IF (SELECT VALUE generation FROM ONLY meta:memory_semantic) != $previous {{ THROW 'semantic memory selection changed'; }};
            UPDATE meta:memory_semantic SET generation = $generation, target_json = $target, dimension = NONE, revision += 1;
            {schema}
            COMMIT TRANSACTION;"), ("previous", previous.generation, "generation", generation, "target", json)).await?;
        Ok(MemorySemanticState {
            generation,
            setting: target.map(|target| MemorySemanticSetting {
                generation,
                target,
                dimension: None,
            }),
        })
    }

    async fn active_semantics(
        &self,
        setting: &MemorySemanticSetting,
    ) -> Result<MemorySemanticSetting, MemoryError> {
        self.semantic_state()
            .await?
            .setting
            .filter(|current| {
                current.generation == setting.generation && current.target == setting.target
            })
            .ok_or_else(|| MemoryError::InvalidInput("Semantic memory selection changed.".into()))
    }

    pub(super) async fn embedding_inputs(
        &self,
        setting: &MemorySemanticSetting,
        ids: &[FactId],
    ) -> Result<Vec<MemoryEmbeddingInput>, MemoryError> {
        self.active_semantics(setting).await?;
        let policy = self.memory_policy().await?;
        if !policy.enabled || ids.len() > MEMORY_EMBEDDING_BATCH {
            return Err(MemoryError::InvalidInput(
                "Remembering is paused or the embedding page is too large.".into(),
            ));
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<_> = ids
            .iter()
            .map(|id| parse_id(&id.0, "fact"))
            .collect::<Result<_, _>>()?;
        let indexed: Vec<RecordId> = self
            .query(
                &format!(
                    "SELECT VALUE fact FROM {} WHERE fact IN $ids;",
                    table(setting.generation)
                ),
                ("ids", ids.clone()),
            )
            .await?
            .take(0)
            .map_err(|error| backend(&error))?;
        let pending: Vec<_> = ids.into_iter().filter(|id| !indexed.contains(id)).collect();
        if pending.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<InputRow> = self.query(
            "SELECT id, content FROM $ids WHERE stopped_at IS NONE AND (->derived_from->episode.kind)[0] IN ['user_message', 'note'];",
            ("ids", pending),
        ).await?.take(0).map_err(|error| backend(&error))?;
        self.active_semantics(setting).await?;
        if self.memory_policy().await? != policy {
            return Err(MemoryError::InvalidInput(
                "Remembering permission changed.".into(),
            ));
        }
        Ok(rows
            .into_iter()
            .filter(|row| {
                !row.content.trim().is_empty() && row.content.len() <= MAX_EMBEDDING_TEXT_BYTES
            })
            .map(|row| MemoryEmbeddingInput {
                fact_id: FactId(row.id.to_sql()),
                text: row.content,
            })
            .collect())
    }

    pub(super) async fn save_vectors(
        &self,
        setting: &MemorySemanticSetting,
        policy: MemoryPolicy,
        vectors: Vec<MemoryVector>,
    ) -> Result<(), MemoryError> {
        let dimension = validate_memory_vectors(&vectors)?;
        let current = self.active_semantics(setting).await?;
        if !policy.enabled || current.dimension.is_some_and(|known| known != dimension) {
            return Err(MemoryError::InvalidInput(
                "Remembering is paused or the search helper changed vector dimensions.".into(),
            ));
        }
        let table = table(setting.generation);
        let index = if current.dimension.is_none() {
            schema::semantic_index(setting.generation, dimension)
        } else {
            String::new()
        };
        let rows: Vec<_> = vectors
            .into_iter()
            .map(|item| {
                Ok(VectorRow {
                    id: RecordId::new(table.clone(), item.fact_id.0.clone()),
                    fact: parse_id(&item.fact_id.0, "fact")?,
                    vector: item.vector.values().to_vec(),
                })
            })
            .collect::<Result<_, MemoryError>>()?;
        self.query(&format!("BEGIN TRANSACTION;
            LET $selection = (SELECT * FROM ONLY meta:memory_semantic);
            LET $policy = (SELECT * FROM ONLY meta:remembering);
            IF $selection.generation != $generation OR $selection.target_json != $target OR !$policy.enabled OR $policy.generation != $policy_generation {{ THROW 'semantic memory permission changed'; }};
            IF $selection.dimension != NONE AND $selection.dimension != $dimension {{ THROW 'semantic memory dimensions changed'; }};
            UPDATE meta:memory_semantic SET dimension = $dimension, revision += 1;
            UPDATE meta:remembering SET revision += 1;
            {index}
            FOR $row IN $rows {{
                LET $fact = (SELECT * FROM ONLY $row.fact);
                IF $fact = NONE OR $fact.stopped_at != NONE {{ THROW 'memory was withdrawn or missing'; }};
                IF (SELECT VALUE (->derived_from->episode.kind)[0] FROM ONLY $row.fact) NOT IN ['user_message', 'note'] {{ THROW 'memory has no user evidence'; }};
                LET $existing = (SELECT VALUE vector FROM ONLY $row.id);
                IF $existing != NONE AND $existing != $row.vector {{ THROW 'memory vector already recorded'; }};
                UPSERT $row.id CONTENT {{ fact: $row.fact, vector: $row.vector }};
            }};
            COMMIT TRANSACTION;"), VectorBindings {
                generation: setting.generation, target: destination(&setting.target)?, policy_generation: policy.generation, dimension, rows,
            }).await?;
        Ok(())
    }

    pub(super) async fn nearest_memories(
        &self,
        setting: &MemorySemanticSetting,
        vector: &EmbeddingVector,
        search: MemorySearch,
        now: Timestamp,
    ) -> Result<Vec<MemorySemanticHit>, MemoryError> {
        search.validate()?;
        let current = self.active_semantics(setting).await?;
        let policy = self.memory_policy().await?;
        if !policy.enabled {
            return Ok(Vec::new());
        }
        let Some(dimension) = current.dimension else {
            return Ok(Vec::new());
        };
        if vector.values().len() != dimension {
            return Err(MemoryError::InvalidInput(
                "The search helper changed vector dimensions.".into(),
            ));
        }
        let historical = search.valid_at.is_some() || search.known_at.is_some();
        let rows: Vec<HitRow> = self.query(&format!("SELECT fact, vector::distance::knn() AS distance FROM {} WHERE vector <|96,120|> $vector
            AND fact.stopped_at IS NONE
            AND ($historical OR (fact.invalid_at IS NONE AND fact.expired_at IS NONE))
            AND (fact.valid_at IS NONE OR fact.valid_at <= $valid)
            AND (fact.invalid_at IS NONE OR fact.invalid_at > $valid OR ($has_known AND fact.expired_at > $known))
            AND fact.created_at <= $known
            AND (!$has_known OR fact.expired_at IS NONE OR fact.expired_at > $known)
            AND ($entity = NONE OR $entity IN fact.entities)
            ORDER BY distance ASC LIMIT 96;", table(setting.generation)), SearchBindings {
                vector: vector.values().to_vec(), historical,
                valid: search.valid_at.or(search.known_at).unwrap_or(now).millis(),
                known: search.known_at.unwrap_or(now).millis(), has_known: search.known_at.is_some(), entity: search.entity.clone(),
            }).await?.take(0).map_err(|error| backend(&error))?;
        let rows: Vec<_> = rows
            .into_iter()
            .filter(|row| row.distance.is_finite() && 1. - row.distance >= MEMORY_SIMILARITY_FLOOR)
            .take(MEMORY_SEMANTIC_CANDIDATES)
            .collect();
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let facts = self
            .fact_leg(
                &format!("SELECT {} FROM $ids;", super::facts::FIELDS),
                (
                    "ids",
                    rows.iter().map(|row| row.fact.clone()).collect::<Vec<_>>(),
                ),
            )
            .await?;
        let mut hits = Vec::new();
        for row in rows {
            if let Some(fact) = facts
                .iter()
                .find(|fact| fact.id.0 == row.fact.to_sql())
                .filter(|fact| search.accepts(fact, now))
            {
                hits.push(MemorySemanticHit {
                    fact: search.as_known(fact.clone()),
                    similarity: (1. - row.distance).clamp(-1., 1.),
                });
            }
        }
        self.active_semantics(setting).await?;
        if self.memory_policy().await? != policy {
            return Ok(Vec::new());
        }
        Ok(hits)
    }
}

#[derive(Debug, SurrealValue)]
struct StateRow {
    generation: u64,
    target_json: Option<String>,
    dimension: Option<usize>,
}
#[derive(Debug, SurrealValue)]
struct InputRow {
    id: RecordId,
    content: String,
}
#[derive(Debug, SurrealValue)]
struct VectorRow {
    id: RecordId,
    fact: RecordId,
    vector: Vec<f32>,
}
#[derive(Debug, SurrealValue)]
struct VectorBindings {
    generation: u64,
    target: String,
    policy_generation: u64,
    dimension: usize,
    rows: Vec<VectorRow>,
}
#[derive(Debug, SurrealValue)]
struct HitRow {
    fact: RecordId,
    distance: f64,
}
#[derive(Debug, SurrealValue)]
struct SearchBindings {
    vector: Vec<f32>,
    historical: bool,
    valid: i64,
    known: i64,
    has_known: bool,
    entity: Option<String>,
}
