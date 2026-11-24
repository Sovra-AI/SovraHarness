//! Local analysis of bounded source text. All source adapters use these rules.
//! Findings refer to supplied lines and retain the original evidence.

mod explore;
pub use explore::{
    Citation, Excerpt, Exploration, ExploreRequest, SourceConnection, SourceLine, SourceReading,
};
pub mod mask;
pub mod measurement;
mod relations;
mod table;
pub use relations::Statement;
pub mod topology;
use rust_stemmers::{Algorithm, Stemmer};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_ANALYSIS_BYTES: usize = 64 * 1024;

/// A single identifier must match a complete token even when nearby records have the same prefix.
pub fn is_identifier(query: &str) -> bool {
    !query.chars().any(char::is_whitespace)
        && query
            .chars()
            .any(|c| c.is_numeric() || matches!(c, '-' | '_' | '/' | '.' | ':' | '\\'))
}

pub fn contains_identifier(text: &str, query: &str) -> bool {
    if query.is_empty() {
        return false;
    }
    let boundary = |c: char| !c.is_alphanumeric() && c != '_';
    let mut at_start = true;
    for (i, c) in text.char_indices() {
        if at_start
            && let Some(tail) = text.get(i + query.len()..)
            && text[i..i + query.len()].eq_ignore_ascii_case(query)
            && tail.chars().next().is_none_or(boundary)
        {
            return true;
        }
        at_start = boundary(c);
    }
    false
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    English,
    French,
    German,
    Spanish,
    Italian,
    Portuguese,
    Dutch,
    Arabic,
}

impl Language {
    pub const fn name(self) -> &'static str {
        match self {
            Self::English => "english",
            Self::French => "french",
            Self::German => "german",
            Self::Spanish => "spanish",
            Self::Italian => "italian",
            Self::Portuguese => "portuguese",
            Self::Dutch => "dutch",
            Self::Arabic => "arabic",
        }
    }
    pub fn stem_text(self, text: &str) -> String {
        use std::fmt::Write as _;
        let stemmer = self.stemmer();
        let mut result = String::with_capacity(text.len());
        for word in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            if !result.is_empty() {
                result.push(' ');
            }
            let _ = write!(result, "{}", stemmer.stem(&word.to_lowercase()));
        }
        result
    }
    /// Find a display window around actual matching wording, retaining UTF-8 byte offsets.
    pub fn excerpt_range(self, text: &str, query: &str, limit: usize) -> std::ops::Range<usize> {
        if text.len() <= limit {
            return 0..text.len();
        }
        let exact = text.find(query).or_else(|| {
            (query.is_ascii() && !query.is_empty())
                .then(|| {
                    text.as_bytes()
                        .windows(query.len())
                        .position(|b| b.eq_ignore_ascii_case(query.as_bytes()))
                })
                .flatten()
        });
        let offset = exact.unwrap_or_else(|| {
            let terms = self.stem_text(query);
            let stemmer = self.stemmer();
            text.split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .find(|word| {
                    let lower = word.to_lowercase();
                    let stem = stemmer.stem(&lower);
                    terms.split_whitespace().any(|term| term == stem)
                })
                .map_or(0, |word| word.as_ptr().addr() - text.as_ptr().addr())
        });
        let take_before = (limit / 3).min(limit.saturating_sub(query.len()));
        let near = text.floor_char_boundary(offset.saturating_sub(take_before));
        let start = text[near..offset].find('\n').map_or(near, |i| near + i + 1);
        start..text.floor_char_boundary(start.saturating_add(limit).min(text.len()))
    }
    pub fn stemmer(self) -> Stemmer {
        Stemmer::create(match self {
            Self::English => Algorithm::English,
            Self::French => Algorithm::French,
            Self::German => Algorithm::German,
            Self::Spanish => Algorithm::Spanish,
            Self::Italian => Algorithm::Italian,
            Self::Portuguese => Algorithm::Portuguese,
            Self::Dutch => Algorithm::Dutch,
            Self::Arabic => Algorithm::Arabic,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnalysisOptions {
    pub language: Language,
    pub window_seconds: u32,
    pub mask_radius: u8,
    pub profiles: Vec<ProfileRule>,
    pub properties: Vec<crate::file_index::graph::DomainProperty>,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            language: Language::English,
            window_seconds: 86_400,
            mask_radius: 1,
            profiles: Vec::new(),
            properties: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRule {
    pub name: String,
    pub required_columns: Vec<String>,
    #[serde(default)]
    pub numeric_columns: Vec<String>,
}

impl AnalysisOptions {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=31_536_000).contains(&self.window_seconds) || self.mask_radius > 64 {
            return Err(
                "Choose a time window of 1 second to 365 days and a mask radius up to 64 pixels.",
            );
        }
        let mut ids = std::collections::BTreeSet::new();
        if self.properties.len() > 32 || self.properties.iter().any(|p| !ids.insert(p.id.as_ref()))
        {
            return Err(
                "Use up to 32 distinct property identifiers with a subject class and an object class.",
            );
        }
        if self.profiles.len() > 16
            || self.profiles.iter().any(|p| {
                p.name.trim().is_empty()
                    || p.name.len() > 100
                    || p.required_columns.is_empty()
                    || p.required_columns.len() + p.numeric_columns.len() > 64
                    || p.required_columns
                        .iter()
                        .chain(&p.numeric_columns)
                        .any(|c| c.trim().is_empty() || c.len() > 100)
            })
        {
            return Err("Use up to 16 named profiles with explicit column traits.");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FindingKind {
    Relation,
    Topology,
    Profile,
    Measurement,
    Morphology,
    Chronology,
    Coverage,
}
impl FindingKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Relation => "Relationship in the wording",
            Self::Topology => "Shape across distances",
            Self::Profile => "Type and traits",
            Self::Measurement => "Measured quantity",
            Self::Morphology => "Word or image shape",
            Self::Chronology => "Dates and time windows",
            Self::Coverage => "Reading limits",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub kind: FindingKind,
    pub from: u32,
    pub to: u32,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeasuredQuantity {
    #[serde(flatten)]
    pub quantity: measurement::Quantity,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceAnalysis {
    pub findings: Vec<Finding>,
    pub statements: Vec<Statement>,
    pub quantities: Vec<MeasuredQuantity>,
    pub bytes_read: usize,
}

pub fn analyze(text: &str, options: &AnalysisOptions) -> Result<SourceAnalysis, &'static str> {
    analyze_with_limits(text, options, AnalysisScope::Preview)
}

/// Analyze an indexing window, retaining every recognized relationship in that window.
pub fn analyze_indexed(text: &str) -> Result<SourceAnalysis, &'static str> {
    analyze_with_limits(
        text,
        &AnalysisOptions::default(),
        AnalysisScope::IndexWindow,
    )
}

#[derive(Clone, Copy)]
enum AnalysisScope {
    Preview,
    IndexWindow,
}
impl AnalysisScope {
    const fn limit(self, preview: usize) -> usize {
        match self {
            Self::Preview => preview,
            Self::IndexWindow => usize::MAX,
        }
    }
}

fn analyze_with_limits(
    text: &str,
    options: &AnalysisOptions,
    scope: AnalysisScope,
) -> Result<SourceAnalysis, &'static str> {
    options.validate()?;
    let end = text.floor_char_boundary(text.len().min(MAX_ANALYSIS_BYTES));
    let end = if end < text.len() {
        text[..end].rfind('\n').map_or(0, |n| n + 1)
    } else {
        end
    };
    let slice = &text[..end];
    let mut statements = Vec::new();
    let mut quantities = Vec::new();
    let mut findings = Vec::new();
    if !inspect_mask(slice, options.mask_radius, &mut findings) {
        let table = table::parse(slice);
        if let Some(data) = &table {
            table::inspect(data, options, &mut findings, &mut quantities, scope);
        }
        statements = relations::inspect(
            slice,
            table.as_ref(),
            &options.properties,
            &mut findings,
            scope,
        );
        findings.extend(statements.iter().map(Statement::finding));
        inspect_words(slice, options.language, &mut findings);
        inspect_statements(slice, &mut findings, &mut quantities, scope);
    }
    if end < text.len() {
        findings.push(Finding {
            kind: FindingKind::Coverage,
            from: 1,
            to: 1,
            detail: format!("Analysis covers the first {end} of {} bytes.", text.len()),
        });
    }
    Ok(SourceAnalysis {
        findings,
        statements,
        quantities,
        bytes_read: end,
    })
}

fn inspect_mask(text: &str, radius: u8, findings: &mut Vec<Finding>) -> bool {
    if text.split_whitespace().next() != Some("P1") {
        return false;
    }
    let mut tokens = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or_default())
        .flat_map(str::split_whitespace);
    let _ = tokens.next();
    let result = (|| {
        let width = tokens.next()?.parse::<usize>().ok()?;
        let height = tokens.next()?.parse::<usize>().ok()?;
        let pixels = tokens
            .map(|v| match v {
                "0" => Some(false),
                "1" => Some(true),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        let source = mask::Mask::new(width, height, pixels).ok()?;
        source.describe(radius).ok()
    })();
    findings.push(Finding {
        kind: if result.is_some() {
            FindingKind::Morphology
        } else {
            FindingKind::Coverage
        },
        from: 1,
        to: 1,
        detail: result
            .unwrap_or_else(|| "This P1 mask is incomplete or exceeds the analysis bounds.".into()),
    });
    true
}

fn inspect_words(text: &str, language: Language, findings: &mut Vec<Finding>) {
    let stemmer = language.stemmer();
    let mut forms = BTreeMap::new();
    let mut surface_change = None;
    for (i, line) in text.lines().enumerate() {
        let lineno = u32::try_from(i + 1).expect("analysis byte limit bounds lines");
        for word in line
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| (3..=64).contains(&w.len()) && w.chars().all(char::is_alphabetic))
        {
            let lower = word.to_lowercase();
            let stem = stemmer.stem(&lower);
            if surface_change.is_none()
                && stem != lower
                && !stem.is_empty()
                && lower.contains(stem.as_ref())
            {
                surface_change = Some(Finding {
                    kind: FindingKind::Morphology,
                    from: lineno,
                    to: lineno,
                    detail: format!(
                        "“{word}” is matched by its root “{stem}”, so a search finds other forms of it ({language:?})"
                    ),
                });
            }
            let entry = forms
                .entry(stem.into_owned())
                .or_insert((word, lineno, None));
            if entry.2.is_none() && !entry.0.eq_ignore_ascii_case(word) {
                entry.2 = Some((word, lineno));
            }
        }
    }
    findings.extend(surface_change);
    for (stem, (first, from, second)) in forms
        .into_iter()
        .filter(|(_, (_, _, s))| s.is_some())
        .take(4)
    {
        if let Some((other, to)) = second {
            findings.push(Finding {
                kind: FindingKind::Morphology,
                from,
                to,
                detail: format!(
                    "“{first}” and “{other}” are forms of one word (root “{stem}”), so a search for either finds both"
                ),
            });
        }
    }
}

fn inspect_statements(
    text: &str,
    findings: &mut Vec<Finding>,
    quantities: &mut Vec<MeasuredQuantity>,
    scope: AnalysisScope,
) {
    let mut measurements = 0;
    let mut dates = 0;
    for (i, line) in text.lines().enumerate() {
        let number = u32::try_from(i + 1).expect("analysis byte limit bounds lines");
        let mut add = |kind, detail| {
            findings.push(Finding {
                kind,
                from: number,
                to: number,
                detail,
            });
        };
        let words: Vec<_> = line.split_whitespace().collect();
        if !line.contains(',') && !line.contains('\t') {
            for pair in words.windows(2) {
                if measurements >= scope.limit(8) {
                    break;
                }
                if let Some(q) =
                    measurement::Quantity::parse(pair[0], pair[1].trim_end_matches(['.', ',', ';']))
                {
                    measurements += 1;
                    add(
                        FindingKind::Measurement,
                        format!("{} = {} {}", q.original, q.value.normalize(), q.unit),
                    );
                    quantities.push(MeasuredQuantity {
                        quantity: q,
                        line: number,
                    });
                }
            }
        }
        if dates < scope.limit(5) {
            for word in &words {
                let token = word.trim_matches(['(', ')', '.', ',', ';', '"']);
                if table::timestamp(token).is_some() {
                    dates += 1;
                    add(
                        FindingKind::Chronology,
                        format!("Date in the source wording · {token}"),
                    );
                    if matches!(scope, AnalysisScope::Preview) {
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readings_use_explicit_traits_calibration_units_and_event_time() {
        let csv = "timestamp,value,unit,instrument,calibration_date,reference,uncertainty\n2024-02-01T12:00:00Z,0.5,kg,scale A,2024-01-01,R1,0.001\n2024-02-01T01:00:00Z,500,g,scale A,2024-01-01,R1,1";
        let options = AnalysisOptions {
            profiles: vec![ProfileRule {
                name: "Weighing log".into(),
                required_columns: vec!["instrument".into()],
                numeric_columns: vec!["value".into()],
            }],
            ..Default::default()
        };
        let a = analyze(csv, &options).unwrap();
        let details: Vec<_> = a.findings.iter().map(|f| f.detail.as_str()).collect();
        assert!(
            details
                .iter()
                .any(|d| d.contains("Weighing log") && d.contains("measured quantities"))
        );
        assert!(
            details
                .iter()
                .any(|d| d.contains("average 500 g") && d.contains("2 readings"))
        );
        assert!(
            details.iter().any(|d| d.contains("scale A")
                && d.contains("R1")
                && d.contains("uncertainty ±1 g"))
        );
        assert!(
            analyze(
                csv,
                &AnalysisOptions {
                    window_seconds: 0,
                    ..options
                }
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod table_formats {
    use super::*;
    #[test]
    fn markdown_artifacts_and_fenced_notes_keep_the_source_lines_and_profile_ties() {
        let options = AnalysisOptions {
            profiles: ["record A", "record B"]
                .into_iter()
                .map(|name| ProfileRule {
                    name: name.into(),
                    required_columns: vec!["sensor".into()],
                    numeric_columns: vec!["value".into()],
                })
                .collect(),
            ..Default::default()
        };
        let md = "| sensor | value | unit |\n|---|---:|---|\n| left | 0.5 | kg |";
        let result = analyze(md, &options).unwrap();
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.kind == FindingKind::Measurement
                    && f.from == 3
                    && f.detail.contains("500 g"))
        );
        assert!(result.findings.iter().any(|f| {
            f.detail
                .contains("equally matching profiles: record A, record B")
        }));
        let data =
            "```csv\nsubject,predicate,object\ngarden:Juniper,botany:contains,botany:seed\n```";
        let result = analyze(data, &options).unwrap();
        assert_eq!(result.statements[0].line, 3);
        assert_eq!(result.statements[0].predicate, "botany:contains");
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.detail.starts_with("Table · "))
        );
    }
}
