//! Artifacts people share with their group: a document, checklist or picture that
//! every computer in the group keeps a copy of.
//!
//! A shared artifact is a signed operation in the group's shared lane, folded with
//! the same rules as shared details: sharing a newer version *replaces* the versions
//! it names, two versions made without seeing each other are both kept ("two
//! versions"), and withdraw / erase work exactly as they do for details. Content is
//! plain text, so it is checked here and never run.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::keys::{DeviceId, OpHash, PersonId};
use super::knowledge::DetailStatus;
use crate::Timestamp;
use crate::artifacts::ArtifactKind;

/// Largest shared artifact, in bytes. Every computer stores every version, and the
/// whole history travels to a computer that joins, so this stays modest.
pub const MAX_SHARED_ARTIFACT_BYTES: usize = 512 * 1024;
/// Longest artifact title, in characters.
pub const MAX_SHARED_TITLE_CHARS: usize = 200;
/// Longest language or format name (`python`, `mermaid`).
pub const MAX_SHARED_LANGUAGE_CHARS: usize = 40;
/// Longest library id a sharer may attach (`KnowledgeBody::ShareArtifact::origin`).
pub const MAX_ORIGIN_CHARS: usize = 80;

/// Why an artifact could not be shared (checked before signing, and again on fold).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ArtifactShareError {
    #[error("it needs a title")]
    NoTitle,
    #[error("that title is too long")]
    TitleTooLong,
    #[error("there's nothing in it to share")]
    Empty,
    #[error("it's too big to share with the group (the limit is 512 KB)")]
    TooBig,
    #[error("that format name is too long")]
    LanguageTooLong,
}

/// The rules a shared artifact must meet, used by the sharer and by every receiver.
pub fn check_artifact(
    title: &str,
    language: &str,
    content: &str,
) -> Result<(), ArtifactShareError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(ArtifactShareError::NoTitle);
    }
    if title.chars().count() > MAX_SHARED_TITLE_CHARS {
        return Err(ArtifactShareError::TitleTooLong);
    }
    if content.trim().is_empty() {
        return Err(ArtifactShareError::Empty);
    }
    if content.len() > MAX_SHARED_ARTIFACT_BYTES {
        return Err(ArtifactShareError::TooBig);
    }
    if language.chars().count() > MAX_SHARED_LANGUAGE_CHARS {
        return Err(ArtifactShareError::LanguageTooLong);
    }
    Ok(())
}

/// One shared artifact version as people see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedArtifact {
    pub id: OpHash,
    pub title: String,
    pub kind: ArtifactKind,
    pub language: String,
    pub content: String,
    pub author: DeviceId,
    /// The sharer's person and display name, from the roster.
    pub person: Option<PersonId>,
    pub shared_by: String,
    pub computer: String,
    pub shared_at: Timestamp,
    /// Earlier shared versions this one replaces.
    pub replaces: Vec<OpHash>,
    /// `Replaced` by a newer version, or `Disputed` when two versions were made at
    /// once (the UI says "two versions").
    pub status: DetailStatus,
    /// The sharer's computer is no longer in the group.
    pub from_former_member: bool,
    /// The sharer's own library id for it (see `KnowledgeBody::ShareArtifact`).
    pub origin: String,
}

impl SharedArtifact {
    /// Whether it is one of the versions people should see first.
    pub fn is_latest(&self) -> bool {
        !self.from_former_member
            && matches!(
                self.status,
                DetailStatus::Current | DetailStatus::Disputed { .. }
            )
    }

    /// Every version in its history, including itself: what a new version of it
    /// should look back on.
    pub fn lineage(&self, all: &std::collections::BTreeMap<OpHash, Self>) -> BTreeSet<OpHash> {
        walk(self.id, |id| {
            all.get(id).map(|artifact| artifact.replaces.as_slice())
        })
    }
}

/// Follows `replaces` back from `start`, including `start` itself.
fn walk<'a>(start: OpHash, replaces: impl Fn(&OpHash) -> Option<&'a [OpHash]>) -> BTreeSet<OpHash> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![start];
    while let Some(id) = stack.pop() {
        if let Some(older) = replaces(&id)
            && seen.insert(id)
        {
            stack.extend(older.iter().copied());
        }
    }
    seen
}

/// Every version in `start`'s history (including itself) among `shared`.
pub fn lineage_ids(start: OpHash, shared: &[SharedArtifact]) -> BTreeSet<OpHash> {
    let by_id: std::collections::BTreeMap<OpHash, &SharedArtifact> = shared
        .iter()
        .map(|artifact| (artifact.id, artifact))
        .collect();
    walk(start, |id| {
        by_id.get(id).map(|artifact| artifact.replaces.as_slice())
    })
}

/// How one of this computer's saved artifacts stands with the group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharingState {
    /// Never shared, and not a copy of something shared.
    NotShared,
    /// The group's latest version says exactly what this copy says.
    UpToDate,
    /// It was shared (or copied from the group) and has changed since: sharing it
    /// again makes it the newest version of `version_of`.
    Changed { version_of: OpHash },
}

/// Whether the saved artifact `local_id` (with its title, content and the shared id
/// it was copied from, if any) is already shared, and as which shared artifact.
pub fn sharing_state(
    local_id: &str,
    shared_from: &str,
    title: &str,
    content: &str,
    this_computer: &DeviceId,
    shared: &[SharedArtifact],
) -> SharingState {
    let copied_from = OpHash::from_hex(shared_from)
        .ok()
        .filter(|id| shared.iter().any(|artifact| artifact.id == *id));
    let link = copied_from.or_else(|| {
        shared
            .iter()
            .filter(|artifact| {
                !local_id.is_empty()
                    && artifact.author == *this_computer
                    && artifact.origin == local_id
            })
            .max_by_key(|artifact| (artifact.shared_at, artifact.id))
            .map(|artifact| artifact.id)
    });
    let Some(link) = link else {
        return SharingState::NotShared;
    };
    let up_to_date = shared.iter().any(|candidate| {
        candidate.is_latest()
            && lineage_ids(candidate.id, shared).contains(&link)
            && candidate.title == title.trim()
            && candidate.content == content
    });
    if up_to_date {
        SharingState::UpToDate
    } else {
        SharingState::Changed { version_of: link }
    }
}
