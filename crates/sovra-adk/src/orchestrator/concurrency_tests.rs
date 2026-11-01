//! Exercise the pinned ADK dispatcher, including queued-call cancellation.

use crate::AdkHarness;
use adk_core::{Content, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Model {
    entered: async_channel::Sender<()>,
    release: async_channel::Receiver<()>,
    active: Arc<AtomicUsize>,
    peak: AtomicUsize,
}

#[async_trait::async_trait]
impl Llm for Model {
    fn name(&self) -> &'static str {
        "bounded-delegation"
    }
    async fn generate_content(
        &self,
        request: LlmRequest,
        _: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        let parts = if request.tools.contains_key("CodingAgent") {
            if request
                .contents
                .iter()
                .flat_map(|content| &content.parts)
                .any(|part| matches!(part, Part::FunctionResponse { .. }))
            {
                vec![Part::Text {
                    text: "All six independent checks finished.".into(),
                }]
            } else {
                (0..6).map(|index| Part::FunctionCall {
                    name: "CodingAgent".into(), args: serde_json::json!({"request":format!("Check independent branch {index}")}),
                    id: Some(format!("branch-{index}")), thought_signature: None,
                }).collect()
            }
        } else {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            let _guard = Active(self.active.clone());
            self.peak.fetch_max(active, Ordering::SeqCst);
            self.entered.send(()).await.unwrap();
            let _ = self.release.recv().await;
            vec![Part::Text {
                text: "Checked.".into(),
            }]
        };
        let response = LlmResponse::new(Content {
            role: "model".into(),
            parts,
        });
        Ok(Box::pin(futures::stream::once(async move { Ok(response) })))
    }
}

async fn run(cancel: bool) {
    let (entered, starts) = async_channel::unbounded();
    let (release, wait) = async_channel::unbounded();
    let active = Arc::new(AtomicUsize::new(0));
    let model = Arc::new(Model {
        entered,
        release: wait,
        active: active.clone(),
        peak: AtomicUsize::new(0),
    });
    let harness =
        AdkHarness::with_llm_and_tools(&sovra_core::AgentConfig::default(), model.clone(), vec![])
            .unwrap();
    let (events, received) = async_channel::unbounded();
    let (abort, registration) = futures::future::AbortHandle::new_pair();
    let task = futures::future::Abortable::new(
        async move {
            harness
                .run_prompt("bounded", "Compare these independent checks.", &events)
                .await;
        },
        registration,
    );
    let proof = async {
        for _ in 0..4 {
            starts.recv().await.unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), starts.recv())
                .await
                .is_err(),
            "More than four delegates started while the first four were held"
        );
        assert_eq!(active.load(Ordering::SeqCst), 4);
        if cancel {
            abort.abort();
        } else {
            release.close();
        }
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        futures::join!(task, proof)
    })
    .await
    .unwrap();
    if cancel {
        assert!(result.is_err());
        assert_eq!(
            active.load(Ordering::SeqCst),
            0,
            "cancelling the owner must drop child model futures"
        );
        assert!(
            starts.try_recv().is_err(),
            "queued delegates must not start after cancellation"
        );
    } else {
        result.unwrap();
        assert_eq!(starts.len(), 2, "both queued delegates must eventually run");
        assert!(
            std::iter::from_fn(|| received.try_recv().ok())
                .any(|event| matches!(event, sovra_core::Event::AgentCompleted { .. }))
        );
    }
    assert_eq!(model.peak.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn independent_delegations_overlap_with_a_four_call_limit() {
    run(false).await;
}

#[tokio::test]
async fn cancelling_delegations_drops_active_and_queued_work() {
    run(true).await;
}

struct OrderedTool {
    name: &'static str,
    concurrent: bool,
    calls: Arc<std::sync::Mutex<Vec<&'static str>>>,
}

#[async_trait::async_trait]
impl adk_core::Tool for OrderedTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &'static str {
        "Record the requested execution order."
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn is_concurrency_safe(&self) -> bool {
        self.concurrent
    }
    fn parameters_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({"type":"object", "properties":{}}))
    }
    async fn execute(
        &self,
        _: Arc<dyn adk_core::ToolContext>,
        _: serde_json::Value,
    ) -> adk_core::Result<serde_json::Value> {
        self.calls.lock().unwrap().push(self.name);
        Ok(serde_json::json!({"done":true}))
    }
}

struct OrderedModel;
#[async_trait::async_trait]
impl Llm for OrderedModel {
    fn name(&self) -> &'static str {
        "ordered-helper-tools"
    }
    async fn generate_content(
        &self,
        request: LlmRequest,
        _: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        let called = request
            .contents
            .iter()
            .flat_map(|content| &content.parts)
            .any(|part| matches!(part, Part::FunctionResponse { .. }));
        let calls = if request.tools.contains_key("CodingAgent") {
            vec![(
                "CodingAgent",
                serde_json::json!({"request":"Look up the date, then read the attachment."}),
            )]
        } else {
            vec![
                ("search_saved_details", serde_json::json!({})),
                ("read_attachment", serde_json::json!({})),
            ]
        };
        let parts = if called {
            vec![Part::Text {
                text: "Checked in order.".into(),
            }]
        } else {
            calls
                .into_iter()
                .map(|(name, args)| Part::FunctionCall {
                    name: name.into(),
                    args,
                    id: Some(format!("call-{name}")),
                    thought_signature: None,
                })
                .collect()
        };
        let response = LlmResponse::new(Content {
            role: "model".into(),
            parts,
        });
        Ok(Box::pin(futures::stream::once(async move { Ok(response) })))
    }
}

#[tokio::test]
async fn a_helper_never_moves_safe_reads_ahead_of_ordered_tools() {
    let calls = Arc::new(std::sync::Mutex::new(vec![]));
    let tools: Vec<Arc<dyn adk_core::Tool>> =
        [("search_saved_details", false), ("read_attachment", true)]
            .into_iter()
            .map(|(name, concurrent)| {
                Arc::new(OrderedTool {
                    name,
                    concurrent,
                    calls: calls.clone(),
                }) as Arc<dyn adk_core::Tool>
            })
            .collect();
    let harness = AdkHarness::with_llm_and_tools(
        &sovra_core::AgentConfig::default(),
        Arc::new(OrderedModel),
        tools,
    )
    .unwrap();
    let (events, received) = async_channel::unbounded();
    harness
        .run_prompt("ordered-helper", "Check these sources.", &events)
        .await;
    let output: Vec<_> = std::iter::from_fn(|| received.try_recv().ok()).collect();
    assert!(
        output
            .iter()
            .any(|event| matches!(event, sovra_core::Event::AgentCompleted { .. })),
        "{output:?}"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        ["search_saved_details", "read_attachment"]
    );
}
