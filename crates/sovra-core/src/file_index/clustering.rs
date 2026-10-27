//! Bounded soft-clustering contracts. Membership is retrieval structure, not truth.

use super::{
    IndexError,
    semantic::EmbeddingVector,
    summary::{MAX_SUMMARY_INPUT_BYTES, MAX_SUMMARY_INPUTS, SummaryInputRef},
};
use serde::{Deserialize, Serialize};
mod mixture;
pub use mixture::{MixtureSelection, select_mixture};

pub const MAX_CLUSTER_INPUTS: usize = 512;
pub const RAPTOR_CLUSTERING_REVISION: &str = "raptor-umap-0.4.5-full-gmm-v1";

/// Full-covariance Gaussian mixture BIC; lower scores are preferred.
pub fn gaussian_bic(
    log_likelihood: f64,
    samples: usize,
    dimensions: usize,
    components: usize,
) -> Result<f64, IndexError> {
    if !log_likelihood.is_finite()
        || !(2..=MAX_CLUSTER_INPUTS).contains(&samples)
        || !(1..=16).contains(&dimensions)
        || !(1..=32).contains(&components)
        || components > samples
    {
        return Err(IndexError(
            "Invalid Gaussian mixture evidence or shape.".into(),
        ));
    }
    let parameters = components * (dimensions * (dimensions + 1) / 2 + dimensions) + components - 1;
    let score = -2. * log_likelihood
        + f64::from(u32::try_from(parameters).expect("bounded mixture parameters"))
            * f64::from(u32::try_from(samples).expect("bounded cluster inputs")).ln();
    if score.is_finite() {
        Ok(score)
    } else {
        Err(IndexError("The mixture score is not finite.".into()))
    }
}

/// Keep overlapping posterior memberships and give every input at least one home.
pub fn soft_memberships(
    probabilities: &[Vec<f64>],
    threshold: f64,
) -> Result<Vec<Vec<usize>>, IndexError> {
    let components = probabilities.first().map_or(0, Vec::len);
    if probabilities.is_empty()
        || probabilities.len() > MAX_CLUSTER_INPUTS
        || !(1..=32).contains(&components)
        || !threshold.is_finite()
        || !(0.05..=0.9).contains(&threshold)
    {
        return Err(IndexError(
            "Invalid soft-clustering probabilities or threshold.".into(),
        ));
    }
    let mut groups = vec![vec![]; components];
    for (index, row) in probabilities.iter().enumerate() {
        if row.len() != components
            || row
                .iter()
                .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
            || (row.iter().sum::<f64>() - 1.).abs() > 1e-6
        {
            return Err(IndexError(
                "Mixture memberships must be finite, normalized probabilities.".into(),
            ));
        }
        let mut strongest = 0;
        let mut assigned = false;
        for (component, probability) in row.iter().enumerate() {
            if *probability > row[strongest] {
                strongest = component;
            }
            if *probability > threshold {
                groups[component].push(index);
                assigned = true;
            }
        }
        if !assigned {
            groups[strongest].push(index);
        }
    }
    groups.retain(|group| !group.is_empty());
    groups.sort();
    groups.dedup();
    Ok(groups)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClusterInput {
    pub reference: SummaryInputRef,
    pub embedding: EmbeddingVector,
    /// Includes source labels and extraction notes; never a guessed token count.
    pub text_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RaptorOptions {
    pub dimensions: u8,
    pub max_components: u8,
    pub membership_threshold: f64,
    pub umap_epochs: u16,
    pub gmm_iterations: u16,
    pub max_group_inputs: usize,
    pub max_group_bytes: usize,
}

impl Default for RaptorOptions {
    fn default() -> Self {
        Self {
            dimensions: 10,
            max_components: 20,
            membership_threshold: 0.1,
            umap_epochs: 200,
            gmm_iterations: 100,
            max_group_inputs: MAX_SUMMARY_INPUTS,
            max_group_bytes: MAX_SUMMARY_INPUT_BYTES,
        }
    }
}

impl RaptorOptions {
    pub fn validate(&self) -> Result<(), IndexError> {
        if !(1..=16).contains(&self.dimensions)
            || !(1..=32).contains(&self.max_components)
            || !self.membership_threshold.is_finite()
            || !(0.05..=0.9).contains(&self.membership_threshold)
            || !(20..=500).contains(&self.umap_epochs)
            || !(10..=200).contains(&self.gmm_iterations)
            || !(2..=MAX_SUMMARY_INPUTS).contains(&self.max_group_inputs)
            || !(1024..=MAX_SUMMARY_INPUT_BYTES).contains(&self.max_group_bytes)
        {
            return Err(IndexError(
                "The clustering settings exceed the supported computation or summary-input bounds."
                    .into(),
            ));
        }
        Ok(())
    }

    pub fn validate_inputs(&self, inputs: &[ClusterInput]) -> Result<(), IndexError> {
        self.validate()?;
        if inputs.is_empty() || inputs.len() > MAX_CLUSTER_INPUTS {
            return Err(IndexError(
                "Cluster 1–512 inputs at a time; split larger map builds into explicit stages."
                    .into(),
            ));
        }
        let dimensions = inputs[0].embedding.values().len();
        for (index, input) in inputs.iter().enumerate() {
            if input.embedding.values().len() != dimensions
                || input.text_bytes == 0
                || input.text_bytes > self.max_group_bytes
            {
                return Err(IndexError("Clustering needs matching embedding dimensions and inputs that each fit the summary budget.".into()));
            }
            if inputs[..index]
                .iter()
                .any(|other| other.reference == input.reference)
            {
                return Err(IndexError(
                    "A clustering stage cannot repeat an input reference.".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterDiagnostics {
    pub reductions: u32,
    pub mixture_fits: u32,
    pub failed_fits: u32,
    /// Semantic bisections used only when repeated soft clustering cannot fit a budget.
    pub budget_splits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterPlan {
    /// Positions in the supplied input array; an input may occur in several groups.
    pub groups: Vec<Vec<usize>>,
    pub diagnostics: ClusterDiagnostics,
    pub algorithm_revision: String,
}

pub trait ClusterObserver: Send + Sync {
    fn cancelled(&self) -> bool;
}

/// CPU work runs on a runtime-owned blocking task with a cooperative observer.
pub trait SummaryClusterer: Send + Sync + std::fmt::Debug {
    fn cluster(
        &self,
        inputs: &[ClusterInput],
        options: &RaptorOptions,
        observer: &dyn ClusterObserver,
    ) -> Result<ClusterPlan, IndexError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bic_counts_full_covariances_means_and_mixing_weights() {
        let score = gaussian_bic(-100., 20, 2, 1).unwrap();
        assert!((score - (200. + 5. * 20_f64.ln())).abs() < 1e-10);
        let two = gaussian_bic(-80., 20, 2, 2).unwrap();
        assert!((two - (160. + 11. * 20_f64.ln())).abs() < 1e-10);
        assert!(two < score);
        assert!(gaussian_bic(f64::INFINITY, 20, 2, 2).is_err());
        assert!(gaussian_bic(-10., 2, 2, 3).is_err());
    }
    #[test]
    fn soft_assignments_keep_overlap_and_never_drop_ambiguous_inputs() {
        let probabilities = vec![vec![0.9, 0.1], vec![0.4, 0.6], vec![0.02, 0.98]];
        assert_eq!(
            soft_memberships(&probabilities, 0.2).unwrap(),
            vec![vec![0, 1], vec![1, 2]]
        );
        assert_eq!(
            soft_memberships(&[vec![0.5, 0.5]], 0.9).unwrap(),
            vec![vec![0]]
        );
        assert!(soft_memberships(&[vec![f64::NAN, 1.]], 0.1).is_err());
        assert!(soft_memberships(&[vec![0.1, 0.2]], 0.1).is_err());
    }
    #[test]
    fn clustering_rejects_unbounded_work_and_incompatible_inputs() {
        let options = RaptorOptions::default();
        assert!(options.validate().is_ok());
        assert!(
            RaptorOptions {
                membership_threshold: f64::NAN,
                ..options
            }
            .validate()
            .is_err()
        );
        assert!(
            RaptorOptions {
                max_components: 33,
                ..options
            }
            .validate()
            .is_err()
        );
        assert!(
            RaptorOptions {
                max_group_bytes: MAX_SUMMARY_INPUT_BYTES + 1,
                ..options
            }
            .validate()
            .is_err()
        );
        let input = ClusterInput {
            reference: SummaryInputRef::Summary(1),
            embedding: EmbeddingVector::new(vec![1., 0.]).unwrap(),
            text_bytes: 100,
        };
        assert!(
            options
                .validate_inputs(std::slice::from_ref(&input))
                .is_ok()
        );
        assert!(options.validate_inputs(&[]).is_err());
        assert!(
            options
                .validate_inputs(&[input.clone(), input.clone()])
                .is_err()
        );
        let different = ClusterInput {
            reference: SummaryInputRef::Summary(2),
            embedding: EmbeddingVector::new(vec![1.]).unwrap(),
            ..input
        };
        assert!(
            options
                .validate_inputs(&[input.clone(), different])
                .is_err()
        );
        assert!(
            options
                .validate_inputs(&[ClusterInput {
                    text_bytes: MAX_SUMMARY_INPUT_BYTES + 1,
                    ..input
                }])
                .is_err()
        );
    }
}
