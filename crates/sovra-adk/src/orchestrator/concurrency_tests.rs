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

/// The live work log shows the parent asking a helper, and the helper's own tool
/// calls in that helper's lane, without those calls entering the parent history.
#[tokio::test]
async fn the_work_log_shows_a_helpers_own_steps_in_its_lane() {
    let calls = Arc::new(std::sync::Mutex::new(vec![]));
    let tools: Vec<Arc<dyn adk_core::Tool>> = ["search_saved_details", "read_attachment"]
        .into_iter()
        .map(|name| {
            Arc::new(OrderedTool {
                name,
                concurrent: false,
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
        .run_prompt("work-log", "Check these sources.", &events)
        .await;
    let logs: Vec<sovra_core::work::WorkActivity> = std::iter::from_fn(|| received.try_recv().ok())
        .filter_map(|event| match event {
            sovra_core::Event::AgUi {
                event:
                    sovra_core::AgUiEvent::ActivitySnapshot {
                        activity: sovra_core::LiveActivity::Work(work),
                        ..
                    },
                ..
            } => Some(work),
            _ => None,
        })
        .collect();
    let last = logs.last().expect("a turn that used tools shows its work");
    assert!(last.finished && !last.stopped, "{last:?}");
    let labels: Vec<(&str, &str)> = last
        .steps
        .iter()
        .map(|step| (step.helper.as_str(), step.label.as_str()))
        .collect();
    assert_eq!(labels[0].0, "", "Sovra asks the helper");
    assert!(labels[0].1.starts_with("Asking the Coding"), "{labels:?}");
    let lane = &last.helpers[0];
    assert_eq!(lane.id, last.steps[0].id, "the lane is the request");
    assert_eq!(lane.steps, 2, "{last:?}");
    assert!(
        last.steps[1..]
            .iter()
            .all(|step| step.helper == lane.id && step.state == sovra_core::work::StepState::Done),
        "{labels:?}"
    );
    assert!(
        logs.iter().any(|log| log
            .helpers
            .first()
            .and_then(|h| h.doing.as_deref())
            .is_some()),
        "while working, the lane says what the helper is doing"
    );
}

/// Never stops asking the same helper the same thing.
struct LoopingModel {
    helper_runs: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Llm for LoopingModel {
    fn name(&self) -> &'static str {
        "looping-parent"
    }
    async fn generate_content(
        &self,
        request: LlmRequest,
        _: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        let part = if request.tools.contains_key("CodingAgent") {
            let turn = request.contents.len();
            Part::FunctionCall {
                name: "CodingAgent".into(),
                args: serde_json::json!({"request": "Check it again"}),
                id: Some(format!("again-{turn}")),
                thought_signature: None,
            }
        } else {
            self.helper_runs.fetch_add(1, Ordering::SeqCst);
            Part::Text {
                text: "Same as before.".into(),
            }
        };
        let response = LlmResponse::new(Content {
            role: "model".into(),
            parts: vec![part],
        });
        Ok(Box::pin(futures::stream::once(async move { Ok(response) })))
    }
}

#[tokio::test]
async fn a_looping_agent_is_refused_repeats_then_stopped_with_plain_words() {
    let helper_runs = Arc::new(AtomicUsize::new(0));
    let harness = AdkHarness::with_llm_and_tools(
        &sovra_core::AgentConfig::default(),
        Arc::new(LoopingModel {
            helper_runs: helper_runs.clone(),
        }),
        vec![],
    )
    .unwrap();
    let (events, received) = async_channel::unbounded();
    tokio::time::timeout(
        Duration::from_secs(10),
        harness.run_prompt("looping", "Check it.", &events),
    )
    .await
    .expect("the step cap ends the loop");
    let output: Vec<_> = std::iter::from_fn(|| received.try_recv().ok()).collect();
    assert_eq!(
        helper_runs.load(Ordering::SeqCst),
        sovra_core::work::REPEAT_LIMIT as usize,
        "identical requests past the limit never reach the helper"
    );
    assert!(
        output.iter().any(|event| matches!(event,
            sovra_core::Event::AgentError { message, .. }
                if message == sovra_core::work::STEP_LIMIT_MESSAGE)),
        "{output:?}"
    );
    let stopped = output.iter().rev().find_map(|event| match event {
        sovra_core::Event::AgUi {
            event:
                sovra_core::AgUiEvent::ActivitySnapshot {
                    activity: sovra_core::LiveActivity::Work(work),
                    ..
                },
            ..
        } => Some(work),
        _ => None,
    });
    let stopped = stopped.expect("the log is closed");
    assert!(stopped.finished && stopped.stopped);
    assert!(
        stopped.status.starts_with("Stopped after"),
        "{}",
        stopped.status
    );
}
