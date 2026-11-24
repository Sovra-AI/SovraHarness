//! What Sovra worked out from indexed files on its own: an overview a helper can
//! read first, and the findings themselves one kind at a time. Read-only.

use super::{Arc, IndexError, IndexedFiles, TOOL_NAMES, Tool, ToolContext, Value, json};
use sovra_core::file_index::automatic::{DiscoveredFinding, DiscoveryOverview, FindingQuery};
use sovra_core::knowledge_analysis::FindingKind;

/// Long lists are cut so the overview stays small; the cut is reported.
const LIST_LIMIT: usize = 30;

#[derive(Clone, Copy)]
enum Operation {
    Overview,
    List,
}

struct FindingTool {
    reader: Arc<dyn IndexedFiles>,
    operation: Operation,
}

pub(super) fn tools(reader: &Arc<dyn IndexedFiles>) -> Vec<Arc<dyn Tool>> {
    [Operation::Overview, Operation::List]
        .into_iter()
        .map(|operation| {
            Arc::new(FindingTool {
                reader: reader.clone(),
                operation,
            }) as Arc<dyn Tool>
        })
        .collect()
}

#[async_trait::async_trait]
impl Tool for FindingTool {
    fn name(&self) -> &str {
        match self.operation {
            Operation::Overview => TOOL_NAMES[9],
            Operation::List => TOOL_NAMES[10],
        }
    }
    fn description(&self) -> &str {
        match self.operation {
            Operation::Overview => {
                "See what is in the connected folders before searching: how many files Sovra sorted, what kinds they are (documents, spreadsheets, photos…), which parts of life they cover, the kinds of things they mention, the relationship types saved, and how many measurements, dated readings, tables and other findings exist. Call this first for broad questions such as 'what do my files say about…', 'what data do I have', or when you need to know which finding kinds or relationship types to ask for. Each relationship type includes the exact relation value for query_file_knowledge; each finding kind includes the kind value for list_file_findings."
            }
            Operation::List => {
                "List findings Sovra worked out from the connected files without a model: measurements with units, dated readings grouped by time window with averages, what each table holds, relationships named in tables or sentences, and more. Choose one kind from overview_file_knowledge; add contains to keep only findings whose wording or file path includes that text (for example a unit, a name, or a file name). Each finding has the file path, extracted-text line and a reference for read_indexed_file, so read the passage before relying on a finding for an exact answer. Findings are automatic readings, not verified facts. Follow next for older findings."
            }
        }
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(match self.operation {
            Operation::Overview => {
                json!({"type":"object","properties":{},"additionalProperties":false})
            }
            Operation::List => json!({"type":"object","properties":{
                "kind":{"type":"string","enum":FindingKind::ALL.map(FindingKind::id)},
                "contains":{"type":"string","maxLength":FindingQuery::MAX_TEXT,"description":"Keep only findings whose wording or file path contains this text, ignoring letter case."},
                "next":{"type":"integer","minimum":1,"description":"The next value returned by the previous page."}
            },"required":["kind"],"additionalProperties":false}),
        })
    }
    async fn execute(&self, _: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        let result = match self.operation {
            Operation::Overview => self.reader.discoveries().await.map(|o| overview(&o)),
            Operation::List => match query(&args) {
                Ok(query) => self.reader.findings(query).await.map(|page| {
                    json!({
                        "findings": page.items.iter().map(finding).collect::<Vec<_>>(),
                        "next": page.next,
                        "source_kind": "automatic_reading",
                        "source_claims_verified": false,
                        "notice": "Automatic readings of the file text. Cite the path and line. Read the passage with read_indexed_file before giving an exact number or date. An empty page is not proof the fact is absent: try another kind, other wording, or search_indexed_files."
                    })
                }),
                Err(error) => Err(error),
            },
        };
        Ok(result.unwrap_or_else(|e| json!({"error":e.to_string()})))
    }
}

fn query(args: &Value) -> Result<FindingQuery, IndexError> {
    let invalid = || {
        IndexError(format!(
            "Use kind (one of {}) with optional contains text and next.",
            FindingKind::ALL.map(FindingKind::id).join(", ")
        ))
    };
    let object = args.as_object().ok_or_else(invalid)?;
    if object
        .keys()
        .any(|k| !matches!(k.as_str(), "kind" | "contains" | "next"))
    {
        return Err(invalid());
    }
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .and_then(FindingKind::from_id)
        .ok_or_else(invalid)?;
    let contains = match object.get("contains") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.as_str()),
        Some(_) => return Err(invalid()),
    };
    let before = match object.get("next") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_i64().filter(|n| *n >= 1).ok_or_else(invalid)?),
    };
    FindingQuery::new(kind, contains, before)
}

fn finding(item: &DiscoveredFinding) -> Value {
    json!({
        "kind": item.kind.id(),
        "detail": item.detail,
        "path": item.path,
        "line": item.line,
        "reference": item.reference,
    })
}

/// Keep the first `LIST_LIMIT` rows and say how many were left out.
fn capped<T>(items: &[T], row: impl Fn(&T) -> Value) -> (Vec<Value>, usize) {
    (
        items.iter().take(LIST_LIMIT).map(row).collect(),
        items.len().saturating_sub(LIST_LIMIT),
    )
}

fn overview(report: &DiscoveryOverview) -> Value {
    let files: u64 = report.kinds.iter().map(|k| k.1).sum();
    let (kinds_of_things, more_things) = capped(
        &report.subjects,
        |(kind, n)| json!({"kind": kind, "label": kind.display_name(), "mentions": n}),
    );
    let (relationship_types, more_types) = capped(
        &report.predicates,
        |(kind, n)| json!({"relation": kind, "label": kind.display_name(), "count": n}),
    );
    let mut notices = vec![
        "Counts cover every connected folder. Use list_file_findings for one finding kind, query_file_knowledge with a relation value for saved relationships, and search_indexed_files for the text itself.".to_owned(),
    ];
    if report.working {
        notices.push("Sovra is still reading some files, so these counts will grow.".into());
    }
    if more_things + more_types > 0 {
        notices.push(format!(
            "Left out {more_things} rarer kinds of things and {more_types} rarer relationship types."
        ));
    }
    json!({
        "files_sorted": files,
        "still_reading": report.working,
        "file_kinds": report.kinds.iter().map(|(kind, n)| json!({"kind": kind, "label": kind.label(), "files": n})).collect::<Vec<_>>(),
        "life_areas": report.areas.iter().map(|(area, n)| json!({"area": area, "label": area.label(), "files": n})).collect::<Vec<_>>(),
        "kinds_of_things": kinds_of_things,
        "relationship_types": relationship_types,
        "findings": report.counts.iter().map(|(kind, n)| json!({"kind": kind.id(), "count": n, "meaning": kind.meaning()})).collect::<Vec<_>>(),
        "examples": report.examples.iter().map(finding).collect::<Vec<_>>(),
        "source_kind": "automatic_reading",
        "notices": notices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finding_arguments_are_bounded_and_named() {
        let q = query(&json!({"kind":"measurement","contains":"  KG ","next":7})).unwrap();
        assert_eq!(q.kind, FindingKind::Measurement);
        assert_eq!(q.contains.as_deref(), Some("kg"));
        assert_eq!(q.before, Some(7));
        assert!(
            query(&json!({"kind":"dated_series","contains":""}))
                .unwrap()
                .contains
                .is_none()
        );
        assert!(query(&json!({"kind":"Measurement"})).is_err());
        assert!(query(&json!({"kind":"table","sql":"x"})).is_err());
        assert!(query(&json!({"kind":"table","next":0})).is_err());
        assert!(query(&json!({"kind":"table","contains":"x".repeat(241)})).is_err());
        for kind in FindingKind::ALL {
            assert_eq!(FindingKind::from_id(kind.id()), Some(kind));
        }
    }
}
