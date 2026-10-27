//! Structured file inventory queries. Filesystem dates describe observations,
//! not the dates of real-world events mentioned inside a document.

use super::{IndexError, IndexStatus, MAX_INDEX_ROOTS, PassageRef, validate_relative_path};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CatalogFilter {
    pub roots: Vec<String>,
    pub path_prefix: Option<String>,
    /// Exact relative parent directory; an empty string selects top-level files.
    pub directory: Option<String>,
    /// Literal, case-sensitive filename substring; never a regex or shell glob.
    pub name_contains: Option<String>,
    pub mime_prefix: Option<String>,
    pub keywords: Option<String>,
    pub modified_after: Option<u64>,
    pub modified_before: Option<u64>,
    pub min_bytes: Option<u64>,
    pub max_bytes: Option<u64>,
    pub has_text: Option<bool>,
    pub extraction: Option<ExtractionState>,
}

impl CatalogFilter {
    pub fn validate(&self) -> Result<(), IndexError> {
        if self.roots.len() > MAX_INDEX_ROOTS
            || self.roots.iter().any(|id| id.is_empty() || id.len() > 120)
        {
            return Err(IndexError(
                "Use at most 16 folder IDs returned by list_indexed_folders.".into(),
            ));
        }
        if let Some(path) = &self.path_prefix {
            validate_relative_path(path)?;
        }
        if let Some(directory) = &self.directory
            && !directory.is_empty()
        {
            validate_relative_path(directory)?;
        }
        if self
            .name_contains
            .as_ref()
            .is_some_and(|name| name.is_empty() || name.len() > 256 || name.contains('\0'))
        {
            return Err(IndexError(
                "name_contains must be a literal filename fragment of 1–256 bytes.".into(),
            ));
        }
        if self.mime_prefix.as_ref().is_some_and(|mime| {
            mime.is_empty()
                || mime.len() > 128
                || !mime
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"/-+._".contains(&c))
        }) {
            return Err(IndexError(
                "Use a MIME type or prefix, such as application/pdf or image/.".into(),
            ));
        }
        if let Some(text) = &self.keywords {
            super::validate_search_text(text)?;
        }
        for (lower, upper, label) in [
            (
                self.modified_after,
                self.modified_before,
                "modification dates",
            ),
            (self.min_bytes, self.max_bytes, "file sizes"),
        ] {
            if lower.zip(upper).is_some_and(|(a, b)| a > b)
                || [lower, upper]
                    .into_iter()
                    .flatten()
                    .any(|n| n > i64::MAX as u64)
            {
                return Err(IndexError(format!(
                    "Use an ordered, nonnegative range for {label}."
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogGroupBy {
    Root,
    Directory,
    MimeType,
    Extension,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogCursor {
    pub revision: u64,
    pub filter_key: String,
    pub root_id: String,
    pub path: String,
    pub file_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CatalogQuery {
    pub filter: CatalogFilter,
    pub group_by: Option<CatalogGroupBy>,
    pub cursor: Option<CatalogCursor>,
    pub limit: usize,
}

impl Default for CatalogQuery {
    fn default() -> Self {
        Self {
            filter: CatalogFilter::default(),
            group_by: None,
            cursor: None,
            limit: 20,
        }
    }
}

impl CatalogQuery {
    pub fn validate(&self) -> Result<(), IndexError> {
        self.filter.validate()?;
        if !(1..=40).contains(&self.limit) {
            return Err(IndexError(
                "Request between 1 and 40 catalogue rows.".into(),
            ));
        }
        if let Some(cursor) = &self.cursor {
            if self.group_by.is_some()
                || cursor.revision > i64::MAX as u64
                || [&cursor.root_id, &cursor.file_id, &cursor.filter_key]
                    .iter()
                    .any(|s| s.is_empty() || s.len() > 120)
            {
                return Err(IndexError(
                    "Use the next_cursor returned by a file listing with the same filters.".into(),
                ));
            }
            // A cursor is an opaque database ordering key, never a filesystem
            // path. Native names may contain characters disallowed in filters.
            if cursor.path.is_empty() || cursor.path.len() > 32_768 || cursor.path.contains('\0') {
                return Err(IndexError("Invalid catalogue continuation path.".into()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionState {
    Text,
    MetadataOnly,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub root_id: String,
    pub root_path: String,
    pub file_id: String,
    pub path: String,
    pub byte_size: Option<u64>,
    pub mime_type: Option<String>,
    pub content_digest: Option<String>,
    pub observed_at: Option<u64>,
    pub modified_at: Option<u64>,
    pub created_at: Option<u64>,
    pub extraction: ExtractionState,
    pub note: Option<String>,
    pub first_passage: Option<PassageRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogGroup {
    pub root_id: Option<String>,
    pub key: String,
    pub files: u64,
    pub total_known_bytes: u64,
    pub unknown_sizes: u64,
    pub files_with_text: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPage {
    pub revision: u64,
    /// Counts refer to matching observed files; scan coverage is reported separately.
    pub total_files: u64,
    pub total_known_bytes: u64,
    pub unknown_sizes: u64,
    pub coverage: Vec<CatalogCoverage>,
    pub files: Vec<CatalogEntry>,
    pub groups: Vec<CatalogGroup>,
    pub next_cursor: Option<CatalogCursor>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogCoverage {
    pub root_id: String,
    pub status: IndexStatus,
    pub completed_at: Option<u64>,
    pub visited: u64,
    pub skipped: u64,
    pub failed: u64,
}

impl From<&super::IndexedRoot> for CatalogCoverage {
    fn from(root: &super::IndexedRoot) -> Self {
        Self {
            root_id: root.id.clone(),
            status: root.status,
            completed_at: root.completed_at,
            visited: root.progress.visited,
            skipped: root.progress.skipped,
            failed: root.progress.failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_queries_accept_literal_filters_and_reject_escapes_and_inverted_ranges() {
        let mut query = CatalogQuery::default();
        query.filter.name_contains = Some("100%_report's.pdf".into());
        query.filter.mime_prefix = Some("application/".into());
        query.filter.modified_after = Some(1_000);
        query.filter.modified_before = Some(2_000);
        assert!(query.validate().is_ok());
        query.filter.modified_before = Some(999);
        assert!(query.validate().is_err());
        query.filter.modified_before = None;
        query.filter.path_prefix = Some("../outside".into());
        assert!(query.validate().is_err());
        query.filter.path_prefix = None;
        query.limit = 0;
        assert!(query.validate().is_err());
        query.limit = 40;
        query.filter.roots = vec!["x".repeat(121)];
        assert!(query.validate().is_err());
        query.filter.roots.clear();
        query.filter.max_bytes = Some(u64::MAX);
        assert!(query.validate().is_err());
        query.filter.max_bytes = None;
        query.filter.keywords = Some("word ".repeat(33));
        assert!(query.validate().is_err());
        query.filter.keywords = Some("word ".repeat(32));
        assert!(query.validate().is_ok());
    }
}
