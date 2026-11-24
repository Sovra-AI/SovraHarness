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
    /// Standard vocabulary shared by the graph and its browser.
    pub const BUILT_INS: &[Self] = &[
        Self::Document,
        Self::Section,
        Self::Topic,
        Self::Person,
        Self::Organization,
        Self::Project,
        Self::Task,
        Self::Category,
        Self::Literal,
    ];

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

    /// The label as a person reads it: `food:grower` shows as "Grower".
    pub fn display_name(&self) -> String {
        let words = readable(self.label());
        let mut chars = words.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        })
    }
}

/// A vocabulary identifier without its namespace and with spaces between words,
/// so `food:grown_by` reads "grown by". Built-in labels pass through unchanged.
fn readable(label: &str) -> String {
    let local = label.rsplit_once(':').map_or(label, |(_, local)| local);
    let words = local
        .split(['_', '-'])
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if words.is_empty() {
        label.to_owned()
    } else {
        words
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
    /// Standard vocabulary shared by the graph and its browser.
    pub const BUILT_INS: &[Self] = &[
        Self::GroundedIn,
        Self::PartOf,
        Self::About,
        Self::ForProject,
        Self::DependsOn,
        Self::RelatedTo,
        Self::Revises,
        Self::IsA,
    ];

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

    /// The relationship as a person reads it: `food:grown_by` shows as "grown by".
    pub fn display_name(&self) -> String {
        readable(self.label())
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn custom_names_read_as_words_and_built_ins_are_unchanged() {
        let property = RelationKind::Property(DomainProperty {
            id: VocabularyId("food:grown_by".into()),
            subject: EntityKind::Domain(VocabularyId("food:product".into())),
            object: EntityKind::Domain(VocabularyId("grower".into())),
        });
        assert_eq!(property.display_name(), "grown by");
        assert_eq!(RelationKind::DependsOn.display_name(), "depends on");
        let RelationKind::Property(property) = property else {
            unreachable!()
        };
        assert_eq!(property.subject.display_name(), "Product");
        assert_eq!(property.object.display_name(), "Grower");
        assert_eq!(EntityKind::Literal.display_name(), "Value");
        assert_eq!(
            EntityKind::Domain(VocabularyId(":".into())).display_name(),
            ":"
        );
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
