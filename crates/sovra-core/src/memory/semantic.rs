//! Optional semantic memory indexes, scoped to an explicitly chosen destination.

use super::{Fact, FactId, MemoryError};
use crate::{file_index::semantic::EmbeddingVector, models::ModelDestination};

pub const MEMORY_EMBEDDING_BATCH: usize = 8;
pub const MEMORY_SEMANTIC_CANDIDATES: usize = 24;
/// A conservative candidate gate, not a calibrated probability of relevance.
pub const MEMORY_SIMILARITY_FLOOR: f64 = 0.6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySemanticSetting {
    pub generation: u64,
    pub target: ModelDestination,
    /// Learned from the first valid batch, then fixed for this index generation.
    pub dimension: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemorySemanticState {
    pub generation: u64,
    pub setting: Option<MemorySemanticSetting>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum MemorySemanticPhase {
    #[default]
    Disabled,
    Paused,
    Updating,
    Ready,
    Failed(String),
}

impl MemorySemanticPhase {
    pub fn label(&self) -> &str {
        match self {
            Self::Disabled => "Search by meaning is off. Matching words still work.",
            Self::Paused => "Search by meaning is paused with remembering.",
            Self::Updating => "Preparing saved details for search by meaning…",
            Self::Ready => "Search by meaning is ready.",
            Self::Failed(message) => message,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemorySemanticStatus {
    pub state: MemorySemanticState,
    pub phase: MemorySemanticPhase,
}

impl MemorySemanticStatus {
    pub fn apply(&mut self, status: &Self) {
        if status.state.generation >= self.state.generation {
            *self = status.clone();
        }
    }
}

/// Text is omitted from diagnostics; this is user-authored conversation evidence.
#[derive(Clone)]
pub struct MemoryEmbeddingInput {
    pub fact_id: FactId,
    pub text: String,
}
impl std::fmt::Debug for MemoryEmbeddingInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryEmbeddingInput")
            .field("fact_id", &self.fact_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct MemoryVector {
    pub fact_id: FactId,
    pub vector: EmbeddingVector,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemorySemanticHit {
    pub fact: Fact,
    pub similarity: f64,
}

/// Vectors share the transport's finite, nonzero, normalized representation.
pub fn validate_memory_vectors(vectors: &[MemoryVector]) -> Result<usize, MemoryError> {
    if vectors.is_empty() || vectors.len() > MEMORY_EMBEDDING_BATCH {
        return Err(MemoryError::InvalidInput(
            "invalid memory embedding batch".into(),
        ));
    }
    let dimension = vectors[0].vector.values().len();
    let mut seen = std::collections::HashSet::new();
    for item in vectors {
        if item.vector.values().len() != dimension
            || !seen.insert(&item.fact_id)
            || item.fact_id.0.trim().is_empty()
            || item.fact_id.0.len() > 512
        {
            return Err(MemoryError::InvalidInput(
                "inconsistent memory embedding batch".into(),
            ));
        }
    }
    Ok(dimension)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_index_status_cannot_override_a_newer_selection() {
        let mut status = MemorySemanticStatus {
            state: MemorySemanticState {
                generation: 2,
                setting: None,
            },
            phase: MemorySemanticPhase::Disabled,
        };
        status.apply(&MemorySemanticStatus {
            state: MemorySemanticState {
                generation: 1,
                setting: None,
            },
            phase: MemorySemanticPhase::Ready,
        });
        assert_eq!(status.phase, MemorySemanticPhase::Disabled);
        assert!(status.phase.label().contains("Matching words"));
        for phase in [
            MemorySemanticPhase::Paused,
            MemorySemanticPhase::Updating,
            MemorySemanticPhase::Ready,
            MemorySemanticPhase::Failed("Unavailable".into()),
        ] {
            assert_ne!(phase.label(), "");
        }
    }
    #[test]
    fn vector_batches_reject_mixed_dimensions_duplicates_and_unbounded_inputs() {
        let vector = MemoryVector {
            fact_id: FactId("fact:one".into()),
            vector: EmbeddingVector::new(vec![1., 0.]).unwrap(),
        };
        assert_eq!(
            validate_memory_vectors(std::slice::from_ref(&vector)).unwrap(),
            2
        );
        assert!(validate_memory_vectors(&[]).is_err());
        assert!(validate_memory_vectors(&vec![vector.clone(); 9]).is_err());
        assert!(validate_memory_vectors(&[vector.clone(), vector.clone()]).is_err());
        assert!(
            validate_memory_vectors(&[
                vector,
                MemoryVector {
                    fact_id: FactId("fact:two".into()),
                    vector: EmbeddingVector::new(vec![1.]).unwrap()
                }
            ])
            .is_err()
        );
        let input = MemoryEmbeddingInput {
            fact_id: FactId("fact:one".into()),
            text: "private memory".into(),
        };
        assert!(!format!("{input:?}").contains("private memory"));
    }
}
