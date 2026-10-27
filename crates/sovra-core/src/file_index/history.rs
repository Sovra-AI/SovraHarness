//! Source observation history. Recorded time and real-world validity are distinct:
//! filesystem modification timestamps never establish a document's event dates.

use super::{IndexError, IndexPassage, validate_relative_path};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceTier {
    FilesystemObservation,
    DeterministicExtraction,
    HeuristicExtraction,
    ModelInference,
    UserAssertion,
    LegacyImport,
}

/// Confidence in the stated observation, not a guarantee about source truth.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Confidence(f64);

// Construction and deserialization reject NaN, so equality is an equivalence
// relation over every value this private representation can hold.
impl Eq for Confidence {}

impl Confidence {
    pub fn new(value: f64) -> Result<Self, IndexError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(IndexError(
                "Confidence must be finite and between 0 and 1.".into(),
            ))
        }
    }

    pub const fn value(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Confidence {
    type Error = IndexError;
    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Confidence> for f64 {
    fn from(value: Confidence) -> Self {
        value.value()
    }
}

/// Explicitly unknown until evidence establishes real-world event validity.
/// Timestamps throughout file history use Unix seconds, as does the catalogue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldValidity {
    pub from: Option<u64>,
    pub until: Option<u64>,
}

impl WorldValidity {
    pub fn validate(&self) -> Result<(), IndexError> {
        if self
            .from
            .zip(self.until)
            .is_some_and(|(from, until)| from > until)
        {
            return Err(IndexError(
                "Real-world validity must be an ordered interval.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceBasis {
    pub tier: EvidenceTier,
    pub confidence: Option<Confidence>,
    pub activity_id: Option<u64>,
    pub recorded_at: u64,
    pub observed_at: Option<u64>,
    /// None means unknown; do not substitute filesystem creation/modification time.
    pub valid_time: Option<WorldValidity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileHistoryQuery {
    pub root_id: String,
    pub file_id: Option<String>,
    /// An exact relative path can locate a current or formerly observed file.
    pub path: Option<String>,
    /// Page older observations using the returned `next_before` value.
    pub before: Option<u64>,
    pub limit: usize,
}

impl Default for FileHistoryQuery {
    fn default() -> Self {
        Self {
            root_id: String::new(),
            file_id: None,
            path: None,
            before: None,
            limit: 10,
        }
    }
}

impl FileHistoryQuery {
    pub fn validate(&self) -> Result<(), IndexError> {
        validate_id(&self.root_id)?;
        if self.file_id.is_some() == self.path.is_some() {
            return Err(IndexError(
                "Provide exactly one file_id or relative path.".into(),
            ));
        }
        if let Some(id) = &self.file_id {
            validate_id(id)?;
        }
        if let Some(path) = &self.path {
            validate_relative_path(path)?;
        }
        if !(1..=20).contains(&self.limit)
            || self
                .before
                .is_some_and(|n| n == 0 || i64::try_from(n).is_err())
        {
            return Err(IndexError(
                "Request 1–20 observations and use the returned next_before cursor.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryPassageRef {
    pub root_id: String,
    pub file_id: String,
    pub observation: u64,
    pub ordinal: u32,
}

impl HistoryPassageRef {
    pub fn validate(&self) -> Result<(), IndexError> {
        validate_id(&self.root_id)?;
        validate_id(&self.file_id)?;
        if self.observation == 0 || i64::try_from(self.observation).is_err() {
            return Err(IndexError(
                "Use the historical reference returned by inspect_file_history.".into(),
            ));
        }
        Ok(())
    }
}

fn validate_id(id: &str) -> Result<(), IndexError> {
    if id.is_empty() || id.len() > 120 || id.contains('\0') {
        return Err(IndexError("Invalid indexed scope or file ID.".into()));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedPresence {
    Present,
    /// A complete scan did not find an eligible path; this is not proof of deletion.
    NotObserved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanActivity {
    pub id: u64,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub status: super::IndexStatus,
    pub extractor_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRepresentation {
    pub version_id: String,
    pub content_digest: String,
    pub mime_type: String,
    pub first_indexed_at: u64,
    pub extractor_revision: Option<String>,
    pub generated_by: Option<u64>,
    pub extracted_passages: u64,
    pub first_passage: Option<HistoryPassageRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileObservation {
    pub sequence: u64,
    pub path: String,
    /// Opaque physical identity; separate from the tracked file and content version.
    pub physical_id: Option<String>,
    pub presence: ObservedPresence,
    pub byte_size: Option<u64>,
    pub modified_at: Option<u64>,
    pub created_at: Option<u64>,
    pub extraction: super::ExtractionState,
    pub note: Option<String>,
    pub basis: EvidenceBasis,
    pub activity: Option<ScanActivity>,
    pub representation: Option<SourceRepresentation>,
    /// Append-only supersession keeps earlier observations intact.
    pub replaces: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileHistory {
    pub root_id: String,
    pub root_path: String,
    pub file_id: String,
    pub current_path: String,
    /// Last recorded index state, not a live filesystem check.
    pub currently_present: bool,
    pub coverage: super::CatalogCoverage,
    pub observations: Vec<FileObservation>,
    pub next_before: Option<u64>,
    /// Earlier extracted versions may predate the observation log.
    pub versions_without_observation: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoricalPassage {
    pub reference: HistoryPassageRef,
    pub passage: IndexPassage,
    pub basis: EvidenceBasis,
    pub next: Option<HistoryPassageRef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_confidence_and_world_time_reject_invalid_values() {
        for bad in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            assert!(Confidence::new(bad).is_err());
        }
        let confidence = Confidence::new(0.75).unwrap();
        assert_eq!(serde_json::to_string(&confidence).unwrap(), "0.75");
        assert!(serde_json::from_str::<Confidence>("2.0").is_err());
        assert!(
            WorldValidity {
                from: Some(20),
                until: Some(10)
            }
            .validate()
            .is_err()
        );
        assert!(
            WorldValidity {
                from: None,
                until: None
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn history_requires_a_single_scoped_selector_and_bounded_cursor() {
        let mut query = FileHistoryQuery {
            root_id: "scope".into(),
            path: Some("notes/a.txt".into()),
            ..FileHistoryQuery::default()
        };
        assert!(query.validate().is_ok());
        query.file_id = Some("file".into());
        assert!(query.validate().is_err());
        query.path = None;
        assert!(query.validate().is_ok());
        query.before = Some(u64::MAX);
        assert!(query.validate().is_err());
        query.before = None;
        query.limit = 21;
        assert!(query.validate().is_err());
        let reference = HistoryPassageRef {
            root_id: "scope".into(),
            file_id: "file".into(),
            observation: 1,
            ordinal: 0,
        };
        assert!(reference.validate().is_ok());
    }
}
