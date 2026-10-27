//! Explicit embedding destinations, validated vectors and deterministic rank fusion.

use serde::{Deserialize, Serialize};

use super::{IndexError, IndexPassage, IndexQuery};
pub use crate::models::ModelDestination as EmbeddingTarget;

pub const EMBEDDING_BATCH: usize = 8;
pub const MAX_EMBEDDING_DIMENSIONS: usize = 4096;
pub const MAX_EMBEDDING_TEXT_BYTES: usize = 8196;

pub fn embedding_targets(state: &crate::models::ModelsState, query: &str) -> Vec<EmbeddingTarget> {
    crate::models::model_destinations(state, query, crate::models::ModelWork::Embeddings)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSetting {
    /// Monotonic consent generation; disabling/re-enabling never resurrects work.
    pub id: u64,
    pub root_id: String,
    pub target: EmbeddingTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticStatus {
    pub setting: SemanticSetting,
    pub embedded: u64,
    pub total: u64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct SearchReport {
    pub passages: Vec<IndexPassage>,
    pub notices: Vec<String>,
}

/// Text is intentionally omitted from Debug output.
#[derive(Clone)]
pub struct EmbeddingInput {
    pub chunk_id: String,
    pub text: String,
}

impl std::fmt::Debug for EmbeddingInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingInput")
            .field("chunk_id", &self.chunk_id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq)]
pub struct EmbeddingVector(Vec<f32>);

impl std::fmt::Debug for EmbeddingVector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingVector")
            .field("dimensions", &self.0.len())
            .finish()
    }
}

impl EmbeddingVector {
    pub fn new(values: Vec<f32>) -> Result<Self, IndexError> {
        if values.is_empty()
            || values.len() > MAX_EMBEDDING_DIMENSIONS
            || values.iter().any(|v| !v.is_finite())
        {
            return Err(IndexError(
                "The embedding must contain 1–4,096 finite values.".into(),
            ));
        }
        let scale = values.iter().map(|v| v.abs()).fold(0.0_f32, f32::max);
        if scale == 0. {
            return Err(IndexError(
                "The embedding server returned a zero vector.".into(),
            ));
        }
        // Scaling before summing squares prevents overflow for finite f32 inputs.
        let scaled: Vec<_> = values.into_iter().map(|v| v / scale).collect();
        let norm = scaled.iter().map(|v| v * v).sum::<f32>().sqrt();
        Ok(Self(scaled.into_iter().map(|v| v / norm).collect()))
    }

    pub fn values(&self) -> &[f32] {
        &self.0
    }

    pub fn cosine(&self, other: &Self) -> Result<f64, IndexError> {
        if self.0.len() != other.0.len() {
            return Err(IndexError("The embedding dimensions changed. Re-enable semantic search to rebuild this folder.".into()));
        }
        Ok(self
            .0
            .iter()
            .zip(&other.0)
            .map(|(a, b)| f64::from(*a) * f64::from(*b))
            .sum::<f64>()
            .clamp(-1., 1.))
    }
}

#[async_trait::async_trait]
pub trait TextEmbedder: Send + Sync + std::fmt::Debug {
    async fn embed(&self, input: &[String]) -> Result<Vec<EmbeddingVector>, IndexError>;
}

/// Optional capability of the local index. All operations run on blocking workers.
pub trait SemanticIndex: Send + Sync {
    fn settings(&self) -> Result<Vec<SemanticSetting>, IndexError>;
    fn configure(&self, root_id: &str, target: Option<&EmbeddingTarget>) -> Result<(), IndexError>;
    fn status(&self, setting: &SemanticSetting) -> Result<SemanticStatus, IndexError>;
    fn pending(&self, setting: &SemanticSetting) -> Result<Vec<EmbeddingInput>, IndexError>;
    fn commit(
        &self,
        setting: &SemanticSetting,
        inputs: &[EmbeddingInput],
        vectors: &[EmbeddingVector],
    ) -> Result<(), IndexError>;
    fn nearest(
        &self,
        setting: &SemanticSetting,
        query: &IndexQuery,
        vector: &EmbeddingVector,
    ) -> Result<Vec<IndexPassage>, IndexError>;
}

/// Each retrieval method gets one vote per passage. Raw BM25 and cosine scores
/// have different scales, so only their ranks participate (RRF k=60).
pub fn fuse_passages(legs: Vec<Vec<IndexPassage>>, limit: usize) -> Vec<IndexPassage> {
    let mut scores = std::collections::BTreeMap::new();
    for leg in legs {
        let mut seen = std::collections::BTreeSet::new();
        for (rank, passage) in leg.into_iter().take(super::MAX_SEARCH_RESULTS).enumerate() {
            let r = &passage.reference;
            let key = (
                r.root_id.clone(),
                r.file_id.clone(),
                r.version_id.clone(),
                r.ordinal,
            );
            if seen.insert(key.clone()) {
                let entry = scores.entry(key).or_insert((0., passage));
                entry.0 += 1.
                    / (61. + f64::from(u32::try_from(rank).expect("bounded result rank fits u32")));
            }
        }
    }
    let mut scores: Vec<_> = scores.into_iter().collect();
    scores.sort_by(|(ka, (a, _)), (kb, (b, _))| b.total_cmp(a).then_with(|| ka.cmp(kb)));
    scores
        .into_iter()
        .take(limit)
        .map(|(_, (_, p))| p)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ModelConnection, ModelInfo, ProviderKind};
    #[test]
    fn catalog_metadata_overrides_incomplete_manual_embedding_entries() {
        use crate::models::{CatalogSnapshot, CatalogState, ModelsState};
        let mut connection = ModelConnection::new("local", ProviderKind::OpenAiCompatible);
        connection.manual_models.push(ModelInfo {
            id: "known-chat".into(),
            ..ModelInfo::default()
        });
        let mut state = ModelsState {
            connections: vec![connection.clone()],
            ..ModelsState::default()
        };
        state.catalogs.insert(
            connection.id,
            CatalogState {
                snapshot: CatalogSnapshot {
                    models: vec![ModelInfo {
                        id: "known-chat".into(),
                        output_modalities: vec!["text".into()],
                        ..ModelInfo::default()
                    }],
                    fetched_at: 1,
                },
                ..CatalogState::default()
            },
        );
        assert_eq!(embedding_targets(&state, ""), Vec::<EmbeddingTarget>::new());
    }
    fn passage(id: &str) -> IndexPassage {
        IndexPassage {
            reference: super::super::PassageRef {
                root_id: "root".into(),
                file_id: id.into(),
                version_id: "version".into(),
                ordinal: 0,
            },
            path: id.into(),
            text: id.into(),
            start_line: 1,
            end_line: 1,
            mime_type: "text/plain".into(),
            note: None,
            observed_at: 1,
            modified_at: None,
        }
    }

    #[test]
    fn rank_fusion_rewards_independent_evidence_and_deduplicates_each_leg() {
        let a = passage("a");
        let b = passage("b");
        let c = passage("c");
        assert_eq!(
            fuse_passages(
                vec![vec![a.clone(), b.clone()], vec![c.clone(), b.clone()]],
                2
            ),
            vec![b.clone(), a.clone()]
        );
        assert_eq!(
            fuse_passages(vec![vec![a.clone(), a.clone(), a], vec![b]], 1),
            vec![passage("a")]
        );
        assert_eq!(fuse_passages(vec![], 4), vec![]);
        assert_eq!(fuse_passages(vec![vec![c]], 0), vec![]);
    }

    #[test]
    fn embedding_choices_are_filtered_and_connection_scoped() {
        let mut first = ModelConnection::new("one", ProviderKind::OpenAiCompatible);
        first.manual_models.push(ModelInfo {
            id: "embed".into(),
            ..ModelInfo::default()
        });
        first.manual_models.push(ModelInfo {
            id: "chat".into(),
            output_modalities: vec!["text".into()],
            ..ModelInfo::default()
        });
        let mut second = first.clone();
        second.id = "two".into();
        let state = crate::models::ModelsState {
            connections: vec![first, second],
            ..crate::models::ModelsState::default()
        };
        let choices = embedding_targets(&state, "EMBED");
        assert_eq!(choices.len(), 2);
        assert_ne!(choices[0].key, choices[1].key);
        assert_eq!(embedding_targets(&state, "chat"), vec![]);
    }
    #[test]
    fn embeddings_are_finite_normalized_and_dimension_checked() {
        for values in [
            vec![],
            vec![0., 0.],
            vec![f32::NAN],
            vec![f32::INFINITY],
            vec![1.; 4097],
        ] {
            assert!(EmbeddingVector::new(values).is_err());
        }
        let v = EmbeddingVector::new(vec![f32::MAX, f32::MAX]).unwrap();
        assert!((v.cosine(&v).unwrap() - 1.).abs() < 0.00001);
        assert!(v.cosine(&EmbeddingVector::new(vec![1.]).unwrap()).is_err());
        assert_eq!(
            EmbeddingVector::new(vec![1., 0.])
                .unwrap()
                .cosine(&EmbeddingVector::new(vec![0., 1.]).unwrap())
                .unwrap(),
            0.
        );
    }

    #[test]
    fn embedding_destinations_require_supported_transport_and_preserve_identity() {
        let mut connection = ModelConnection::new("test", ProviderKind::Ollama);
        let mut model = ModelInfo {
            id: "embed".into(),
            ..ModelInfo::default()
        };
        assert!(EmbeddingTarget::for_embeddings(&connection, &model).is_none());
        model.output_modalities.push("embeddings".into());
        let target = EmbeddingTarget::for_embeddings(&connection, &model).unwrap();
        connection.base_url.push_str("/changed");
        assert_ne!(
            target,
            EmbeddingTarget::for_embeddings(&connection, &model).unwrap()
        );
        connection.kind = ProviderKind::Anthropic;
        assert!(EmbeddingTarget::for_embeddings(&connection, &model).is_none());
    }
}
