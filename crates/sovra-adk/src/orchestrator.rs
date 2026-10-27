//! Chat orchestrator. One parent delegates to roster specialists through ADK
//! `AgentTool`s, so independent specialists run together on isolated branches.
//!
//! Which specialists exist this turn is decided in `sovra-core` from the
//! capabilities actually present; each specialist gets exactly its providers'
//! tools. The orchestrator names no specialist itself, so the roster can grow.

use std::sync::Arc;

use adk_agent::{Agent, LlmAgent, LlmAgentBuilder};
use adk_core::{Llm, Tool};
use adk_tool::{AgentTool, AgentToolSessionSnapshot};
use std::collections::BTreeSet;

use sovra_core::{
    Capability, SpecialistAgentSpec, available_specialists, default_sovra_srx_agents,
    required_capabilities,
};

use crate::Error;
use crate::capabilities::{self, CapabilityProvider};

/// Parent agent id. Matches the roster entry the router already uses.
pub(crate) const ORCHESTRATOR_ID: &str = "OrchestratorAgent";

/// Root agent whose `sub_agents` are the roster delegates.
///
/// The inner model has no sub-agents, so ADK does not offer `transfer_to_agent`.
/// Delegation is an `AgentTool` call and the parent stays the speaker.
struct Orchestrator {
    description: String,
    inner: LlmAgent,
    delegates: Vec<Arc<dyn Agent>>,
}

impl std::fmt::Debug for Orchestrator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.delegates.iter().map(|agent| agent.name()).collect();
        formatter
            .debug_struct("Orchestrator")
            .field("delegates", &names)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl Agent for Orchestrator {
    fn name(&self) -> &'static str {
        ORCHESTRATOR_ID
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn sub_agents(&self) -> &[Arc<dyn Agent>] {
        &self.delegates
    }

    fn supports_agent_transfer(&self) -> bool {
        // Resume the parent on the next turn. A specialist that spoke inside
        // an AgentTool must not become the next root.
        false
    }

    fn transfer_targets_for(&self, _agent_name: &str) -> Option<Vec<String>> {
        // Empty list forbids handoff. Specialists are tools, not transfers.
        Some(Vec::new())
    }

    async fn run(
        &self,
        ctx: Arc<dyn adk_core::InvocationContext>,
    ) -> adk_core::Result<adk_core::EventStream> {
        self.inner.run(ctx).await
    }
}

/// Tool wrapper so a specialist call is an agent delegation and is logged
/// with the orchestrator name. `AgentTool` runs the child on an isolated session.
struct DelegateTool {
    inner: AgentTool,
}

impl std::fmt::Debug for DelegateTool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DelegateTool")
            .field("delegate", &self.inner.name())
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl Tool for DelegateTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Option<serde_json::Value> {
        self.inner.parameters_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn is_agent_delegation(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        ctx: Arc<dyn adk_core::ToolContext>,
        args: serde_json::Value,
    ) -> adk_core::Result<serde_json::Value> {
        let delegate = self.inner.name();
        tracing::info!(
            orchestrator = ORCHESTRATOR_ID,
            %delegate,
            "orchestrator delegating to {delegate}"
        );
        self.inner.execute(ctx, args).await
    }
}

pub(crate) struct OrchestratorParts {
    pub llm: Arc<dyn Llm>,
    pub parent_tools: Vec<Arc<dyn Tool>>,
    pub instruction: String,
    /// Real tool sets this turn. Specialists whose requirements are unmet are
    /// not offered, so the parent cannot route to one that would only guess.
    pub providers: Vec<Arc<dyn CapabilityProvider>>,
    pub tools_enabled: bool,
}

pub(crate) fn build(parts: OrchestratorParts) -> Result<Arc<dyn Agent>, Error> {
    let roster = default_sovra_srx_agents();
    let available: BTreeSet<Capability> = parts
        .providers
        .iter()
        .map(|provider| provider.capability())
        .collect();
    let selected: Vec<&SpecialistAgentSpec> = if parts.tools_enabled {
        available_specialists(&roster, &available)
    } else {
        Vec::new()
    };
    let description = roster
        .iter()
        .find(|spec| spec.id == ORCHESTRATOR_ID)
        .map_or_else(
            || "Orchestrates the specialist roster.".to_owned(),
            |spec| spec.description.clone(),
        );

    let mut delegates = Vec::with_capacity(selected.len());
    for spec in &selected {
        let needs = required_capabilities(&spec.id);
        let equipping: Vec<&Arc<dyn CapabilityProvider>> = parts
            .providers
            .iter()
            .filter(|provider| needs.contains(&provider.capability()))
            .collect();
        let tools = equipping
            .iter()
            .flat_map(|provider| provider.tools())
            .collect();
        let mut instruction = format!(
            "You are {}. {} Answer only the request in this call. Do not invent titles or URLs.",
            spec.id, spec.description
        );
        for provider in &equipping {
            instruction.push_str(&provider.procedure());
        }
        delegates.push(child_agent(
            spec,
            Arc::clone(&parts.llm),
            tools,
            instruction,
        )?);
    }

    let mut parent_tools = Vec::new();
    for tool in parts.parent_tools {
        let name = tool.name();
        if capabilities::is_provider_tool(name) {
            continue;
        }
        if selected.iter().any(|spec| spec.id == name) {
            continue;
        }
        parent_tools.push(tool);
    }

    let routing_rules: Vec<String> = parts
        .providers
        .iter()
        .filter_map(|provider| provider.routing_rule())
        .collect();
    let instruction = parent_instruction(
        &parts.instruction,
        &selected,
        &routing_rules,
        available.contains(&Capability::WebSearch),
    );
    let mut builder = LlmAgentBuilder::new(ORCHESTRATOR_ID)
        .description(description.clone())
        .instruction(instruction)
        .model(parts.llm)
        .parallelize_agent_delegations(true);
    for tool in parent_tools {
        builder = builder.tool(tool);
    }
    for delegate in &delegates {
        let agent_tool = AgentTool::new(Arc::clone(delegate))
            .session_snapshot(AgentToolSessionSnapshot::Isolated)
            .forward_shared_state(false)
            .forward_memory(false)
            .history_max_events(0);
        builder = builder.tool(Arc::new(DelegateTool { inner: agent_tool }));
    }
    let inner = builder.build().map_err(Error::AgentBuild)?;
    tracing::debug!(
        specialists = delegates.len(),
        capabilities = ?available,
        "orchestrator built"
    );
    Ok(Arc::new(Orchestrator {
        description,
        inner,
        delegates,
    }))
}

fn child_agent(
    spec: &SpecialistAgentSpec,
    llm: Arc<dyn Llm>,
    tools: Vec<Arc<dyn Tool>>,
    instruction: String,
) -> Result<Arc<dyn Agent>, Error> {
    let mut builder = LlmAgentBuilder::new(spec.id.as_str())
        .description(spec.description.clone())
        .instruction(instruction)
        .model(llm);
    for tool in tools {
        builder = builder.tool(tool);
    }
    let agent = builder.build().map_err(Error::AgentBuild)?;
    Ok(Arc::new(agent))
}

/// The parent's instruction: the catalogue it routes from, then the rules.
///
/// ADK routes by description, so the catalogue lists only specialists that can
/// do their job this turn, and each provider adds its own "when to call" rule.
fn parent_instruction(
    base: &str,
    delegates: &[&SpecialistAgentSpec],
    routing_rules: &[String],
    web_search_available: bool,
) -> String {
    let mut catalog = String::new();
    for spec in delegates {
        catalog.push_str("- ");
        catalog.push_str(&spec.id);
        catalog.push_str(": ");
        catalog.push_str(&spec.description);
        catalog.push('\n');
    }
    if catalog.is_empty() {
        catalog.push_str("(none this turn)\n");
    }
    let mut rules: Vec<String> = delegates
        .iter()
        .filter(|spec| active_skill_requests(base, &spec.id))
        .map(|spec| {
            format!(
                "The active procedure requests {id}. Call the {id} tool before you answer, even \
                 when the user's message is only the question and does not say \"search the \
                 web\", \"search the internet\", or \"web search for\". Then use only what {id} \
                 returns and cite only the titles and URLs it gives.",
                id = spec.id
            )
        })
        .collect();
    rules.extend(routing_rules.iter().cloned());
    if !web_search_available {
        rules.push(
            "Web search is off. No specialist can reach the internet this turn. Do not invent \
             titles or URLs and do not claim to have searched the web."
                .to_owned(),
        );
    }
    rules.push(
        "Only the specialists listed above exist this turn. If a request needs something none \
         of them can do (for example email, calendars or running code), say plainly that it is \
         not available yet instead of guessing."
            .to_owned(),
    );
    format!(
        "{base}\n\nYou are {ORCHESTRATOR_ID}. You speak to the user. These are the specialists \
         you can call as tools:\n{catalog}{rules}\n\
         Call a specialist by its tool name. Independent specialists run at the same time, and \
         each one sees only the request you send it. Do not transfer the conversation away from \
         yourself.",
        rules = rules.join("\n")
    )
}

/// The selected procedure's instruction block lists the specialist it may call.
fn active_skill_requests(instruction: &str, tool: &str) -> bool {
    let Some(value) = skill_block(instruction) else {
        return false;
    };
    value
        .get("requested_tools")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|tools| tools.iter().any(|item| item.as_str() == Some(tool)))
}

fn skill_block(instruction: &str) -> Option<serde_json::Value> {
    let rest = instruction.split_once("Active user-selected skill.")?.1;
    let start = rest.find('{')?;
    let bytes = &rest.as_bytes()[start..];
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escape = false;
    for (index, byte) in bytes.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if *byte == b'\\' {
                escape = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match *byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_slice(&bytes[..=index]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use adk_core::{Content, FinishReason, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part};
    use async_channel::{Receiver, Sender};
    use futures::stream;
    use serde_json::{Value, json};
    use sovra_core::files::{Attachment, FileContent, ModelPurpose};
    use sovra_core::skills::SkillCatalog;
    use sovra_core::{AgentConfig, Event, WEB_SEARCH_AGENT_ID, default_sovra_srx_agents};

    use super::{BTreeSet, Capability, available_specialists, parent_instruction};
    use crate::capabilities::CapabilityProvider as _;

    use super::ORCHESTRATOR_ID;
    use crate::AdkHarness;
    use crate::web_search::SearchClient;

    const HIT_TITLE: &str = "Rust Release Notes";
    const HIT_URL: &str = "https://doc.rust-lang.org/releases.html";
    const SIBLING_URL: &str = "https://sibling.example/keep";
    const SEARCH_BODY: &str = r#"{"results":[
        {"title":"Rust Release Notes","url":"https://doc.rust-lang.org/releases.html","content":"notes"},
        {"title":"Duplicate","url":"https://doc.rust-lang.org/releases.html#top","content":"again"},
        {"title":"Broken","content":"missing url"}
    ]}"#;

    fn verification_line(line: &str) {
        // `clippy.toml` allows print macros in tests, so an `expect` on
        // `print_stderr` is unfulfilled. These lines are the verification log.
        eprintln!("{line}");
    }

    fn drive<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("ADK tool timeout needs a tokio timer")
            .block_on(future)
    }

    /// What core says the orchestrator may offer with (or without) web search.
    fn roster_pairs(include_search: bool) -> Vec<(String, String)> {
        let roster = default_sovra_srx_agents();
        let mut available = BTreeSet::new();
        if include_search {
            available.insert(Capability::WebSearch);
        }
        available_specialists(&roster, &available)
            .into_iter()
            .map(|spec| (spec.id.clone(), spec.description.clone()))
            .collect()
    }

    fn harness_with(llm: Arc<dyn Llm>) -> AdkHarness {
        AdkHarness::with_llm_and_tools(&AgentConfig::default(), llm, Vec::new()).expect("harness")
    }

    #[test]
    fn orchestrator_delegates_match_the_roster() {
        let enabled = harness_with(Arc::new(IdleLlm))
            .with_web_search(SearchClient::failing("unused"))
            .expect("enabled orchestrator");
        assert_eq!(enabled.orchestrator_name(), ORCHESTRATOR_ID);
        assert_eq!(enabled.delegates(), roster_pairs(true));
        let disabled = enabled.without_web_search().expect("disabled orchestrator");
        assert_eq!(disabled.orchestrator_name(), ORCHESTRATOR_ID);
        let remaining = roster_pairs(false);
        assert_eq!(disabled.delegates(), remaining);
        // Every web-backed specialist leaves with the search, not just WebSearchAgent.
        for web_backed in [WEB_SEARCH_AGENT_ID, "DeepResearchAgent", "StockMarketAgent"] {
            assert!(
                remaining.iter().all(|(id, _)| id != web_backed),
                "{web_backed}"
            );
        }
        assert_eq!(remaining.len() + 5, roster_pairs(true).len());
    }

    #[test]
    fn orchestrator_offers_only_specialists_that_can_do_their_job() {
        let enabled = harness_with(Arc::new(IdleLlm))
            .with_web_search(SearchClient::failing("unused"))
            .expect("enabled orchestrator");
        let ids: Vec<String> = enabled.delegates().into_iter().map(|(id, _)| id).collect();
        for absent in [
            ORCHESTRATOR_ID,
            "GoogleWorkspace",
            "TodosAgent",
            "GitHubAgent",
            "ExecutionAgent",
        ] {
            assert!(
                !ids.iter().any(|id| id == absent),
                "{absent} offered without its tools"
            );
        }
        for present in [WEB_SEARCH_AGENT_ID, "DeepResearchAgent", "CodingAgent"] {
            assert!(ids.iter().any(|id| id == present), "{present} missing");
        }
    }

    #[test]
    fn parent_instruction_lists_the_catalogue_and_each_providers_rule() {
        let roster = default_sovra_srx_agents();
        let selected = available_specialists(&roster, &BTreeSet::from([Capability::WebSearch]));
        let provider = crate::capabilities::WebSearchProvider::new(SearchClient::failing("x"));
        let rules = vec![provider.routing_rule().expect("web rule")];
        let with_web = parent_instruction("base", &selected, &rules, true);
        assert!(with_web.contains("- WebSearchAgent: Looks things up on the internet"));
        assert!(with_web.contains("opening hours"));
        assert!(!with_web.contains("Web search is off"));
        assert!(!with_web.contains("- GoogleWorkspace"));
        assert!(with_web.contains("not available yet"));

        let offline = available_specialists(&roster, &BTreeSet::new());
        let without = parent_instruction("base", &offline, &[], false);
        assert!(without.contains("Web search is off"));
        assert!(!without.contains("- WebSearchAgent"));
    }

    #[test]
    fn web_backed_specialists_carry_the_search_tools_and_procedure() {
        drive(async {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let recorder = Arc::new(ToolsByAgentLlm {
                seen: Arc::clone(&seen),
            });
            let harness = harness_with(recorder)
                .with_web_search(SearchClient::failing("unused"))
                .expect("enabled");
            let (tx, rx) = async_channel::unbounded();
            harness.run_prompt("tools-by-agent", "hello", &tx).await;
            let _ = collect_answer(&rx).await;
            let seen = seen.lock().expect("seen");
            let parent = seen.first().expect("parent request");
            assert!(parent.iter().any(|name| name == "DeepResearchAgent"));
            assert!(
                parent
                    .iter()
                    .all(|name| name != "web_search" && name != "read_web_page")
            );
        });
    }

    /// Records the tool names of every request it receives.
    struct ToolsByAgentLlm {
        seen: Arc<Mutex<Vec<Vec<String>>>>,
    }

    #[async_trait::async_trait]
    impl Llm for ToolsByAgentLlm {
        fn name(&self) -> &'static str {
            "tools-by-agent"
        }

        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            self.seen
                .lock()
                .expect("seen")
                .push(request.tools.keys().cloned().collect());
            let response = LlmResponse {
                content: Some(Content::new("model").with_text("hello")),
                finish_reason: Some(FinishReason::Stop),
                partial: false,
                turn_complete: true,
                ..LlmResponse::default()
            };
            Ok(Box::pin(stream::once(async move { Ok(response) })))
        }
    }

    #[test]
    fn orchestrator_search_cites_explicit_phrases_and_skips_greetings() {
        drive(async {
            for prompt in [
                "search the web for rust code release notes",
                "search the internet for rust code",
                "web search for the rust compiler",
            ] {
                let (answer, hits) = run_phrase(prompt, false).await;
                assert!(answer.contains(HIT_TITLE), "{prompt}: {answer}");
                assert!(answer.contains(HIT_URL), "{prompt}: {answer}");
                assert_eq!(answer.matches(HIT_URL).count(), 1, "{prompt}: {answer}");
                assert!(!answer.contains("https://invented.example"), "{answer}");
                assert!(hits >= 1, "{prompt} did not call SearXNG");
            }
            let (attached, hits) =
                run_phrase("search the web for rust code release notes", true).await;
            assert!(attached.contains(HIT_TITLE), "{attached}");
            assert!(attached.contains(HIT_URL), "{attached}");
            assert_eq!(attached.matches(HIT_URL).count(), 1, "{attached}");
            assert!(hits >= 1, "file-attached search did not delegate");

            let (greeting, hits) = run_phrase("hello", false).await;
            assert_eq!(hits, 0, "a greeting must not call web search");
            assert!(!greeting.contains(HIT_URL), "{greeting}");
            assert!(!greeting.contains(HIT_TITLE), "{greeting}");

            let seen = Arc::new(Mutex::new(Vec::new()));
            let recorder = Arc::new(RecordingLlm {
                seen: Arc::clone(&seen),
            });
            let disabled = harness_with(recorder)
                .with_web_search(SearchClient::failing("disabled"))
                .expect("enabled")
                .without_web_search()
                .expect("disabled");
            assert!(
                disabled
                    .delegates()
                    .iter()
                    .all(|(id, _)| id != WEB_SEARCH_AGENT_ID)
            );
            let (tx, rx) = async_channel::unbounded();
            disabled
                .run_prompt("disabled", "search the web for rust release notes", &tx)
                .await;
            let answer = collect_answer(&rx).await;
            assert!(!answer.contains(HIT_URL), "{answer}");
            let names = seen.lock().expect("tool names");
            assert!(
                names
                    .iter()
                    .all(|name| name != "web_search" && name != WEB_SEARCH_AGENT_ID),
                "disabled turn exposed {names:?}"
            );
        });
    }

    #[test]
    fn orchestrator_parallel_two_delegates_overlap_and_keep_the_sibling_citation() {
        drive(async {
            let notes = Arc::new(BranchNotes::default());
            let (release_tx, release_rx) = async_channel::unbounded();
            let llm = Arc::new(ParallelLlm {
                notes: Arc::clone(&notes),
                release_tx,
                release_rx,
                web_calls: AtomicUsize::new(0),
                parent_calls: AtomicUsize::new(0),
            });
            let client = SearchClient::from_fn(|query| async move {
                match query.as_str() {
                    "keep-query" => Ok(format!(
                        r#"{{"results":[{{"title":"Sibling","url":"{SIBLING_URL}","content":"ok"}}]}}"#
                    )),
                    "bad-query" => Err("upstream timeout".to_owned()),
                    other => Err(format!("unexpected query {other}")),
                }
            });
            let harness = harness_with(llm)
                .with_web_search(client)
                .expect("parallel orchestrator");
            let (tx, rx) = async_channel::unbounded();
            harness
                .run_prompt("parallel", "compare the notes and the code", &tx)
                .await;
            let answer = collect_answer(&rx).await;
            assert!(
                notes.max_in_flight.load(Ordering::SeqCst) >= 2,
                "delegates did not overlap"
            );
            let web_text = notes.web.lock().expect("web branch").clone();
            let code_text = notes.code.lock().expect("code branch").clone();
            assert!(web_text.contains("search-notes"), "{web_text}");
            assert!(!web_text.contains("coding-only-marker"), "{web_text}");
            assert!(code_text.contains("coding-only-marker"), "{code_text}");
            assert!(!code_text.contains("search-notes"), "{code_text}");
            assert!(answer.contains(SIBLING_URL), "{answer}");
            assert!(answer.contains("Sibling"), "{answer}");
            assert!(!answer.contains("https://invented.example"), "{answer}");
            verification_line("overlapping execution");
            verification_line(SIBLING_URL);
        });
    }

    #[test]
    fn orchestrator_parallel_two_sessions_cite_only_their_own_url() {
        drive(async {
            let max_in_flight = Arc::new(AtomicUsize::new(0));
            let in_flight = Arc::new(AtomicUsize::new(0));
            let started = Arc::new(AtomicUsize::new(0));
            let (release_tx, release_rx) = async_channel::unbounded::<()>();
            let client = SearchClient::from_fn({
                let max_in_flight = Arc::clone(&max_in_flight);
                let in_flight = Arc::clone(&in_flight);
                let started = Arc::clone(&started);
                move |query| {
                    let max_in_flight = Arc::clone(&max_in_flight);
                    let in_flight = Arc::clone(&in_flight);
                    let started = Arc::clone(&started);
                    let release_tx = release_tx.clone();
                    let release_rx = release_rx.clone();
                    async move {
                        let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                        max_in_flight.fetch_max(now, Ordering::SeqCst);
                        if started.fetch_add(1, Ordering::SeqCst) + 1 == 2 {
                            let _ = release_tx.try_send(());
                            let _ = release_tx.try_send(());
                        }
                        let _ =
                            tokio::time::timeout(Duration::from_secs(5), release_rx.recv()).await;
                        in_flight.fetch_sub(1, Ordering::SeqCst);
                        match query.as_str() {
                            "alpha" => Ok(
                                r#"{"results":[{"title":"Alpha","url":"https://alpha.example/a","content":"a"}]}"#
                                    .to_owned(),
                            ),
                            "beta" => Ok(
                                r#"{"results":[{"title":"Beta","url":"https://beta.example/b","content":"b"}]}"#
                                    .to_owned(),
                            ),
                            other => Err(format!("unexpected query {other}")),
                        }
                    }
                }
            });
            let left = harness_with(Arc::new(CitingLlm::new(vec!["alpha".to_owned()])))
                .with_web_search(client.clone())
                .expect("left");
            let right = harness_with(Arc::new(CitingLlm::new(vec!["beta".to_owned()])))
                .with_web_search(client)
                .expect("right");
            let (left_tx, left_rx) = async_channel::unbounded();
            let (right_tx, right_rx) = async_channel::unbounded();
            let left_run = left.run_prompt("session-alpha", "search the web for alpha", &left_tx);
            let right_run = right.run_prompt("session-beta", "search the web for beta", &right_tx);
            futures::future::join(left_run, right_run).await;
            let alpha = collect_answer(&left_rx).await;
            let beta = collect_answer(&right_rx).await;
            assert!(
                max_in_flight.load(Ordering::SeqCst) >= 2,
                "sessions did not overlap"
            );
            assert!(alpha.contains("https://alpha.example/a"), "{alpha}");
            assert!(!alpha.contains("https://beta.example/b"), "{alpha}");
            assert!(beta.contains("https://beta.example/b"), "{beta}");
            assert!(!beta.contains("https://alpha.example/a"), "{beta}");
            verification_line("no cross-session URL");
        });
    }

    #[test]
    fn web_search_skill_delegates_to_the_tool_it_declares() {
        drive(async {
            let invocation = SkillCatalog::default()
                .resolve("/web-search rust release notes")
                .expect("slash command parses")
                .expect("web-search is a real builtin");
            assert_eq!(invocation.arguments, "rust release notes");
            assert_eq!(invocation.skill.tools, [WEB_SEARCH_AGENT_ID]);
            let declared = invocation.skill.tools[0].clone();

            let hits = Arc::new(AtomicUsize::new(0));
            let recorded = Arc::clone(&hits);
            let client = SearchClient::from_fn(move |_query| {
                recorded.fetch_add(1, Ordering::SeqCst);
                let body = SEARCH_BODY.to_owned();
                async move { Ok(body) }
            });
            let llm = Arc::new(ProcedureLlm::new(vec!["rust release notes".to_owned()]));
            let harness = harness_with(Arc::clone(&llm) as Arc<dyn Llm>)
                .with_web_search(client)
                .expect("search orchestrator")
                .with_skill(&invocation.skill)
                .expect("the builtin skill attaches");
            let (tx, rx) = async_channel::unbounded();
            harness
                .run_prompt("skill-session", invocation.arguments, &tx)
                .await;
            let answer = collect_answer(&rx).await;
            let delegated = llm.delegated.lock().expect("delegation record").clone();
            let (tool, prompt) = delegated.expect("the parent called the skill's tool");
            assert_eq!(tool, declared);
            assert!(prompt.contains("rust release notes"), "{prompt}");
            assert!(
                !prompt.to_ascii_lowercase().contains("search the web"),
                "{prompt}"
            );
            assert!(answer.contains(HIT_TITLE), "{answer}");
            assert!(answer.contains(HIT_URL), "{answer}");
            assert_eq!(answer.matches(HIT_URL).count(), 1, "{answer}");
            assert!(!answer.contains("https://invented.example"), "{answer}");
            assert!(hits.load(Ordering::SeqCst) >= 1, "SearXNG was not called");

            let plain_hits = Arc::new(AtomicUsize::new(0));
            let plain_recorded = Arc::clone(&plain_hits);
            let plain_client = SearchClient::from_fn(move |_query| {
                plain_recorded.fetch_add(1, Ordering::SeqCst);
                let body = SEARCH_BODY.to_owned();
                async move { Ok(body) }
            });
            let plain =
                harness_with(
                    Arc::new(ProcedureLlm::new(vec!["rust release notes".to_owned()]))
                        as Arc<dyn Llm>,
                )
                .with_web_search(plain_client)
                .expect("plain orchestrator");
            let (plain_tx, plain_rx) = async_channel::unbounded();
            plain
                .run_prompt("plain-session", "rust release notes", &plain_tx)
                .await;
            let plain_answer = collect_answer(&plain_rx).await;
            assert_eq!(plain_hits.load(Ordering::SeqCst), 0);
            assert!(!plain_answer.contains(HIT_URL), "{plain_answer}");

            let disabled = harness_with(Arc::new(IdleLlm))
                .with_web_search(SearchClient::failing("off"))
                .expect("enabled")
                .without_web_search()
                .expect("disabled");
            assert!(disabled.with_skill(&invocation.skill).is_err());
        });
    }

    async fn run_phrase(prompt: &str, attachment: bool) -> (String, usize) {
        let hits = Arc::new(AtomicUsize::new(0));
        let recorded = Arc::clone(&hits);
        let client = SearchClient::from_fn(move |_query| {
            recorded.fetch_add(1, Ordering::SeqCst);
            let body = SEARCH_BODY.to_owned();
            async move { Ok(body) }
        });
        let harness = harness_with(Arc::new(CitingLlm::new(vec![
            "rust release notes".to_owned(),
        ])))
        .with_web_search(client)
        .expect("search orchestrator");
        assert_eq!(harness.orchestrator_name(), ORCHESTRATOR_ID);
        let (tx, rx) = async_channel::unbounded();
        if attachment {
            let harness = harness
                .with_files(Arc::new(vec![sample_file()]), true)
                .expect("files stay on the orchestrator");
            assert_eq!(harness.orchestrator_name(), ORCHESTRATOR_ID);
            assert!(
                harness
                    .delegates()
                    .iter()
                    .any(|(id, _)| id == WEB_SEARCH_AGENT_ID)
            );
            harness
                .run_with_files("file-session", prompt, 4_000, &tx)
                .await;
        } else {
            harness.run_prompt("search-session", prompt, &tx).await;
        }
        (collect_answer(&rx).await, hits.load(Ordering::SeqCst))
    }

    fn sample_file() -> FileContent {
        FileContent {
            raw: None,
            attachment: Attachment {
                id: "notes".into(),
                name: "notes.txt".into(),
                mime_type: "text/plain".into(),
                size_bytes: 18,
                purpose: ModelPurpose::Documents,
                sections: 1,
                note: None,
                thumbnail: Vec::new(),
            },
            text: "local file marker".into(),
            media: Vec::new(),
        }
    }

    async fn collect_answer(rx: &Receiver<Event>) -> String {
        let mut answer = String::new();
        while let Ok(event) = rx.recv().await {
            match event {
                Event::AgentChunk { text, .. } => answer.push_str(&text),
                Event::AgentCompleted { .. } => break,
                Event::AgentError { message, .. } => panic!("agent error: {message}"),
                _ => {}
            }
        }
        answer
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
            Err(adk_core::AdkError::model(
                "roster inspection does not call the model",
            ))
        }
    }

    struct CitingLlm {
        queries: Vec<String>,
        calls: AtomicUsize,
    }

    impl CitingLlm {
        fn new(queries: Vec<String>) -> Self {
            Self {
                queries,
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl Llm for CitingLlm {
        fn name(&self) -> &'static str {
            "citing-orchestrator"
        }

        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            if request.tools.contains_key(WEB_SEARCH_AGENT_ID) {
                if let Some(text) = delegated_response(&request, WEB_SEARCH_AGENT_ID) {
                    return Ok(once_text(&text));
                }
                let prompt = user_text(&request);
                if is_short_greeting(&prompt) {
                    return Ok(once_text("Hello"));
                }
                return Ok(once_call(
                    WEB_SEARCH_AGENT_ID,
                    json!({ "request": prompt }),
                    "delegate-web",
                ));
            }
            if request.tools.contains_key("web_search") {
                let call = self.calls.fetch_add(1, Ordering::SeqCst);
                let response = if call == 0 {
                    once_call(
                        "web_search",
                        json!({ "queries": self.queries.clone() }),
                        "search-1",
                    )
                } else {
                    once_text(&cite_tool_result(&request))
                };
                return Ok(response);
            }
            Ok(once_text("Hello"))
        }
    }

    /// Parent that calls only the tool named by the active skill, and only when
    /// the shipped instruction orders that call. The child still uses `web_search`.
    struct ProcedureLlm {
        queries: Vec<String>,
        calls: AtomicUsize,
        delegated: Mutex<Option<(String, String)>>,
    }

    impl ProcedureLlm {
        fn new(queries: Vec<String>) -> Self {
            Self {
                queries,
                calls: AtomicUsize::new(0),
                delegated: Mutex::new(None),
            }
        }
    }

    #[async_trait::async_trait]
    impl Llm for ProcedureLlm {
        fn name(&self) -> &'static str {
            "procedure-orchestrator"
        }

        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            if request.tools.contains_key("web_search") {
                let call = self.calls.fetch_add(1, Ordering::SeqCst);
                let response = if call == 0 {
                    once_call(
                        "web_search",
                        json!({ "queries": self.queries.clone() }),
                        "skill-search",
                    )
                } else {
                    once_text(&cite_tool_result(&request))
                };
                return Ok(response);
            }
            let corpus = instruction_corpus(&request);
            let Some(tool) = super::skill_block(&corpus).and_then(|value| {
                value
                    .get("requested_tools")
                    .and_then(Value::as_array)
                    .and_then(|tools| tools.first())
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            }) else {
                return Ok(once_text("answered directly"));
            };
            if !request.tools.contains_key(&tool)
                || !instruction_orders_declared_tool(&corpus, &tool)
            {
                return Ok(once_text("instruction did not order the declared tool"));
            }
            if let Some(text) = delegated_response(&request, &tool) {
                return Ok(once_text(&text));
            }
            let prompt = user_text(&request);
            *self.delegated.lock().expect("delegation record") =
                Some((tool.clone(), prompt.clone()));
            Ok(once_call(
                &tool,
                json!({ "request": prompt }),
                "skill-delegate",
            ))
        }
    }

    fn instruction_corpus(request: &LlmRequest) -> String {
        let mut text = String::new();
        for content in &request.contents {
            for part in &content.parts {
                if let Part::Text { text: chunk } = part {
                    text.push_str(chunk);
                    text.push('\n');
                }
            }
        }
        text
    }

    fn instruction_orders_declared_tool(corpus: &str, tool: &str) -> bool {
        let lower = corpus.to_ascii_lowercase();
        let tool_lower = tool.to_ascii_lowercase();
        lower.contains("the active procedure requests")
            && lower.contains(&format!("call the {tool_lower} tool"))
            && !lower.contains(&format!("without calling {tool_lower}"))
    }

    struct RecordingLlm {
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl Llm for RecordingLlm {
        fn name(&self) -> &'static str {
            "recording-orchestrator"
        }

        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            self.seen
                .lock()
                .expect("seen tools")
                .extend(request.tools.keys().cloned());
            Ok(once_text("Hello"))
        }
    }

    struct BranchNotes {
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        web: Mutex<String>,
        code: Mutex<String>,
    }

    impl Default for BranchNotes {
        fn default() -> Self {
            Self {
                in_flight: AtomicUsize::new(0),
                max_in_flight: AtomicUsize::new(0),
                web: Mutex::new(String::new()),
                code: Mutex::new(String::new()),
            }
        }
    }

    struct ParallelLlm {
        notes: Arc<BranchNotes>,
        release_tx: Sender<()>,
        release_rx: Receiver<()>,
        web_calls: AtomicUsize,
        parent_calls: AtomicUsize,
    }

    impl ParallelLlm {
        async fn wait_for_peer(&self) {
            let now = self.notes.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.notes.max_in_flight.fetch_max(now, Ordering::SeqCst);
            if now == 2 {
                let _ = self.release_tx.try_send(());
                let _ = self.release_tx.try_send(());
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), self.release_rx.recv()).await;
            self.notes.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl Llm for ParallelLlm {
        fn name(&self) -> &'static str {
            "parallel-orchestrator"
        }

        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            if request.tools.contains_key("web_search") {
                let call = self.web_calls.fetch_add(1, Ordering::SeqCst);
                if call == 0 {
                    *self.notes.web.lock().expect("web text") = user_text(&request);
                    self.wait_for_peer().await;
                    return Ok(once_call(
                        "web_search",
                        json!({ "queries": ["keep-query", "bad-query"] }),
                        "search-sibling",
                    ));
                }
                return Ok(once_text(&cite_tool_result(&request)));
            }
            if request.tools.contains_key(WEB_SEARCH_AGENT_ID) {
                let call = self.parent_calls.fetch_add(1, Ordering::SeqCst);
                if let Some(text) = delegated_response(&request, WEB_SEARCH_AGENT_ID) {
                    return Ok(once_text(&text));
                }
                if call > 0 {
                    return Ok(once_text("parent did not see the citation"));
                }
                return Ok(two_delegations());
            }
            *self.notes.code.lock().expect("code text") = user_text(&request);
            self.wait_for_peer().await;
            Err(adk_core::AdkError::agent("coding branch failed"))
        }
    }

    fn two_delegations() -> LlmResponseStream {
        let response = LlmResponse {
            content: Some(Content {
                role: "model".to_owned(),
                parts: vec![
                    function_part(
                        WEB_SEARCH_AGENT_ID,
                        json!({ "request": "search-notes" }),
                        "call-web",
                    ),
                    function_part(
                        "CodingAgent",
                        json!({ "request": "coding-only-marker" }),
                        "call-code",
                    ),
                ],
            }),
            finish_reason: Some(FinishReason::Stop),
            partial: false,
            turn_complete: true,
            ..LlmResponse::default()
        };
        Box::pin(stream::once(async move { Ok(response) }))
    }

    fn function_part(name: &str, args: Value, id: &str) -> Part {
        Part::FunctionCall {
            name: name.to_owned(),
            args,
            id: Some(id.to_owned()),
            thought_signature: None,
        }
    }

    fn once_call(name: &str, args: Value, id: &str) -> LlmResponseStream {
        let response = LlmResponse {
            content: Some(Content {
                role: "model".to_owned(),
                parts: vec![function_part(name, args, id)],
            }),
            finish_reason: Some(FinishReason::Stop),
            partial: false,
            turn_complete: true,
            ..LlmResponse::default()
        };
        Box::pin(stream::once(async move { Ok(response) }))
    }

    fn once_text(text: &str) -> LlmResponseStream {
        let response = LlmResponse {
            content: Some(Content::new("model").with_text(text)),
            finish_reason: Some(FinishReason::Stop),
            partial: false,
            turn_complete: true,
            ..LlmResponse::default()
        };
        Box::pin(stream::once(async move { Ok(response) }))
    }

    fn user_text(request: &LlmRequest) -> String {
        // The agent instruction is an earlier user message. The turn's prompt
        // is the latest user message, which is all a delegate is given.
        for content in request.contents.iter().rev() {
            if content.role != "user" {
                continue;
            }
            let mut text = String::new();
            for part in &content.parts {
                if let Part::Text { text: chunk } = part {
                    text.push_str(chunk);
                    text.push('\n');
                }
            }
            if !text.is_empty() {
                return text;
            }
        }
        String::new()
    }

    fn is_short_greeting(prompt: &str) -> bool {
        let lower = prompt.trim().to_ascii_lowercase();
        let words = lower.split_whitespace().count();
        words <= 12
            && !lower.contains("search")
            && (lower.starts_with("hi")
                || lower.starts_with("hello")
                || lower.starts_with("hey")
                || lower.starts_with("thanks"))
    }

    fn delegated_response(request: &LlmRequest, name: &str) -> Option<String> {
        for content in request.contents.iter().rev() {
            for part in content.parts.iter().rev() {
                let Part::FunctionResponse {
                    function_response, ..
                } = part
                else {
                    continue;
                };
                if function_response.name != name {
                    continue;
                }
                return function_response
                    .response
                    .get("response")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
            }
        }
        None
    }

    fn cite_tool_result(request: &LlmRequest) -> String {
        let mut lines = Vec::new();
        for content in &request.contents {
            for part in &content.parts {
                let Part::FunctionResponse {
                    function_response, ..
                } = part
                else {
                    continue;
                };
                let Some(results) = function_response
                    .response
                    .get("results")
                    .and_then(Value::as_array)
                else {
                    continue;
                };
                for hit in results {
                    let title = hit.get("title").and_then(Value::as_str).unwrap_or("");
                    let url = hit.get("url").and_then(Value::as_str).unwrap_or("");
                    if !url.is_empty() {
                        lines.push(format!("{title} {url}"));
                    }
                }
            }
        }
        if lines.is_empty() {
            "no tool result".to_owned()
        } else {
            lines.join("\n")
        }
    }
}
