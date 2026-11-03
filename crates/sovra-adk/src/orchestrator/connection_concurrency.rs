//! The real connection tools, and the strategy the orchestrator actually picks.
//!
//! Linking and unlinking stay on the Connections page. They are not model tools.
//! `prepare_message` only creates a review; it does not send.

use std::sync::Arc;

use adk_core::{Llm, LlmRequest, LlmResponseStream, Tool, ToolExecutionStrategy};
use sovra_core::AgentConfig;
use sovra_core::communications::MessagingAccess;
use sovra_core::google::GoogleService;

use crate::AdkHarness;
use crate::google::GoogleTools;

use super::execution_strategy;

const MUTATING: [&str; 6] = [
    "prepare_message",
    "link",
    "unlink",
    "draft_email",
    "add_calendar_event",
    "add_todo",
];

struct Idle;

#[async_trait::async_trait]
impl MessagingAccess for Idle {
    async fn call(
        &self,
        _request: sovra_core::communications::MessagingRequest,
    ) -> Result<serde_json::Value, sovra_core::connections::ChannelError> {
        Ok(serde_json::json!({}))
    }
}

struct IdleLlm;

#[async_trait::async_trait]
impl Llm for IdleLlm {
    fn name(&self) -> &'static str {
        "idle"
    }

    async fn generate_content(
        &self,
        _request: LlmRequest,
        _stream: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        Err(adk_core::AdkError::model("not called"))
    }
}

fn harness() -> AdkHarness {
    let built = AdkHarness::with_llm_and_tools(&AgentConfig::default(), Arc::new(IdleLlm), vec![])
        .expect("stub model");
    let messaging = built
        .with_messaging(Some(Arc::new(Idle)))
        .expect("messaging tools");
    let google = GoogleTools::from_fn(GoogleService::ALL.into_iter().collect(), |_| async {
        Ok(serde_json::json!({}))
    });
    messaging.with_google(Some(&google)).expect("google tools")
}

fn named<'a>(tools: &'a [Arc<dyn Tool>], name: &str) -> &'a Arc<dyn Tool> {
    tools
        .iter()
        .find(|tool| tool.name() == name)
        .unwrap_or_else(|| panic!("{name} is a shipped connection tool"))
}

fn assert_ordered(strategy: ToolExecutionStrategy) {
    assert!(
        matches!(strategy, ToolExecutionStrategy::Sequential),
        "a helper that can change something stays ordered, never Parallel"
    );
}

fn assert_automatic(strategy: ToolExecutionStrategy) {
    assert!(
        matches!(strategy, ToolExecutionStrategy::Auto),
        "an all-read-only helper may run its reads together"
    );
}

#[test]
fn connection_tools_stay_ordered_unless_every_tool_is_a_safe_read() {
    let tools = harness().offered_tools().to_vec();
    assert!(
        tools
            .iter()
            .all(|tool| tool.name() != "link" && tool.name() != "unlink"),
        "linking and unlinking are not model tools"
    );
    for tool in &tools {
        if MUTATING.contains(&tool.name()) {
            assert!(!tool.is_read_only(), "{}", tool.name());
            assert!(!tool.is_concurrency_safe(), "{}", tool.name());
        }
    }
    for name in [
        "messaging_status",
        "list_recent_chats",
        "read_recent_messages",
        "message_receipt",
        "search_email",
        "read_email",
        "list_calendar_events",
        "search_drive_files",
        "read_drive_file",
        "search_contacts",
        "list_todos",
        "find_people",
    ] {
        let tool = named(&tools, name);
        assert!(tool.is_read_only(), "{name}");
        assert!(tool.is_concurrency_safe(), "{name}");
    }
    assert!(!named(&tools, "prepare_message").is_read_only());
    assert!(!named(&tools, "draft_email").is_concurrency_safe());

    assert_ordered(execution_strategy(&tools));
    let reads: Vec<_> = tools
        .iter()
        .filter(|tool| tool.is_read_only() && tool.is_concurrency_safe())
        .cloned()
        .collect();
    assert!(!reads.is_empty());
    assert_automatic(execution_strategy(&reads));
    let with_prepare: Vec<_> = tools
        .iter()
        .filter(|tool| tool.name() == "prepare_message" || tool.name() == "messaging_status")
        .cloned()
        .collect();
    assert_ordered(execution_strategy(&with_prepare));
}

#[test]
fn people_lookup_is_a_safe_read_and_prepare_stays_ordered() {
    let tools = harness().offered_tools().to_vec();
    let lookup = named(&tools, "find_people");
    assert!(lookup.is_read_only());
    assert!(lookup.is_concurrency_safe());
    let prepare = named(&tools, "prepare_message");
    assert!(!prepare.is_read_only());
    assert!(!prepare.is_concurrency_safe());
    assert_ordered(execution_strategy(&tools));
    assert_automatic(execution_strategy(&[Arc::clone(lookup)]));
    assert_ordered(execution_strategy(&[
        Arc::clone(lookup),
        Arc::clone(prepare),
    ]));
    assert!(
        !matches!(
            execution_strategy(&[Arc::clone(lookup), Arc::clone(prepare)]),
            ToolExecutionStrategy::Parallel
        ),
        "preparing a message stays ordered"
    );
}
