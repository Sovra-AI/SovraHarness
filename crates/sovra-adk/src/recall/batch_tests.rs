//! Real tool rounds must refresh every query in a batch without persisting evidence.

use super::*;
use adk_core::{LlmResponse, Part};
use sovra_core::{
    AgentConfig, Episode, EpisodeId, EpisodeKind, Fact, FactId, MemoryError, Timestamp,
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Reader {
    enabled: AtomicBool,
    queries: Mutex<Vec<MemorySearch>>,
}

#[async_trait::async_trait]
impl MemoryReader for Reader {
    async fn search_batch(
        &self,
        queries: Vec<MemorySearch>,
    ) -> Result<Vec<MemoryContext>, MemoryError> {
        let mut contexts = Vec::new();
        for query in queries {
            contexts.push(self.search(query).await?);
        }
        Ok(contexts)
    }
    async fn search(&self, query: MemorySearch) -> Result<MemoryContext, MemoryError> {
        self.queries.lock().unwrap().push(query.clone());
        let mut context = MemoryContext::default();
        if self.enabled.load(Ordering::SeqCst) && query.valid_at.is_some() {
            let source = Episode {
                id: EpisodeId(format!("episode:{}", query.text)),
                session_id: "synthetic-memory".into(),
                kind: EpisodeKind::UserMessage,
                content: format!("Evidence for {}", query.text),
                occurred_at: Timestamp(1),
            };
            context.push(
                &Fact {
                    id: FactId(format!("fact:{}", query.text)),
                    content: source.content.clone(),
                    entities: vec![],
                    source: Some(source.id.clone()),
                    replaces: None,
                    extraction: None,
                    created_at: Timestamp(2),
                    valid_at: None,
                    invalid_at: None,
                    expired_at: None,
                    stopped_at: None,
                },
                &source,
            );
        }
        Ok(context)
    }
}

struct Model {
    reader: Arc<Reader>,
    step: AtomicUsize,
}

fn call(query: &str) -> Part {
    Part::FunctionCall {
        name: lookup::NAME.into(),
        args: serde_json::json!({"query":query,"valid_at":"2025-01-01"}),
        id: Some(query.into()),
        thought_signature: None,
    }
}

#[async_trait::async_trait]
impl Llm for Model {
    fn name(&self) -> &'static str {
        "batch-memory-fixture"
    }
    async fn generate_content(
        &self,
        request: LlmRequest,
        _: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        let text = serde_json::to_string(&request).unwrap();
        let parts = match self.step.fetch_add(1, Ordering::SeqCst) {
            0 => vec![call("tea"), call("parcel")],
            1 => {
                assert!(
                    text.contains("Evidence for tea"),
                    "the first concurrent query was discarded"
                );
                assert!(text.contains("Evidence for parcel"));
                self.reader.enabled.store(false, Ordering::SeqCst);
                vec![call("tea")]
            }
            2 => {
                assert!(!text.contains("Evidence for tea"));
                assert!(!text.contains("Evidence for parcel"));
                vec![Part::Text {
                    text: "The saved evidence is no longer available.".into(),
                }]
            }
            _ => panic!("unexpected model round"),
        };
        Ok(Box::pin(futures::stream::once(async {
            Ok(LlmResponse::new(Content {
                role: "model".into(),
                parts,
            }))
        })))
    }
}

#[tokio::test]
async fn every_query_in_one_tool_round_is_attached_and_withdrawal_refreshes_the_batch() {
    let reader = Arc::new(Reader {
        enabled: AtomicBool::new(true),
        queries: Mutex::new(vec![]),
    });
    let model = Arc::new(Model {
        reader: reader.clone(),
        step: AtomicUsize::new(0),
    });
    let base =
        AdkHarness::with_llm_and_tools(&AgentConfig::default(), model.clone(), vec![]).unwrap();
    let (events, _received) = async_channel::unbounded();
    base.with_conversation_memory(
        Some(reader.clone()),
        "Compare the earlier details",
        Timestamp(100),
    )
    .unwrap()
    .run_prompt("batch-memory", "Compare the earlier details", &events)
    .await;
    assert_eq!(model.step.load(Ordering::SeqCst), 3);
    assert_eq!(
        reader
            .queries
            .lock()
            .unwrap()
            .iter()
            .map(|q| q.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Compare the earlier details", "tea", "parcel", "tea"]
    );
}

#[tokio::test]
async fn lookup_payloads_share_one_byte_budget_without_truncating_quotes_or_losing_filters() {
    let reader = Reader {
        enabled: AtomicBool::new(true),
        queries: Mutex::new(vec![]),
    };
    for count in [1, 4] {
        let queries = (0..count)
            .map(|index| MemorySearch {
                text: format!("query-{index}"),
                entity: Some(format!("person-{index}")),
                valid_at: Some(Timestamp(index)),
                known_at: Some(Timestamp(index + 1)),
            })
            .collect::<Vec<_>>();
        let mut contexts = Vec::new();
        let quote = "full quote é ".repeat(95);
        for query in &queries {
            let mut context = reader.search(query.clone()).await.unwrap();
            let mut item = context.memories[0].clone();
            item.quote = quote.clone();
            item.statement = quote.clone();
            context.memories = (0..8)
                .map(|index| {
                    let mut item = item.clone();
                    item.fact_id = format!("fact:{}-{index}", query.text);
                    item
                })
                .collect();
            contexts.push(context);
        }
        let data = evidence::encode(queries.clone(), contexts);
        assert!(data.len() <= MAX_MEMORY_CONTEXT_BYTES);
        let value: serde_json::Value = serde_json::from_str(&data).unwrap();
        let entries = if count == 1 {
            vec![serde_json::json!({"query":value["query"],"evidence":value})]
        } else {
            value["lookups"].as_array().unwrap().clone()
        };
        assert_eq!(
            entries.len(),
            usize::try_from(count).expect("batch size fits usize")
        );
        for (entry, query) in entries.iter().zip(&queries) {
            assert_eq!(entry["query"], serde_json::to_value(query).unwrap());
            let items = entry["evidence"]["memories"].as_array().unwrap();
            assert_ne!(items.len(), 0);
            for item in items {
                assert_eq!(item["quote"], quote);
            }
            assert!(
                entry["evidence"]["notice"]
                    .as_str()
                    .unwrap()
                    .contains("bounded selection")
            );
        }
    }
}

struct MissingIdModel(AtomicUsize);
#[async_trait::async_trait]
impl Llm for MissingIdModel {
    fn name(&self) -> &'static str {
        "missing-call-id"
    }
    async fn generate_content(
        &self,
        request: LlmRequest,
        _: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        let content = if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            Content {
                role: "model".into(),
                parts: vec![Part::FunctionCall {
                    name: lookup::NAME.into(),
                    args: serde_json::json!({"query":"tea"}),
                    id: None,
                    thought_signature: None,
                }],
            }
        } else {
            let parts = request
                .contents
                .iter()
                .flat_map(|content| &content.parts)
                .collect::<Vec<_>>();
            let call_id = parts
                .iter()
                .find_map(|part| match part {
                    Part::FunctionCall { id, .. } => id.as_deref(),
                    _ => None,
                })
                .expect("provider history needs a call ID");
            assert_ne!(call_id.trim(), "");
            let result_id = parts
                .iter()
                .find_map(|part| match part {
                    Part::FunctionResponse { id, .. } => id.as_deref(),
                    _ => None,
                })
                .expect("tool result needs a call ID");
            assert_eq!(call_id, result_id);
            Content::new("model").with_text("No permitted evidence was found.")
        };
        Ok(Box::pin(futures::stream::iter([Ok(LlmResponse::new(
            content,
        ))])))
    }
}
#[tokio::test]
async fn a_provider_lookup_without_an_id_keeps_a_matching_call_and_result_on_the_next_round() {
    let model = Arc::new(MissingIdModel(AtomicUsize::new(0)));
    let reader = Arc::new(Reader {
        enabled: AtomicBool::new(false),
        queries: Mutex::new(vec![]),
    });
    let base =
        AdkHarness::with_llm_and_tools(&AgentConfig::default(), model.clone(), vec![]).unwrap();
    let (events, _received) = async_channel::unbounded();
    base.with_conversation_memory(Some(reader), "Find tea", Timestamp(100))
        .unwrap()
        .run_prompt("missing-id", "Find tea", &events)
        .await;
    assert_eq!(model.0.load(Ordering::SeqCst), 2);
}
