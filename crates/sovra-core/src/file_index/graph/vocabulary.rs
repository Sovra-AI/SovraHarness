//! The stored vocabulary is open to domain-specific classes and typed properties.
//! Legacy unit variants keep their original JSON identifiers.

use super::IndexError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VocabularyId(String);

impl AsRef<str> for VocabularyId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for VocabularyId {
    type Error = IndexError;
    fn try_from(id: String) -> Result<Self, IndexError> {
        if id.len() > 120
            || !id.starts_with(|c: char| c.is_ascii_lowercase())
            || !id.bytes().all(|b| {
                b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || matches!(b, b'_' | b'-' | b'.' | b':')
            })
        {
            return Err(IndexError(
                "Use a short lowercase identifier such as farming:grown_by for a domain term."
                    .into(),
            ));
        }
        Ok(Self(id))
    }
}

impl From<VocabularyId> for String {
    fn from(id: VocabularyId) -> Self {
        id.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Document,
    Section,
    Topic,
    Person,
    Organization,
    Project,
    Task,
    Category,
    Literal,
    Domain(VocabularyId),
}

impl EntityKind {
    pub fn matches_source_class(&self, class: &str) -> bool {
        self.label().eq_ignore_ascii_case(class)
            || (*self == Self::Literal && class.eq_ignore_ascii_case("literal"))
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Document => "Document",
            Self::Section => "Section",
            Self::Topic => "Topic",
            Self::Person => "Person",
            Self::Organization => "Organization",
            Self::Project => "Project",
            Self::Task => "Task",
            Self::Category => "Category",
            Self::Literal => "Value",
            Self::Domain(id) => &id.0,
        }
    }
}

/// A stable property identifier declares both endpoint classes. Distinct namespaces
/// allow two vocabularies to use the same local name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainProperty {
    pub id: VocabularyId,
    pub subject: EntityKind,
    pub object: EntityKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    GroundedIn,
    PartOf,
    About,
    ForProject,
    DependsOn,
    RelatedTo,
    Revises,
    IsA,
    Property(DomainProperty),
}

impl RelationKind {
    pub fn accepts(&self, subject: Option<&EntityKind>, object: Option<&EntityKind>) -> bool {
        use EntityKind::{Category, Document, Project, Section, Task};
        match self {
            Self::GroundedIn => subject.is_some() && object.is_none(),
            Self::PartOf => matches!(
                (subject, object),
                (Some(Section), Some(Document | Section)) | (Some(Task), Some(Project | Task))
            ),
            Self::About => matches!(subject, Some(Document | Section)) && object.is_some(),
            Self::ForProject => {
                matches!(subject, Some(Document | Section | Task)) && object == Some(&Project)
            }
            Self::DependsOn => {
                matches!(subject, Some(Document | Project | Task))
                    && matches!(object, Some(Document | Project | Task))
            }
            Self::RelatedTo => subject.is_some() && object.is_some(),
            Self::Revises => subject == Some(&Document) && object == Some(&Document),
            Self::IsA => subject.is_some() && object == Some(&Category),
            Self::Property(property) => {
                subject == Some(&property.subject) && object == Some(&property.object)
            }
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::GroundedIn => "grounded in",
            Self::PartOf => "part of",
            Self::About => "about",
            Self::ForProject => "for project",
            Self::DependsOn => "depends on",
            Self::RelatedTo => "related to",
            Self::Revises => "revises",
            Self::IsA => "is a",
            Self::Property(property) => &property.id.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphDirection {
    #[default]
    Either,
    Outgoing,
    Incoming,
}
