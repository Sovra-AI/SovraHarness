//! Derived vectors are scoped to a revocable destination and current source chunks.

use super::{LocalFileIndex, PASSAGES, db_error, get_root, passage};
use rusqlite::{Connection, OptionalExtension as _, params};
use sovra_core::file_index::{
    IndexError, IndexPassage, IndexQuery,
    semantic::{
        EMBEDDING_BATCH, EmbeddingInput, EmbeddingTarget, EmbeddingVector, SemanticIndex,
        SemanticSetting, SemanticStatus,
    },
};

const MAX_VECTORS: i64 = 20_000;
const CURRENT: &str = "SELECT DISTINCT vc.chunk_id FROM index_files f JOIN index_version_chunks vc ON vc.version_id=f.current_version WHERE f.root_id=?1 AND f.available=1";

fn bounded(db: &mut Connection) -> Result<(), IndexError> {
    let started = std::time::Instant::now();
    db.progress_handler(
        1000,
        Some(move || started.elapsed() > std::time::Duration::from_secs(5)),
    )
    .map_err(|e| db_error(&e))
}

fn sql_id(id: u64) -> Result<i64, IndexError> {
    i64::try_from(id).map_err(|_| IndexError("Invalid semantic generation.".into()))
}

fn approved(db: &Connection, setting: &SemanticSetting) -> Result<(), IndexError> {
    let target: Option<String> = db
        .query_row(
            "SELECT target FROM index_semantic_settings WHERE id=?1 AND root_id=?2",
            params![sql_id(setting.id)?, setting.root_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| db_error(&e))?;
    if target
        .as_deref()
        .and_then(|s| serde_json::from_str::<EmbeddingTarget>(s).ok())
        .as_ref()
        != Some(&setting.target)
    {
        return Err(IndexError(
            "Semantic search permission changed. Search again after choosing a model.".into(),
        ));
    }
    Ok(())
}

impl SemanticIndex for LocalFileIndex {
    fn settings(&self) -> Result<Vec<SemanticSetting>, IndexError> {
        let db = self.connect()?;
        let mut statement = db
            .prepare("SELECT id,root_id,target FROM index_semantic_settings ORDER BY id")
            .map_err(|e| db_error(&e))?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| db_error(&e))?;
        rows.map(|row| {
            let (id, root_id, json) = row.map_err(|e| db_error(&e))?;
            let id =
                u64::try_from(id).map_err(|_| IndexError("Invalid semantic generation.".into()))?;
            let target = serde_json::from_str(&json)
                .map_err(|_| IndexError("Saved embedding settings could not be read.".into()))?;
            Ok(SemanticSetting {
                id,
                root_id,
                target,
            })
        })
        .collect()
    }

    fn configure(&self, root_id: &str, target: Option<&EmbeddingTarget>) -> Result<(), IndexError> {
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        get_root(&tx, root_id)?;
        tx.execute(
            "DELETE FROM index_semantic_settings WHERE root_id=?1",
            [root_id],
        )
        .map_err(|e| db_error(&e))?;
        if let Some(target) = target {
            let json = serde_json::to_string(target)
                .map_err(|_| IndexError("Invalid embedding destination.".into()))?;
            tx.execute(
                "INSERT INTO index_semantic_settings(root_id,target) VALUES(?1,?2)",
                params![root_id, json],
            )
            .map_err(|e| db_error(&e))?;
        }
        tx.commit().map_err(|e| db_error(&e))
    }

    fn status(&self, setting: &SemanticSetting) -> Result<SemanticStatus, IndexError> {
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        approved(&tx, setting)?;
        let (total, embedded): (i64,i64) = tx.query_row(&format!("SELECT COUNT(*),COUNT(e.chunk_id) FROM ({CURRENT}) c LEFT JOIN index_embeddings e ON e.chunk_id=c.chunk_id AND e.setting_id=?2"), params![setting.root_id,sql_id(setting.id)?], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e| db_error(&e))?;
        Ok(SemanticStatus {
            setting: setting.clone(),
            total: u64::try_from(total).expect("SQL count is nonnegative"),
            embedded: u64::try_from(embedded).expect("SQL count is nonnegative"),
            message: None,
        })
    }

    fn pending(&self, setting: &SemanticSetting) -> Result<Vec<EmbeddingInput>, IndexError> {
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        approved(&tx, setting)?;
        tx.execute(
            &format!(
                "DELETE FROM index_embeddings WHERE setting_id=?2 AND chunk_id NOT IN ({CURRENT})"
            ),
            params![setting.root_id, sql_id(setting.id)?],
        )
        .map_err(|e| db_error(&e))?;
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM index_embeddings WHERE setting_id=?1",
                [sql_id(setting.id)?],
                |r| r.get(0),
            )
            .map_err(|e| db_error(&e))?;
        let limit = MAX_VECTORS
            .saturating_sub(count)
            .min(i64::try_from(EMBEDDING_BATCH).expect("batch fits i64"));
        let sql = format!(
            "SELECT c.id,c.body FROM ({CURRENT}) current JOIN index_chunks c ON c.id=current.chunk_id LEFT JOIN index_embeddings e ON e.chunk_id=c.id AND e.setting_id=?2 WHERE e.chunk_id IS NULL ORDER BY c.id LIMIT ?3"
        );
        let inputs = {
            let mut statement = tx.prepare(&sql).map_err(|e| db_error(&e))?;
            statement
                .query_map(
                    params![setting.root_id, sql_id(setting.id)?, limit.max(1)],
                    |r| {
                        Ok(EmbeddingInput {
                            chunk_id: r.get(0)?,
                            text: r.get(1)?,
                        })
                    },
                )
                .map_err(|e| db_error(&e))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| db_error(&e))?
        };
        tx.commit().map_err(|e| db_error(&e))?;
        if limit == 0 && !inputs.is_empty() {
            return Err(IndexError("Semantic indexing reached 20,000 unique passages for this folder. Keyword search still covers its indexed text.".into()));
        }
        Ok(inputs)
    }

    fn commit(
        &self,
        setting: &SemanticSetting,
        inputs: &[EmbeddingInput],
        vectors: &[EmbeddingVector],
    ) -> Result<(), IndexError> {
        if inputs.is_empty() || inputs.len() > EMBEDDING_BATCH || inputs.len() != vectors.len() {
            return Err(IndexError(
                "Embedding batch size does not match its source passages.".into(),
            ));
        }
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        approved(&tx, setting)?;
        let dimensions: Option<u16> = tx
            .query_row(
                "SELECT dimensions FROM index_semantic_settings WHERE id=?1",
                [sql_id(setting.id)?],
                |r| r.get(0),
            )
            .map_err(|e| db_error(&e))?;
        let dimensions = dimensions.unwrap_or(
            u16::try_from(vectors[0].values().len()).expect("validated dimensions fit u16"),
        );
        if vectors
            .iter()
            .any(|v| v.values().len() != usize::from(dimensions))
        {
            return Err(IndexError("The embedding dimensions changed. Re-enable semantic search to rebuild this folder.".into()));
        }
        tx.execute(
            "UPDATE index_semantic_settings SET dimensions=?2 WHERE id=?1",
            params![sql_id(setting.id)?, dimensions],
        )
        .map_err(|e| db_error(&e))?;
        for (input, vector) in inputs.iter().zip(vectors) {
            let bytes: Vec<u8> = vector
                .values()
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            tx.execute(&format!("INSERT OR IGNORE INTO index_embeddings(setting_id,chunk_id,vector) SELECT ?2,c.id,?4 FROM index_chunks c WHERE c.id=?3 AND c.body=?5 AND c.id IN ({CURRENT})"), params![setting.root_id,sql_id(setting.id)?,input.chunk_id,bytes,input.text]).map_err(|e| db_error(&e))?;
        }
        tx.commit().map_err(|e| db_error(&e))
    }

    fn nearest(
        &self,
        setting: &SemanticSetting,
        query: &IndexQuery,
        vector: &EmbeddingVector,
    ) -> Result<Vec<IndexPassage>, IndexError> {
        query.validate()?;
        if !query.roots.is_empty() && !query.roots.contains(&setting.root_id) {
            return Ok(vec![]);
        }
        let mut db = self.connect()?;
        bounded(&mut db)?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        approved(&tx, setting)?;
        let prefix = query.path_prefix.as_deref().unwrap_or("");
        let mut best = Vec::new();
        let started = std::time::Instant::now();
        {
            let mut statement = tx.prepare("SELECT f.id,f.current_version,vc.ordinal,e.vector FROM index_files f JOIN index_version_chunks vc ON vc.version_id=f.current_version JOIN index_embeddings e ON e.chunk_id=vc.chunk_id AND e.setting_id=?2 WHERE f.root_id=?1 AND f.available=1 AND substr(f.path,1,length(?3))=?3 ORDER BY f.id,vc.ordinal").map_err(|e| db_error(&e))?;
            let rows = statement
                .query_map(params![setting.root_id, sql_id(setting.id)?, prefix], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, u32>(2)?,
                        r.get::<_, Vec<u8>>(3)?,
                    ))
                })
                .map_err(|e| db_error(&e))?;
            for row in rows {
                if started.elapsed() > std::time::Duration::from_secs(5) {
                    return Err(IndexError(
                        "Semantic ranking took too long. Search a smaller folder.".into(),
                    ));
                }
                let (file, version, ordinal, bytes) = row.map_err(|e| db_error(&e))?;
                if bytes.len() % 4 != 0 {
                    return Err(IndexError(
                        "An embedding is corrupt. Re-enable semantic search to rebuild it.".into(),
                    ));
                }
                let values = bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect();
                let score = vector.cosine(&EmbeddingVector::new(values)?)?;
                best.push((score, file, version, ordinal));
                best.sort_by(|a, b| {
                    b.0.total_cmp(&a.0)
                        .then_with(|| (&a.1, a.3).cmp(&(&b.1, b.3)))
                });
                best.truncate(query.limit);
            }
        }
        let sql = format!(
            "{PASSAGES} WHERE f.root_id=?1 AND f.id=?2 AND v.id=?3 AND vc.ordinal=?4 AND f.available=1 AND f.current_version=v.id"
        );
        best.into_iter()
            .map(|(_, file, version, ordinal)| {
                tx.query_row(
                    &sql,
                    params![setting.root_id, file, version, ordinal],
                    passage,
                )
                .map_err(|e| db_error(&e))
            })
            .collect()
    }
}
