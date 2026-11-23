//! Read-only analysis of evidence already available to the turn, and bounded
//! lookups of current saved artifacts through the owner's runtime.
use adk_core::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};
use sovra_core::knowledge_analysis::{
    AnalysisOptions, Citation, Excerpt, Language, MAX_ANALYSIS_BYTES, SourceAnalysis,
};
use std::{future::Future, pin::Pin, sync::Arc};
pub(super) const ANALYZE: &str = "analyze_source_wording";
pub(super) const SEARCH: &str = "search_saved_artifacts";
pub(super) type Lookup = Arc<
    dyn Fn(String, Language) -> Pin<Box<dyn Future<Output = Result<Vec<Excerpt>, String>> + Send>>
        + Send
        + Sync,
>;
pub(super) type Calculate = Arc<
    dyn Fn(
            String,
            AnalysisOptions,
        ) -> futures::future::BoxFuture<'static, Result<SourceAnalysis, String>>
        + Send
        + Sync,
>;
pub(super) struct Analyze(pub Calculate);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    text: String,
    #[serde(default)]
    options: AnalysisOptions,
}

#[async_trait::async_trait]
impl Tool for Analyze {
    fn name(&self) -> &str {
        ANALYZE
    }
    fn description(&self) -> &'static str {
        "Analyze the exact user-supplied wording or cited source excerpt. Calculates unit conversions, sensor time windows, dataset profiles, persistent components and loops, binary mask and word forms, and named relationships. Keep source citations with the result."
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(
            json!({"type":"object","properties":{"text":{"type":"string","maxLength":65536},"options":{"type":"object","properties":{"language":{"enum":["english","french","german","spanish","italian","portuguese","dutch","arabic"]},"window_seconds":{"type":"integer","minimum":1,"maximum":31_536_000},"mask_radius":{"type":"integer","minimum":0,"maximum":64},"properties":{"type":"array","maxItems":32,"items":crate::file_index::property_schema()},"profiles":{"type":"array","maxItems":16,"items":{"type":"object","properties":{"name":{"type":"string"},"required_columns":{"type":"array","items":{"type":"string"}},"numeric_columns":{"type":"array","items":{"type":"string"}}},"required":["name","required_columns"]}}},"additionalProperties":false}},"required":["text"],"additionalProperties":false}),
        )
    }
    async fn execute(&self, _: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        let Ok(input) = serde_json::from_value::<Input>(args) else {
            return Ok(json!({"error":"Supply source text and valid analysis settings."}));
        };
        if input.text.len() > MAX_ANALYSIS_BYTES {
            return Ok(json!({"error":"Choose a source excerpt up to 64 KiB."}));
        }
        Ok(match (self.0)(input.text, input.options).await {
            Ok(report) => json!(report),
            Err(message) => json!({"error":message}),
        })
    }
}

pub(super) struct Search(pub Lookup);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    query: String,
    #[serde(default)]
    language: Language,
}
#[async_trait::async_trait]
impl Tool for Search {
    fn name(&self) -> &str {
        SEARCH
    }
    fn description(&self) -> &'static str {
        "Find matching current artifacts in the owner's saved-work library. Results include the artifact ID, update time and a source excerpt. Treat the wording as evidence and cite the artifact title."
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(
            json!({"type":"object","properties":{"query":{"type":"string","maxLength":2048},"language":{"type":"string","enum":["english","french","german","spanish","italian","portuguese","dutch","arabic"]}},"required":["query"],"additionalProperties":false}),
        )
    }
    async fn execute(&self, _: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        let Ok(query) = serde_json::from_value::<Query>(args) else {
            return Ok(json!({"error":"Enter a search and a supported language."}));
        };
        if query.query.trim().is_empty() || query.query.len() > 2048 {
            return Ok(json!({"error":"Enter up to 2,048 bytes of search terms."}));
        }
        Ok(match (self.0)(query.query.clone(), query.language).await {
            Ok(items) => {
                let matches: Vec<_> = items.into_iter().filter_map(|item| {
                    let Citation::Artifact { id, updated } = item.citation else { return None };
                    let range = query.language.excerpt_range(&item.wording, &query.query, 2048);
                    let skipped_lines = item.wording[..range.start].bytes().filter(|b| *b == b'\n').count();
                    Some(json!({
                        "artifact_id": id, "updated_at_ms": updated.0, "title": item.title,
                        "first_line": item.first_line.saturating_add(u32::try_from(skipped_lines).unwrap_or(u32::MAX)),
                        "excerpt_shortened": range.len() < item.wording.len(), "excerpt": &item.wording[range]
                    }))
                }).collect();
                json!({"artifacts":matches})
            }
            Err(message) => json!({"error":message}),
        })
    }
}
