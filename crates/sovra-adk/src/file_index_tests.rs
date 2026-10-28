//! The real ADK loop discovers a skill, searches, recovers from invalid calls,
//! reads cited evidence and returns control to the parent for synthesis.

use crate::AdkHarness;
use adk_core::{Content, FinishReason, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part};
use serde_json::{Value, json};
use sovra_core::{
    AgentConfig, Event,
    file_index::{
        CatalogEntry, CatalogPage, CatalogQuery, Confidence, EvidenceBasis, EvidenceTier,
        ExtractionState, FileHistory, FileHistoryQuery, FileObservation, HistoricalPassage,
        HistoryPassageRef, IndexError, IndexPassage, IndexProgress, IndexQuery, IndexStatus,
        IndexedFiles, IndexedRoot, ObservedPresence, PassageRef, ScanActivity,
        SourceRepresentation,
    },
    skills::SkillCatalog,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug, Default)]
struct Reader {
    searches: AtomicUsize,
    proposals: AtomicUsize,
}

fn passage() -> IndexPassage {
    IndexPassage {
        reference: PassageRef {
            root_id: "root".into(),
            file_id: "report".into(),
            version_id: "v1".into(),
            ordinal: 0,
        },
        path: "notes/budget.md".into(),
        text: "The orchid budget is 420 dollars.".into(),
        start_line: 7,
        end_line: 7,
        mime_type: "text/markdown".into(),
        note: None,
        observed_at: 100,
        modified_at: Some(90),
    }
}

#[async_trait::async_trait]
impl IndexedFiles for Reader {
    async fn propose_knowledge(
        &self,
        draft: sovra_core::file_index::graph::GraphDraft,
        origin: sovra_core::file_index::graph::GraphOrigin,
    ) -> Result<sovra_core::file_index::graph::KnowledgeProposal, IndexError> {
        draft.validate()?;
        origin.validate()?;
        assert_eq!(origin.session_id, "index-test");
        self.proposals.fetch_add(1, Ordering::SeqCst);
        Ok(sovra_core::file_index::graph::KnowledgeProposal {
            id: 1,
            state: sovra_core::file_index::graph::ProposalState::Pending,
            created_at: 100,
            reviewed_at: None,
            origin,
            draft,
            sources: vec![passage()],
            existing_entities: vec![],
            replaced_relations: vec![],
            required_roots: vec!["root".into()],
        })
    }
    async fn knowledge(
        &self,
        query: sovra_core::file_index::graph::GraphQuery,
    ) -> Result<sovra_core::file_index::graph::GraphPage, IndexError> {
        query.validate()?;
        Ok(sovra_core::file_index::graph::GraphPage {
            entities: vec![],
            relations: vec![],
            sources: vec![],
            truncated: false,
        })
    }
    async fn history(&self, query: FileHistoryQuery) -> Result<FileHistory, IndexError> {
        query.validate()?;
        assert_eq!(query.root_id, "root");
        assert_eq!(query.file_id.as_deref(), Some("report"));
        Ok(FileHistory {
            root_id: "root".into(),
            root_path: "/chosen/project".into(),
            file_id: "report".into(),
            current_path: passage().path,
            currently_present: true,
            coverage: sovra_core::file_index::CatalogCoverage {
                root_id: "root".into(),
                status: IndexStatus::Ready,
                completed_at: Some(100),
                visited: 1,
                skipped: 0,
                failed: 0,
            },
            next_before: None,
            versions_without_observation: 0,
            observations: vec![FileObservation {
                sequence: 1,
                path: "draft/budget.md".into(),
                physical_id: Some("physical".into()),
                presence: ObservedPresence::Present,
                byte_size: Some(37),
                modified_at: Some(80),
                created_at: None,
                extraction: ExtractionState::Text,
                note: None,
                basis: historical_basis(),
                replaces: None,
                activity: Some(ScanActivity {
                    id: 1,
                    started_at: 90,
                    finished_at: Some(90),
                    status: IndexStatus::Ready,
                    extractor_revision: "extract-v2".into(),
                }),
                representation: Some(SourceRepresentation {
                    version_id: "v0".into(),
                    content_digest: "old-digest".into(),
                    mime_type: "text/markdown".into(),
                    first_indexed_at: 90,
                    extractor_revision: Some("extract-v2".into()),
                    generated_by: Some(1),
                    extracted_passages: 1,
                    first_passage: Some(historical_reference()),
                }),
            }],
        })
    }
    async fn read_history(
        &self,
        reference: HistoryPassageRef,
    ) -> Result<HistoricalPassage, IndexError> {
        assert_eq!(reference, historical_reference());
        let mut source = passage();
        source.reference.version_id = "v0".into();
        source.path = "draft/budget.md".into();
        source.text = "The old orchid budget was 300 dollars.".into();
        source.observed_at = 90;
        source.modified_at = Some(80);
        Ok(HistoricalPassage {
            reference,
            passage: source,
            basis: historical_basis(),
            next: None,
        })
    }
    async fn folders(&self) -> Result<Vec<IndexedRoot>, IndexError> {
        Ok(vec![IndexedRoot {
            id: "root".into(),
            path: "/chosen/project".into(),
            status: IndexStatus::Ready,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: Some(100),
            auto_refresh: true,
            scan_options: sovra_core::file_index::ScanOptions::default(),
        }])
    }

    async fn catalogue(&self, query: CatalogQuery) -> Result<CatalogPage, IndexError> {
        query.validate()?;
        assert_eq!(query.filter.roots, vec!["root"]);
        Ok(CatalogPage {
            revision: 1,
            total_files: 1,
            total_known_bytes: 37,
            unknown_sizes: 0,
            coverage: vec![],
            files: vec![CatalogEntry {
                root_id: "root".into(),
                root_path: "/chosen/project".into(),
                file_id: "report".into(),
                path: passage().path,
                byte_size: Some(37),
                mime_type: Some("text/markdown".into()),
                content_digest: Some("digest".into()),
                observed_at: Some(100),
                modified_at: Some(90),
                created_at: None,
                extraction: ExtractionState::Text,
                note: None,
                first_passage: Some(passage().reference),
                passage_count: 1,
            }],
            groups: Vec::new(),
            next_cursor: None,
            has_more: false,
        })
    }

    async fn search(&self, query: IndexQuery) -> Result<Vec<IndexPassage>, IndexError> {
        assert_eq!(query.text, "orchid budget");
        self.searches.fetch_add(1, Ordering::SeqCst);
        Ok(vec![passage()])
    }
    async fn read(&self, reference: PassageRef) -> Result<IndexPassage, IndexError> {
        if reference == passage().reference {
            Ok(passage())
        } else {
            Err(IndexError(
                "This source version is no longer current.".into(),
            ))
        }
    }
}

fn historical_reference() -> HistoryPassageRef {
    HistoryPassageRef {
        root_id: "root".into(),
        file_id: "report".into(),
        observation: 1,
        ordinal: 0,
    }
}

fn historical_basis() -> EvidenceBasis {
    EvidenceBasis {
        tier: EvidenceTier::FilesystemObservation,
        confidence: Some(Confidence::new(1.0).unwrap()),
        activity_id: Some(1),
        recorded_at: 90,
        observed_at: Some(90),
        valid_time: None,
    }
}

struct Model {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl Llm for Model {
    fn name(&self) -> &'static str {
        "indexed-files-test"
    }
    async fn generate_content(
        &self,
        request: LlmRequest,
        _: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        assert!(request.tools.contains_key("search_indexed_files"));
        assert!(request.tools.contains_key("read_indexed_file"));
        assert!(request.tools.contains_key("propose_file_knowledge"));
        assert!(!request.tools.contains_key("approve_file_knowledge"));
        let last = request
            .contents
            .iter()
            .rev()
            .flat_map(|content| content.parts.iter().rev())
            .find_map(|part| match part {
                Part::FunctionResponse {
                    function_response, ..
                } => Some(&function_response.response),
                _ => None,
            });
        let step = self.calls.fetch_add(1, Ordering::SeqCst);
        let part = match step {
            0 => call("load_skill", json!({"name":"search-files"}), step),
            1 => {
                assert!(
                    last.expect("skill loaded")["procedure"]
                        .as_str()
                        .expect("instructions")
                        .contains("saved snapshots")
                );
                call("list_indexed_folders", json!({}), step)
            }
            2 => {
                assert_eq!(last.expect("scope list")["folders"][0]["id"], "root");
                call(
                    "query_indexed_files",
                    json!({"filter":{"roots":["root"],"keywords":"orchid budget"}}),
                    step,
                )
            }
            3 => {
                let file = &last.expect("catalogue")["catalogue"]["files"][0];
                assert_eq!(file["first_passage"], json!(passage().reference));
                call(
                    "search_indexed_files",
                    json!({"query":"orchid budget","limit":0}),
                    step,
                )
            }
            4 => {
                assert!(last.expect("validation response").get("error").is_some());
                call(
                    "search_indexed_files",
                    json!({"query":"orchid budget","roots":["root"]}),
                    step,
                )
            }
            5 => {
                let result = last.expect("search response");
                assert_eq!(result["passages"][0]["start_line"], 7);
                assert_eq!(result["passages"][0]["text"], passage().text);
                let mut reference = passage().reference;
                reference.version_id = "stale".into();
                call("read_indexed_file", json!({"reference":reference}), step)
            }
            6 => {
                assert!(
                    last.expect("stale reference response")
                        .get("error")
                        .is_some()
                );
                call(
                    "read_indexed_file",
                    json!({"reference":passage().reference}),
                    step,
                )
            }
            7 => {
                assert_eq!(
                    last.expect("read response")["passage"]["text"],
                    passage().text
                );
                call("load_skill", json!({"name":"file-history"}), step)
            }
            8 => {
                assert!(
                    last.expect("history skill")["procedure"]
                        .as_str()
                        .unwrap()
                        .contains("legacy imports")
                );
                call(
                    "inspect_file_history",
                    json!({"root_id":"root","file_id":"report"}),
                    step,
                )
            }
            9 => {
                let observation = &last.expect("history")["history"]["observations"][0];
                assert!(observation["basis"]["valid_time"].is_null());
                let reference = observation["representation"]["first_passage"].clone();
                call("read_file_history", json!({"reference":reference}), step)
            }
            10 => {
                let result = last.expect("historical source");
                assert_eq!(result["source_kind"], "historical_snapshot");
                assert_eq!(
                    result["historical_source"]["passage"]["path"],
                    "draft/budget.md"
                );
                assert!(
                    result["historical_source"]["passage"]["text"]
                        .as_str()
                        .unwrap()
                        .contains("300 dollars")
                );
                call("load_skill", json!({"name":"map-files"}), step)
            }
            11 => {
                assert!(
                    last.expect("mapping skill")["procedure"]
                        .as_str()
                        .unwrap()
                        .contains("native review")
                );
                let mut proposal = knowledge_draft();
                proposal["approve"] = json!(true);
                call("propose_file_knowledge", proposal, step)
            }
            12 => {
                assert!(
                    last.expect("approval injection rejected")
                        .get("error")
                        .is_some()
                );
                call("propose_file_knowledge", knowledge_draft(), step)
            }
            13 => {
                let result = last.expect("pending proposal");
                assert_eq!(result["state"], "pending");
                assert!(result["notice"].as_str().unwrap().contains("Do not claim"));
                call("query_file_knowledge", json!({}), step)
            }
            14 => {
                let result = last.expect("saved knowledge");
                assert_eq!(result["source_claims_verified"], false);
                assert_eq!(result["knowledge"]["relations"], json!([]));
                Part::Text { text: "The saved report budgets 420 dollars (notes/budget.md, v1, line 7), up from 300 dollars in the historical snapshot (draft/budget.md, v0, line 7). The proposed mapping is waiting for review in Knowledge.".into() }
            }
            _ => panic!("unexpected extra model call"),
        };
        let response = LlmResponse {
            content: Some(Content {
                role: "model".into(),
                parts: vec![part],
            }),
            finish_reason: Some(FinishReason::Stop),
            partial: false,
            turn_complete: true,
            ..LlmResponse::default()
        };
        Ok(Box::pin(futures::stream::once(async move { Ok(response) })))
    }
}

fn call(name: &str, args: Value, step: usize) -> Part {
    Part::FunctionCall {
        name: name.into(),
        args,
        id: Some(format!("index-{step}")),
        thought_signature: None,
    }
}

fn knowledge_draft() -> Value {
    json!({"title":"Orchid budget knowledge","evidence":[passage().reference],
        "entities":[{"local_id":"doc","kind":"document","label":"Budget report","evidence":[0]}],
        "relations":[{"subject":{"kind":"new","id":"doc"},"relation":"grounded_in","object":{"kind":"source","id":0},"statement":"This source represents the budget report.","evidence":[0]}]})
}

#[test]
fn agent_discovers_searches_reads_and_synthesizes_without_expanding_tool_permissions() {
    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
        let reader = Arc::new(Reader::default());
        let model = Arc::new(Model { calls: AtomicUsize::new(0) });
        let base = AdkHarness::with_llm_and_tools(&AgentConfig::default(), model.clone(), vec![]).unwrap();
        let catalog = Arc::new(SkillCatalog::default());
        let skill = catalog.find("search-files").unwrap();
        let harness = base.with_indexed_files(Some(reader.clone())).unwrap().with_skill_catalog(catalog.clone()).unwrap().with_skill(skill).unwrap();
        let (events, received) = async_channel::unbounded();
        harness.run_prompt("index-test", "What is the orchid budget and how did it change?", &events).await;
        let events: Vec<_> = std::iter::from_fn(|| received.try_recv().ok()).collect();
        assert!(events.iter().any(|event| matches!(event, Event::AgentCompleted { .. })), "{events:?}");
        assert!(events.iter().any(|event| matches!(event, Event::AgentChunk { text, .. } if text.contains("420 dollars"))));
        assert_eq!(reader.searches.load(Ordering::SeqCst), 1);
        assert_eq!(model.calls.load(Ordering::SeqCst), 15);
        assert_eq!(reader.proposals.load(Ordering::SeqCst), 1);
        assert!(harness.with_skill(catalog.find("file-history").unwrap()).is_ok());
        assert!(harness.with_indexed_files(None).unwrap().with_skill(skill).is_err());
        let disabled = base.with_files(Arc::new(vec![]), false).unwrap().with_indexed_files(Some(reader)).unwrap().with_skill_catalog(catalog.clone()).unwrap();
        assert!(disabled.with_skill(skill).is_err());
        assert!(disabled.with_skill(catalog.find("file-history").unwrap()).is_err());
        assert_eq!(disabled.skill_capabilities(), [] as [String; 0]);
    });
}
