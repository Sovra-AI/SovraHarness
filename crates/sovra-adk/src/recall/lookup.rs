//! A per-turn query receipt. Evidence itself never enters a persisted tool response.

use adk_core::{LlmRequest, Part, Tool, ToolContext};
use serde_json::{Value, json};
use sovra_core::{MemoryError, MemorySearch, Timestamp};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use time::{Date, OffsetDateTime, format_description::well_known::Rfc3339};

pub(crate) const NAME: &str = "search_saved_details";
const LIMIT: usize = sovra_core::MAX_MEMORY_SEARCHES;
pub(super) const GUIDANCE: &str = "When a question or task depends on the user's circumstances or earlier interactions and the supplied evidence does not answer it, call search_saved_details before saying you do not know. Reformulate with concise alternative wording and synonyms, rather than repeating a failed question verbatim. Omit unknown entity filters. Search again when results are incomplete; never treat empty results as proof nothing is saved. For a historical question, first call search_saved_details with the requested date; initial automatic evidence describes current beliefs, not the past. For ambiguous dates or people, ask for clarification instead of inventing precision. valid_at asks what applied then; known_at asks what had been learned by then. These are distinct. Use explicit entities only when supported by the conversation or evidence. A tool receipt schedules the next fresh evidence lookup; the receipt is not evidence. Compare dates with separate lookups when needed, citing each supported statement. Withdrawn versions remain excluded at every date.";

pub(super) struct Lookup {
    invocation: u64,
    calls: AtomicUsize,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    query: String,
    entity: Option<String>,
    valid_at: Option<String>,
    known_at: Option<String>,
}

fn parse(args: Value) -> Result<MemorySearch, MemoryError> {
    let args: Arguments = serde_json::from_value(args).map_err(|_| {
        MemoryError::InvalidInput(
            "Use query and optional entity, valid_at and known_at fields.".into(),
        )
    })?;
    let query = MemorySearch {
        text: args.query,
        entity: args.entity,
        valid_at: args.valid_at.as_deref().map(parse_time).transpose()?,
        known_at: args.known_at.as_deref().map(parse_time).transpose()?,
    };
    query.validate()?;
    Ok(query)
}

fn parse_time(value: &str) -> Result<Timestamp, MemoryError> {
    let invalid = || {
        MemoryError::InvalidInput("Use YYYY-MM-DD (midnight UTC), or an RFC 3339 date-time with an explicit UTC offset. Do not guess an ambiguous date.".into())
    };
    if value.len() > 40 {
        return Err(invalid());
    }
    let date = time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
        .expect("static date format");
    let at = OffsetDateTime::parse(value, &Rfc3339)
        .or_else(|_| Date::parse(value, &date).map(|date| date.midnight().assume_utc()))
        .map_err(|_| invalid())?;
    i64::try_from(at.unix_timestamp_nanos().div_euclid(1_000_000))
        .map(Timestamp)
        .map_err(|_| invalid())
}

pub(super) fn format_time(at: Timestamp) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(at.millis()) * 1_000_000)
        .ok()
        .and_then(|at| at.format(&Rfc3339).ok())
        .unwrap_or_else(|| format!("Unix milliseconds {}", at.millis()))
}

impl Lookup {
    pub(super) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            invocation: NEXT.fetch_add(1, Ordering::Relaxed),
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn queries(
        &self,
        request: &LlmRequest,
    ) -> Result<Option<Vec<MemorySearch>>, MemoryError> {
        let mut receipts = Vec::new();
        for part in request
            .contents
            .iter()
            .rev()
            .flat_map(|content| content.parts.iter().rev())
        {
            match part {
                Part::FunctionResponse {
                    function_response, ..
                } if function_response.name == NAME
                    && function_response.response["invocation"].as_u64()
                        == Some(self.invocation) =>
                {
                    receipts.push(&function_response.response);
                }
                // Responses can be separate Content records. Their originating
                // model call bounds the round, independently of provider roles.
                Part::FunctionCall { .. } if !receipts.is_empty() => break,
                _ => {}
            }
        }
        if receipts.is_empty() {
            return Ok(None);
        }
        receipts.reverse();
        let queries = receipts
            .into_iter()
            .map(|receipt| parse(receipt["lookup"].clone()))
            .collect::<Result<Vec<_>, _>>()?;
        sovra_core::validate_memory_searches(&queries)?;
        Ok(Some(queries))
    }

    fn receipt(&self, args: &Value) -> Value {
        if self.calls.fetch_add(1, Ordering::Relaxed) >= LIMIT {
            return json!({"invocation": self.invocation, "error": "The four saved-detail lookups for this turn are used. Explain any remaining uncertainty."});
        }
        match parse(args.clone()) {
            Ok(_) => {
                json!({"invocation": self.invocation, "lookup": args, "notice": "Query accepted. Read the fresh saved evidence attached to the next model request. This receipt contains no remembered facts."})
            }
            Err(error) => json!({"invocation": self.invocation, "error": error.to_string()}),
        }
    }
}

#[async_trait::async_trait]
impl Tool for Lookup {
    fn name(&self) -> &'static str {
        NAME
    }
    fn description(&self) -> &'static str {
        "Request fresh conversation evidence for a focused question, optionally about an exact person/entity, a past valid date, or what was known by a date. Dates accept YYYY-MM-DD (00:00 UTC) or RFC 3339 with an explicit offset. Only use constraints supported by the user's question or context; clarify ambiguous dates. At most four lookups per turn, shared with helpers. The receipt contains only the query; fresh evidence and citations appear in the next model request. No results means missing evidence, never proof that something did not happen. No files or external documents are searched."
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({"type":"object", "properties": {
            "query": {"type":"string", "maxLength":4096},
            "entity": {"type":"string", "maxLength":256, "description":"Exact supported entity name; omit when unknown."},
            "valid_at": {"type":"string", "maxLength":40, "description":"When the claim applied in the world. Omit for current beliefs."},
            "known_at": {"type":"string", "maxLength":40, "description":"What Sovra had learned by this date. Distinct from when an event occurred; omit unless requested."}
        }, "required":["query"], "additionalProperties":false}))
    }
    async fn execute(&self, _: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        Ok(self.receipt(&args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookup_dates_are_explicit_and_validated_without_local_clock_or_timezone() {
        assert_eq!(parse_time("1970-01-01").unwrap(), Timestamp(0));
        assert_eq!(
            parse_time("1969-12-31T23:59:59.999999999Z").unwrap(),
            Timestamp(-1)
        );
        assert_eq!(
            parse_time("1970-01-01T04:00:00+04:00").unwrap(),
            Timestamp(0)
        );
        assert_eq!(
            parse_time("1969-12-31T23:59:59Z").unwrap(),
            Timestamp(-1000)
        );
        assert_eq!(format_time(Timestamp(0)), "1970-01-01T00:00:00Z");
        assert!(format_time(Timestamp(i64::MAX)).starts_with("Unix milliseconds"));
        assert!(parse_time("2024-02-29").is_ok());
        for invalid in [
            "2025-02-29",
            "yesterday",
            "June",
            "2025-04-31",
            "2025-01-01T00:00:00",
            "2025-01-01 trailing",
        ] {
            assert!(parse_time(invalid).is_err(), "{invalid}");
        }
        for invalid in [
            json!({"query":"tea","entity":3}),
            json!({"query":"tea","extra":true}),
            json!({"query":"x".repeat(4097)}),
            json!({"query":"tea","valid_at":"yesterday"}),
        ] {
            assert!(parse(invalid).is_err());
        }
    }
    #[test]
    fn query_receipts_are_bounded_and_do_not_contain_evidence() {
        let lookup = Lookup::new();
        for _ in 0..LIMIT {
            let result = lookup.receipt(&json!({"query":"tea", "valid_at":"2024-01-01"}));
            assert!(result.get("lookup").is_some());
            assert!(result.get("memories").is_none());
        }
        assert!(
            lookup
                .receipt(&json!({"query":"tea"}))
                .get("error")
                .is_some()
        );
        let query = parse(json!({"query":"tea", "entity":"Maya", "valid_at":"2024-01-01", "known_at":"2025-01-01"})).unwrap();
        assert_eq!(query.entity.as_deref(), Some("Maya"));
        assert!(query.known_at > query.valid_at);
    }

    #[test]
    fn invalid_receipts_do_not_fall_back_to_current_beliefs_and_old_receipts_are_ignored() {
        let lookup = Lookup::new();
        let mut request = LlmRequest::new("test", vec![]);
        let content = |receipt| adk_core::Content {
            role: "tool".into(),
            parts: vec![Part::FunctionResponse {
                function_response: adk_core::FunctionResponseData::new(NAME, receipt),
                id: Some("query".into()),
                annotations: None,
            }],
        };
        request.contents.push(content(
            lookup.receipt(&json!({"query":"tea", "valid_at":"2024-01-01"})),
        ));
        assert!(
            lookup.queries(&request).unwrap().unwrap()[0]
                .valid_at
                .is_some()
        );
        request.contents.push(content(
            lookup.receipt(&json!({"query":"tea", "valid_at":"yesterday"})),
        ));
        assert!(lookup.queries(&request).is_err());
        let next = Lookup::new();
        assert!(next.queries(&request).unwrap().is_none());
    }
}
