//! Source identity and wording travel with every local analysis.
use super::{AnalysisOptions, SourceAnalysis, analyze};
use crate::{file_index::PassageRef, memory::Timestamp};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExploreRequest {
    pub id: u64,
    pub query: String,
    pub options: AnalysisOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Citation {
    File {
        reference: PassageRef,
        observed: u64,
    },
    User {
        source_id: crate::EpisodeId,
        citation: String,
        recorded: Timestamp,
    },
    Artifact {
        id: String,
        updated: Timestamp,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excerpt {
    pub mask: Option<super::mask::Mask>,
    pub citation: Citation,
    pub title: String,
    pub provenance: String,
    pub wording: String,
    pub first_line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReading {
    pub excerpt: Excerpt,
    pub analysis: SourceAnalysis,
}

/// An original line in one excerpt returned by this search. Lines are one-based
/// and relative to the excerpt, including when the excerpt starts mid-file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceLine {
    pub reading: usize,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceConnection {
    pub label: String,
    pub sources: Vec<SourceLine>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exploration {
    pub readings: Vec<SourceReading>,
    pub shared: Vec<SourceConnection>,
    pub notices: Vec<String>,
}

impl Exploration {
    pub fn from_excerpts(
        excerpts: Vec<Excerpt>,
        options: &AnalysisOptions,
        mut notices: Vec<String>,
    ) -> Result<Self, &'static str> {
        options.validate()?;
        let mut readings = Vec::with_capacity(excerpts.len());
        let mut values: BTreeMap<String, BTreeSet<SourceLine>> = BTreeMap::new();
        for mut excerpt in excerpts {
            let mut analysis = analyze(&excerpt.wording, options)?;
            if let Some(mask) = excerpt.mask.take() {
                analysis.findings.push(super::Finding {
                    kind: super::FindingKind::Morphology,
                    from: 1,
                    to: 1,
                    detail: mask.describe(options.mask_radius)?,
                });
            }
            for measured in &analysis.quantities {
                let quantity = &measured.quantity;
                let key = format!("{} {}", quantity.value.normalize(), quantity.unit);
                values.entry(key).or_default().insert(SourceLine {
                    reading: readings.len(),
                    line: measured.line,
                });
            }
            readings.push(SourceReading { excerpt, analysis });
        }
        let mut shared: Vec<_> = values
            .into_iter()
            .filter(|(_, lines)| {
                lines
                    .first()
                    .zip(lines.last())
                    .is_some_and(|(a, b)| a.reading != b.reading)
            })
            .map(|(value, lines)| SourceConnection {
                label: format!("Equivalent reading of {value}"),
                sources: lines.into_iter().collect(),
            })
            .collect();
        // Only explicit identifiers bridge sources automatically. A shared display
        // name alone cannot establish that two records describe the same person.
        let mut subjects = BTreeMap::<&str, Vec<(usize, &super::Statement)>>::new();
        for (i, reading) in readings.iter().enumerate() {
            for edge in &reading.analysis.statements {
                if edge.explicit && edge.subject.contains(':') {
                    subjects.entry(&edge.subject).or_default().push((i, edge));
                }
            }
        }
        let mut paths = BTreeSet::new();
        'readings: for (i, reading) in readings.iter().enumerate() {
            for edge in reading
                .analysis
                .statements
                .iter()
                .filter(|edge| edge.explicit)
            {
                for &(j, next) in subjects.get(edge.object.as_str()).into_iter().flatten() {
                    if i == j
                        || (edge.object_type.is_some()
                            && next.subject_type.is_some()
                            && edge.object_type != next.subject_type)
                    {
                        continue;
                    }
                    paths.insert(SourceConnection {
                        label: format!(
                            "{} → {} → {} → {} → {}",
                            edge.subject, edge.predicate, edge.object, next.predicate, next.object
                        ),
                        sources: vec![
                            SourceLine {
                                reading: i,
                                line: edge.line,
                            },
                            SourceLine {
                                reading: j,
                                line: next.line,
                            },
                        ],
                    });
                    if paths.len() > 16 {
                        break 'readings;
                    }
                }
            }
        }
        if paths.len() > 16 {
            notices.push(
                "Showing 16 linked identifier paths. Narrow your search to explore more.".into(),
            );
        }
        shared.extend(paths.into_iter().take(16));
        Ok(Self {
            readings,
            shared,
            notices,
        })
    }
}

impl Excerpt {
    pub fn artifact(
        id: String,
        updated: Timestamp,
        title: String,
        content: &str,
        offset: usize,
    ) -> Self {
        let offset = content.floor_char_boundary(offset.min(content.len()));
        let start = if offset < super::MAX_ANALYSIS_BYTES {
            0
        } else {
            let near =
                content.floor_char_boundary(offset.saturating_sub(super::MAX_ANALYSIS_BYTES / 2));
            content[near..offset]
                .find('\n')
                .map_or(near, |i| near + i + 1)
        };
        let end = content.floor_char_boundary(content.len().min(start + super::MAX_ANALYSIS_BYTES));
        Self::artifact_range(id, updated, title, content, start..end)
    }

    pub fn artifact_range(
        id: String,
        updated: Timestamp,
        title: String,
        content: &str,
        range: std::ops::Range<usize>,
    ) -> Self {
        let start = content.floor_char_boundary(range.start.min(content.len()));
        let end = content.floor_char_boundary(
            range
                .end
                .clamp(start, content.len())
                .min(start.saturating_add(super::MAX_ANALYSIS_BYTES)),
        );
        let line = u32::try_from(content[..start].bytes().filter(|c| *c == b'\n').count())
            .unwrap_or(u32::MAX - 1)
            + 1;
        Self {
            mask: None,
            citation: Citation::Artifact { id, updated },
            title,
            provenance: format!(
                "Saved artifact · current version · bytes {start}–{end} of {}",
                content.len()
            ),
            wording: content[start..end].into(),
            first_line: line,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::graph::{DomainProperty, EntityKind, VocabularyId};
    #[test]
    fn narrow_agent_excerpt_finds_late_identifiers_and_inflected_words() {
        let text = format!(
            "{}\nCafé: les maisons près de SITE-778.",
            "ø\n".repeat(4000)
        );
        for query in ["site-778", "maison"] {
            let range = super::super::Language::French.excerpt_range(&text, query, 96);
            assert!(text[range].contains("les maisons près de SITE-778."));
        }
    }
    #[test]
    fn linked_sources_match_declared_predicates_and_classes_and_preserve_line_evidence() {
        let domain = |s: &str| EntityKind::Domain(VocabularyId::try_from(s.to_owned()).unwrap());
        let options = AnalysisOptions {
            properties: vec![DomainProperty {
                id: "food:grown_by".to_owned().try_into().unwrap(),
                subject: domain("product"),
                object: domain("grower"),
            }],
            ..Default::default()
        };
        let headers = "subject,predicate,object,subject_type,object_type\n";
        let sources=[
            ("Fruit","food:orange,food:grown_by,farm:g1,product,grower\nfood:orange,food:grown_by,food:vitamin_c,product,nutrient"),
            ("Farm","farm:g1,place:located_in,place:orchard,grower,location")
        ].into_iter().map(|(name,rows)|Excerpt::artifact(name.into(),Timestamp(1),name.into(),&format!("{headers}{rows}"),0)).collect();
        let result = Exploration::from_excerpts(sources, &options, vec![]).unwrap();
        assert_eq!(result.readings[0].analysis.statements.len(), 1);
        assert!(
            result.readings[0]
                .analysis
                .findings
                .iter()
                .any(|f| f.from == 3 && f.detail.contains("Excluded"))
        );
        assert!(
            result
                .shared
                .iter()
                .any(|p| p.label.contains("farm:g1 → place:located_in")
                    && p.sources
                        == [
                            SourceLine {
                                reading: 0,
                                line: 2
                            },
                            SourceLine {
                                reading: 1,
                                line: 2
                            }
                        ])
        );
    }
}
