//! A relation has two ordered ends and an explicit predicate. Keep source claims
//! distinct from a reviewed edge in the folder graph.
use super::{Finding, FindingKind};
use serde::Serialize;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Statement {
    pub subject: String,
    pub subject_type: Option<String>,
    pub object_type: Option<String>,
    pub predicate: String,
    pub object: String,
    pub line: u32,
    pub explicit: bool,
}
impl Statement {
    pub(super) fn finding(&self) -> Finding {
        let origin = if self.explicit {
            "Named relation in the table"
        } else {
            "Possible relation from wording"
        };
        let typed = |term: &str, kind: Option<&str>| {
            kind.map_or_else(|| term.to_owned(), |class| format!("{term} [{class}]"))
        };
        Finding {
            kind: FindingKind::Relation,
            from: self.line,
            to: self.line,
            detail: format!(
                "{origin} · {} → {} → {}",
                typed(&self.subject, self.subject_type.as_deref()),
                self.predicate,
                typed(&self.object, self.object_type.as_deref())
            ),
        }
    }
}
pub(super) fn inspect(
    text: &str,
    table: Option<&super::table::Table>,
    properties: &[crate::file_index::graph::DomainProperty],
    findings: &mut Vec<Finding>,
) -> Vec<Statement> {
    let mut statements = Vec::new();
    if let Some(table) = table {
        let header = &table.header;
        let column = |name: &str| header.iter().position(|h| h.eq_ignore_ascii_case(name));
        if let (Some(s), Some(p), Some(o)) =
            (column("subject"), column("predicate"), column("object"))
        {
            for row in &table.rows {
                if [s, p, o]
                    .iter()
                    .any(|&i| row[i].is_empty() || row[i].len() > 256)
                {
                    continue;
                }
                let read_class =
                    |col: Option<usize>| col.map(|i| row[i].trim()).filter(|v| !v.is_empty());
                let subject_type = read_class(column("subject_type"));
                let object_type = read_class(column("object_type"));
                let line = row
                    .position()
                    .and_then(|p| u32::try_from(p.line()).ok())
                    .unwrap_or(1);
                if let Some(rule) = properties.iter().find(|rule| rule.id.as_ref() == &row[p])
                    && (!subject_type.is_some_and(|t| rule.subject.matches_source_class(t))
                        || !object_type.is_some_and(|t| rule.object.matches_source_class(t)))
                {
                    if findings.len() < 32 {
                        findings.push(Finding{kind:FindingKind::Coverage,from:line,to:line,detail:format!("Excluded {} from named connections. This property requires {} → {}.",&row[p],rule.subject.label(),rule.object.label())});
                    }
                    continue;
                }
                statements.push(Statement {
                    subject: row[s].into(),
                    subject_type: subject_type.map(str::to_owned),
                    object_type: object_type.map(str::to_owned),
                    predicate: row[p].into(),
                    object: row[o].into(),
                    line: row
                        .position()
                        .and_then(|p| u32::try_from(p.line()).ok())
                        .unwrap_or(1),
                    explicit: true,
                });
                if statements.len() == 32 {
                    break;
                }
            }
            return statements;
        }
    }
    for (i, line) in text.lines().enumerate() {
        if line.len() > 1024 {
            continue;
        }
        for sentence in line.split_terminator(['.', ';']) {
            let sentence = sentence.trim();
            if sentence.len() > 300 {
                continue;
            }
            let lower = sentence.to_ascii_lowercase();
            for (marker, predicate) in [
                (" is a kind of ", "is_a"),
                (" is a type of ", "is_a"),
                (" is part of ", "part_of"),
                (" is grown by ", "grown_by"),
                (" is sold in ", "sold_in"),
                (" depends on ", "depends_on"),
                (" works for ", "works_for"),
                (" contains ", "contains"),
                (" is an ", "is_a"),
                (" is a ", "is_a"),
            ] {
                let Some(at) = lower.find(marker) else {
                    continue;
                };
                let subject = sentence[..at].trim();
                let object = sentence[at + marker.len()..].trim();
                if subject.is_empty() || object.is_empty() {
                    continue;
                }
                statements.push(Statement {
                    subject: subject.into(),
                    subject_type: None,
                    object_type: None,
                    predicate: predicate.into(),
                    object: object.into(),
                    line: u32::try_from(i + 1).expect("bounded lines"),
                    explicit: false,
                });
                break;
            }
            if statements.len() == 8 {
                return statements;
            }
        }
    }
    statements
}
