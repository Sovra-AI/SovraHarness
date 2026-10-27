//! Keyless `SearXNG` client and the read-only `web_search` tool.
//!
//! One tool call may carry several queries. They run together, up to
//! [`SEARCH_QUERY_CONCURRENCY`], and a failed query does not discard the others.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use adk_core::{Tool, ToolContext};
use futures::stream::{self, StreamExt as _};
use serde_json::{Value, json};
use sovra_core::{QueryOutcome, RankedSearch, rank_search_outcomes};

use crate::Error;

/// How many queries inside one `web_search` call may be in flight at once.
pub(crate) const SEARCH_QUERY_CONCURRENCY: usize = 4;

const MAX_QUERIES: usize = 8;
const WEB_SEARCH_TOOL_NAME: &str = "web_search";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

const WEB_SEARCH_DESCRIPTION: &str = "\
Search the public web through the configured SearXNG server. Pass one or more queries in \
`queries`. The call is read-only and runs the queries together. The result lists `results` \
with title, url, and snippet taken from the server, plus `errors` for queries that failed. \
Cite only those titles and URLs. Do not invent sources.";

type SearchFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;
type SearchFn = Arc<dyn Fn(String) -> SearchFuture + Send + Sync>;

/// Injectable `SearXNG` JSON client. Cloning shares the same backend.
#[derive(Clone)]
pub struct SearchClient {
    search: SearchFn,
}

impl std::fmt::Debug for SearchClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SearchClient")
            .finish_non_exhaustive()
    }
}

impl SearchClient {
    /// HTTP client for `GET {base}/search?q=&format=json`. No API key is sent.
    pub fn http(base_url: impl Into<String>) -> Result<Self, Error> {
        let base_url = normalize_base(&base_url.into())?;
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(2))
            .build()
            .map_err(|error| Error::SearchClient(error.to_string()))?;
        Ok(Self::from_fn(move |query| {
            let client = client.clone();
            let base_url = base_url.clone();
            async move { fetch_searxng(&client, &base_url, &query).await }
        }))
    }

    /// Client whose every query fails with `message`. Used when the HTTP client cannot be built.
    pub fn failing(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::from_fn(move |_| {
            let message = message.clone();
            async move { Err(message) }
        })
    }

    /// Test and custom backends. The closure receives one query and returns a `SearXNG` JSON body.
    pub fn from_fn<F, Fut>(search: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, String>> + Send + 'static,
    {
        Self {
            search: Arc::new(move |query| Box::pin(search(query))),
        }
    }

    pub(crate) async fn search_many(&self, queries: Vec<String>) -> Vec<QueryOutcome> {
        let mut indexed = stream::iter(queries.into_iter().enumerate())
            .map(|(index, query)| {
                let search = Arc::clone(&self.search);
                async move {
                    let body = search(query.clone()).await;
                    (index, QueryOutcome { query, body })
                }
            })
            .buffer_unordered(SEARCH_QUERY_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        indexed.sort_by_key(|(index, _)| *index);
        indexed.into_iter().map(|(_, outcome)| outcome).collect()
    }
}

pub(crate) fn web_search_tool(client: SearchClient) -> Arc<dyn Tool> {
    Arc::new(WebSearchTool { client })
}

pub(crate) fn web_search_instruction(base: &str) -> String {
    format!(
        "{base}\n\nThis turn is handled by WebSearchAgent. Call the web_search tool before you \
         answer. You may pass several queries in one call; they run at the same time, and one \
         failure does not drop the others. Cite the title and URL of every source you use, and \
         use only titles and URLs present in the tool result. If the tool returns an error or no \
         results, say that the search failed. Do not invent links, titles, or page contents."
    )
}

pub(crate) fn web_search_disabled_instruction(base: &str) -> String {
    format!(
        "{base}\n\nWeb search is turned off for this turn. Do not claim to have searched the web \
         and do not invent titles or URLs."
    )
}

pub(crate) fn chat_tools(native_ui: bool, search: Option<SearchClient>) -> Vec<Arc<dyn Tool>> {
    let mut tools = if native_ui {
        crate::native_ui::tools()
    } else {
        Vec::new()
    };
    if let Some(client) = search {
        tools.push(web_search_tool(client));
    }
    tools
}

pub(crate) fn searxng_search_url(base: &str, query: &str) -> Result<String, String> {
    let base = base.trim().trim_end_matches('/');
    let mut url =
        reqwest::Url::parse(&format!("{base}/search")).map_err(|error| error.to_string())?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.clear();
        pairs.append_pair("q", query);
        pairs.append_pair("format", "json");
    }
    if url.query_pairs().any(|(key, _)| {
        let key = key.to_ascii_lowercase();
        key.contains("key") || key.contains("token") || key.contains("secret")
    }) {
        return Err("search URL must not carry an API key".to_owned());
    }
    Ok(url.to_string())
}

struct WebSearchTool {
    client: SearchClient,
}

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        WEB_SEARCH_TOOL_NAME
    }

    fn description(&self) -> &str {
        WEB_SEARCH_DESCRIPTION
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "queries": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "One or more search queries to run together."
                }
            },
            "required": ["queries"]
        }))
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        let queries = queries_from_args(&args).map_err(adk_core::AdkError::tool)?;
        let outcomes = self.client.search_many(queries).await;
        let ranked: RankedSearch = rank_search_outcomes(&outcomes);
        tracing::debug!(
            hits = ranked.hits.len(),
            errors = ranked.errors.len(),
            "web search finished"
        );
        serde_json::to_value(ranked).map_err(|error| adk_core::AdkError::tool(error.to_string()))
    }
}

fn queries_from_args(args: &Value) -> Result<Vec<String>, String> {
    let mut queries = Vec::new();
    if let Some(values) = args.get("queries").and_then(Value::as_array) {
        for value in values {
            push_query(&mut queries, value.as_str());
        }
    }
    if queries.is_empty() {
        push_query(&mut queries, args.get("query").and_then(Value::as_str));
    }
    if queries.is_empty() {
        return Err("web_search requires one or more queries".to_owned());
    }
    queries.truncate(MAX_QUERIES);
    Ok(queries)
}

fn push_query(queries: &mut Vec<String>, raw: Option<&str>) {
    let Some(raw) = raw else {
        return;
    };
    let query = raw.trim();
    if query.is_empty() || queries.iter().any(|existing| existing == query) {
        return;
    }
    queries.push(query.chars().take(300).collect());
}

fn normalize_base(base: &str) -> Result<String, Error> {
    let base = base.trim().trim_end_matches('/').to_owned();
    let url = reqwest::Url::parse(&base).map_err(|error| Error::SearchClient(error.to_string()))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(Error::SearchClient(
            "SearXNG base URL must be http or https".to_owned(),
        ));
    }
    if url.host_str().is_none() {
        return Err(Error::SearchClient(
            "SearXNG base URL needs a host".to_owned(),
        ));
    }
    Ok(base)
}

async fn fetch_searxng(
    client: &reqwest::Client,
    base: &str,
    query: &str,
) -> Result<String, String> {
    let url = searxng_search_url(base, query)?;
    let response = client
        .get(url)
        .header(http::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("SearXNG HTTP {}", response.status()));
    }
    response.text().await.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::AdkHarness;
    use adk_core::{Content, FinishReason, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part};
    use async_channel::Receiver;
    use sovra_core::{AgentConfig, Event, WEB_SEARCH_AGENT_ID};

    #[test]
    fn search_url_is_keyless_searxng_json() {
        let url = searxng_search_url("https://searx.example/", "rust async").expect("url");
        assert!(url.starts_with("https://searx.example/search?"));
        assert!(url.contains("format=json"));
        assert!(url.contains("q=rust"));
        assert!(!url.to_ascii_lowercase().contains("api_key"));
        assert!(!url.to_ascii_lowercase().contains("token"));
    }

    #[test]
    fn disabled_tool_list_omits_web_search() {
        let disabled = chat_tools(false, None);
        assert!(
            disabled
                .iter()
                .all(|tool| tool.name() != WEB_SEARCH_TOOL_NAME)
        );
        let enabled = chat_tools(
            false,
            Some(SearchClient::from_fn(|_query| async {
                Ok("{}".to_owned())
            })),
        );
        let tool = enabled
            .iter()
            .find(|tool| tool.name() == WEB_SEARCH_TOOL_NAME)
            .expect("enabled list includes web_search");
        assert!(tool.is_read_only());
        assert!(tool.is_concurrency_safe());
    }

    /// ADK wraps each tool call in `tokio::time::timeout`, so the real harness
    /// path needs a timer. This crate's library code does not depend on tokio.
    fn drive<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("ADK tool timeout needs a tokio timer")
            .block_on(future)
    }

    #[test]
    fn harness_cites_injected_searxng_hit_and_omits_tool_when_disabled() {
        drive(async {
            let body = r#"{"results":[
                {"title":"Rust Release Notes","url":"https://doc.rust-lang.org/releases.html","content":"notes"},
                {"title":"Duplicate","url":"https://doc.rust-lang.org/releases.html#top","content":"again"},
                {"title":"Broken","content":"missing url"}
            ]}"#;
            let client = SearchClient::from_fn(move |_query| {
                let body = body.to_owned();
                async move { Ok(body) }
            });
            let llm = Arc::new(CitingLlm::new(vec!["rust release notes".to_owned()]));
            let base = AdkHarness::with_llm_and_tools(
                &AgentConfig::default(),
                Arc::clone(&llm) as Arc<dyn Llm>,
                Vec::new(),
            )
            .expect("harness");
            let harness = base.with_web_search(client).expect("search harness");
            let (tx, rx) = async_channel::unbounded();
            harness
                .run_prompt(
                    "search-session",
                    "search the web for rust release notes",
                    &tx,
                )
                .await;
            let answer = collect_answer(&rx).await;
            assert!(
                answer.contains("Rust Release Notes"),
                "answer missing title: {answer}"
            );
            assert!(
                answer.contains("https://doc.rust-lang.org/releases.html"),
                "answer missing url: {answer}"
            );
            assert!(!answer.contains("https://invented.example"));

            let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
            let recorder = Arc::new(RecordingLlm {
                seen: Arc::clone(&seen),
            });
            let plain = AdkHarness::with_llm_and_tools(
                &AgentConfig::default(),
                recorder,
                chat_tools(false, None),
            )
            .expect("disabled harness");
            let (tx, rx) = async_channel::unbounded();
            plain.run_prompt("plain", "hello", &tx).await;
            let _ = collect_answer(&rx).await;
            let names = seen.lock().expect("tool names");
            assert!(
                names.iter().all(|name| name != WEB_SEARCH_TOOL_NAME),
                "disabled turn exposed {names:?}"
            );
        });
    }

    #[test]
    fn multi_query_overlaps_and_keeps_the_sibling_citation() {
        drive(async {
            let expected = 3_usize;
            let started = Arc::new(AtomicUsize::new(0));
            let in_flight = Arc::new(AtomicUsize::new(0));
            let max_in_flight = Arc::new(AtomicUsize::new(0));
            let (release_tx, release_rx) = async_channel::unbounded::<()>();
            let client = SearchClient::from_fn({
                let started = Arc::clone(&started);
                let in_flight = Arc::clone(&in_flight);
                let max_in_flight = Arc::clone(&max_in_flight);
                let release_tx = release_tx.clone();
                let release_rx = release_rx.clone();
                move |query: String| {
                    let started = Arc::clone(&started);
                    let in_flight = Arc::clone(&in_flight);
                    let max_in_flight = Arc::clone(&max_in_flight);
                    let release_tx = release_tx.clone();
                    let release_rx = release_rx.clone();
                    async move {
                        let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                        max_in_flight.fetch_max(now, Ordering::SeqCst);
                        let seen = started.fetch_add(1, Ordering::SeqCst) + 1;
                        if seen == expected {
                            for _ in 0..expected {
                                let _ = release_tx.try_send(());
                            }
                        }
                        let _ = release_rx.recv().await;
                        in_flight.fetch_sub(1, Ordering::SeqCst);
                        match query.as_str() {
                            "one" => Ok(r#"{"results":[{"title":"One","url":"https://one.example/a","content":"a"}]}"#.to_owned()),
                            "two" => Ok(r#"{"results":[{"title":"Two","url":"https://two.example/b","content":"b"},{"title":"Dup","url":"https://one.example/a","content":"dup"}]}"#.to_owned()),
                            "bad" => Err("upstream timeout".to_owned()),
                            other => Err(format!("unexpected query {other}")),
                        }
                    }
                }
            });
            let llm = Arc::new(CitingLlm::new(vec![
                "one".to_owned(),
                "two".to_owned(),
                "bad".to_owned(),
            ]));
            let base = AdkHarness::with_llm_and_tools(&AgentConfig::default(), llm, Vec::new())
                .expect("harness");
            let harness = base.with_web_search(client).expect("search harness");
            let (tx, rx) = async_channel::unbounded();
            harness
                .run_prompt("overlap", "search the web for both", &tx)
                .await;
            let answer = collect_answer(&rx).await;
            assert!(
                max_in_flight.load(Ordering::SeqCst) >= 2,
                "queries did not overlap"
            );
            assert!(answer.contains("https://one.example/a"), "{answer}");
            assert!(answer.contains("https://two.example/b"), "{answer}");
            assert!(answer.contains("One"), "{answer}");
            assert!(answer.contains("Two"), "{answer}");
            assert!(!answer.contains("https://invented.example"));
            assert_eq!(answer.matches("https://one.example/a").count(), 1);
        });
    }

    #[test]
    fn two_sessions_cite_only_their_own_url() {
        drive(async {
            let client = SearchClient::from_fn(|query| async move {
                let (title, url) = match query.as_str() {
                    "alpha" => ("Alpha", "https://alpha.example/a"),
                    "beta" => ("Beta", "https://beta.example/b"),
                    other => return Err(format!("unexpected query {other}")),
                };
                Ok(format!(
                    r#"{{"results":[{{"title":"{title}","url":"{url}","content":"hit"}}]}}"#
                ))
            });
            let left = session_harness(client.clone(), "alpha");
            let right = session_harness(client, "beta");
            let (left_tx, left_rx) = async_channel::unbounded();
            let (right_tx, right_rx) = async_channel::unbounded();
            let left_run = left.run_prompt("session-alpha", "search the web for alpha", &left_tx);
            let right_run = right.run_prompt("session-beta", "search the web for beta", &right_tx);
            futures::future::join(left_run, right_run).await;
            let alpha = collect_answer(&left_rx).await;
            let beta = collect_answer(&right_rx).await;
            assert!(alpha.contains("https://alpha.example/a"), "{alpha}");
            assert!(!alpha.contains("https://beta.example/b"), "{alpha}");
            assert!(beta.contains("https://beta.example/b"), "{beta}");
            assert!(!beta.contains("https://alpha.example/a"), "{beta}");
            let _ = WEB_SEARCH_AGENT_ID;
        });
    }

    fn session_harness(client: SearchClient, query: &str) -> AdkHarness {
        let llm = Arc::new(CitingLlm::new(vec![query.to_owned()]));
        let base = AdkHarness::with_llm_and_tools(&AgentConfig::default(), llm, Vec::new())
            .expect("harness");
        base.with_web_search(client).expect("search harness")
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
            "citing-search"
        }

        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            // The parent orchestrator calls WebSearchAgent. The child is the
            // only agent whose tool list contains web_search.
            if request.tools.contains_key(WEB_SEARCH_AGENT_ID) {
                if let Some(text) = delegated_response(&request, WEB_SEARCH_AGENT_ID) {
                    let response = text_response(&text);
                    return Ok(Box::pin(stream::once(async move { Ok(response) })));
                }
                let prompt = user_text(&request);
                if is_short_greeting(&prompt) {
                    let response = text_response("Hello");
                    return Ok(Box::pin(stream::once(async move { Ok(response) })));
                }
                let response = function_call_named(
                    WEB_SEARCH_AGENT_ID,
                    json!({ "request": prompt }),
                    "delegate-web",
                );
                return Ok(Box::pin(stream::once(async move { Ok(response) })));
            }
            if request.tools.contains_key(WEB_SEARCH_TOOL_NAME) {
                let call = self.calls.fetch_add(1, Ordering::SeqCst);
                let response = if call == 0 {
                    function_call(json!({ "queries": self.queries.clone() }))
                } else {
                    text_response(&cite_tool_result(&request))
                };
                return Ok(Box::pin(stream::once(async move { Ok(response) })));
            }
            let response = text_response("Hello");
            Ok(Box::pin(stream::once(async move { Ok(response) })))
        }
    }

    struct RecordingLlm {
        seen: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl Llm for RecordingLlm {
        fn name(&self) -> &'static str {
            "recording-search"
        }

        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            let mut seen = self.seen.lock().expect("seen tools");
            seen.extend(request.tools.keys().cloned());
            drop(seen);
            let response = text_response("hello");
            Ok(Box::pin(stream::once(async move { Ok(response) })))
        }
    }

    fn function_call(args: Value) -> LlmResponse {
        function_call_named(WEB_SEARCH_TOOL_NAME, args, "search-1")
    }

    fn function_call_named(name: &str, args: Value, id: &str) -> LlmResponse {
        LlmResponse {
            content: Some(Content {
                role: "model".to_owned(),
                parts: vec![Part::FunctionCall {
                    name: name.to_owned(),
                    args,
                    id: Some(id.to_owned()),
                    thought_signature: None,
                }],
            }),
            finish_reason: Some(FinishReason::Stop),
            partial: false,
            turn_complete: true,
            ..LlmResponse::default()
        }
    }

    fn text_response(text: &str) -> LlmResponse {
        LlmResponse {
            content: Some(Content::new("model").with_text(text)),
            finish_reason: Some(FinishReason::Stop),
            partial: false,
            turn_complete: true,
            ..LlmResponse::default()
        }
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

    fn user_text(request: &LlmRequest) -> String {
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
