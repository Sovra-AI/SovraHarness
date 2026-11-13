//! Final provider-boundary admission after all context middleware has run.

use adk_core::{Content, Llm, LlmRequest, LlmResponseStream, Part};
use futures::StreamExt as _;
use serde::{Deserialize, Serialize};
use sovra_core::{ContextBudget, ContextLimitError, ContextTrim};
use std::sync::Arc;

// Private middleware metadata is removed before measuring or calling a provider.
// Indices identify our injected contents, never strings found in user text.
const MARKS: &str = "sovra_optional_context";
const MEDIA_DURATION: &str = "sovra_media_duration_millis";
pub(crate) const ERROR_CODE: &str = "model.sovra.context_budget";
const OMITTED_HISTORY: &str = "Earlier conversation was shortened to fit this request. Ask for missing details rather than inventing them; no earlier actions were replayed.";
const OMITTED_MEMORY: &str = "Saved evidence was omitted to fit this request. Do not invent remembered facts or treat missing evidence as proof of absence.";
const NOTICE: &str =
    "Some saved evidence was omitted to fit this request; do not infer a complete history.";

#[derive(Default, Serialize, Deserialize)]
struct Marks {
    history: Vec<usize>,
    history_guide: Option<usize>,
    memory: Option<usize>,
}
impl Marks {
    fn inserted(&mut self, at: usize, count: usize) {
        for index in self
            .history
            .iter_mut()
            .chain(self.history_guide.iter_mut())
            .chain(self.memory.iter_mut())
        {
            if *index >= at {
                *index = index.saturating_add(count);
            }
        }
    }
    fn removed(&mut self, at: usize) {
        for index in self
            .history
            .iter_mut()
            .chain(self.history_guide.iter_mut())
            .chain(self.memory.iter_mut())
        {
            if *index > at {
                *index -= 1;
            }
        }
    }
    fn valid(&self, len: usize) -> bool {
        self.history
            .iter()
            .chain(self.history_guide.iter())
            .chain(self.memory.iter())
            .all(|index| *index < len)
            && self.history.windows(2).all(|pair| pair[0] < pair[1])
            && self
                .history_guide
                .is_none_or(|index| !self.history.contains(&index))
            && self.memory.is_none_or(|index| {
                !self.history.contains(&index) && self.history_guide != Some(index)
            })
    }
}

fn error() -> adk_core::AdkError {
    adk_core::AdkError::new(
        adk_core::ErrorComponent::Model,
        adk_core::ErrorCategory::InvalidInput,
        ERROR_CODE,
        ContextLimitError.to_string(),
    )
}

fn take_marks(request: &mut LlmRequest) -> adk_core::Result<Marks> {
    request
        .config
        .as_mut()
        .and_then(|config| config.extensions.remove(MARKS))
        .map_or_else(
            || Ok(Marks::default()),
            |value| serde_json::from_value(value).map_err(|_| error()),
        )
}

pub(crate) fn mark_history(
    request: &mut LlmRequest,
    at: usize,
    messages: usize,
) -> adk_core::Result<()> {
    let mut marks = take_marks(request)?;
    marks.inserted(at, messages + 1);
    marks.history = ((at + 1)..(at + 1 + messages)).collect();
    marks.history_guide = Some(at);
    put_marks(request, &marks)
}

pub(crate) fn mark_memory(request: &mut LlmRequest, at: usize) -> adk_core::Result<()> {
    let mut marks = take_marks(request)?;
    marks.inserted(at, 1);
    marks.memory = Some(at);
    put_marks(request, &marks)
}

fn put_marks(request: &mut LlmRequest, marks: &Marks) -> adk_core::Result<()> {
    request.config.get_or_insert_default().extensions.insert(
        MARKS.into(),
        serde_json::to_value(marks).map_err(|_| error())?,
    );
    Ok(())
}

struct BudgetModel {
    inner: Arc<dyn Llm>,
    budget: ContextBudget,
}

pub(crate) fn wrap(inner: Arc<dyn Llm>, budget: ContextBudget) -> Arc<dyn Llm> {
    Arc::new(BudgetModel { inner, budget })
}

#[async_trait::async_trait]
impl Llm for BudgetModel {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn schema_adapter(&self) -> &dyn adk_core::SchemaAdapter {
        self.inner.schema_adapter()
    }
    fn uses_interactions_api(&self) -> bool {
        self.inner.uses_interactions_api()
    }
    async fn generate_content(
        &self,
        mut request: LlmRequest,
        stream: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        prepare(&mut request, self.budget)?;
        strip_media_hints(&mut request);
        let mut call_ids = request
            .contents
            .iter()
            .flat_map(|content| &content.parts)
            .filter_map(|part| match part {
                Part::FunctionCall { id, .. } | Part::FunctionResponse { id, .. } => id.clone(),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        let responses = self.inner.generate_content(request, stream).await?;
        Ok(Box::pin(responses.map(move |result| {
            result.map(|mut response| {
                response.interaction_id = None;
                if !response.partial
                    && let Some(content) = &mut response.content
                {
                    complete_call_ids(content, &mut call_ids);
                }
                response
            })
        })))
    }
}

// Some compatible providers parse textual tool calls without assigning an ID.
// Assign before ADK executes the call so its result and saved history share it.
pub(crate) fn complete_call_ids(
    content: &mut Content,
    used: &mut std::collections::HashSet<String>,
) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    for part in &content.parts {
        if let Part::FunctionCall { id: Some(id), .. } = part {
            used.insert(id.clone());
        }
    }
    for part in &mut content.parts {
        if let Part::FunctionCall { id, .. } = part
            && id.as_ref().is_none_or(|id| id.trim().is_empty())
        {
            loop {
                let candidate = format!("sovra_call_{}", NEXT.fetch_add(1, Ordering::Relaxed));
                if used.insert(candidate.clone()) {
                    *id = Some(candidate);
                    break;
                }
            }
        }
    }
}

fn prepare(request: &mut LlmRequest, budget: ContextBudget) -> adk_core::Result<()> {
    let mut marks = take_marks(request)?;
    if !marks.valid(request.contents.len()) {
        return Err(error());
    }
    let config = request.config.get_or_insert_default();
    if config.cached_content.is_some() {
        return Err(error());
    }
    let output = config
        .max_output_tokens
        .map(u64::try_from)
        .transpose()
        .map_err(|_| error())?;
    let allowance = budget.allowance(output).map_err(|_| error())?;
    config.max_output_tokens =
        Some(i32::try_from(allowance.output_tokens).expect("output allowance fits i32"));
    // Hidden server history cannot be included in a local admission calculation.
    request.previous_response_id = None;
    let mut evidence = marks
        .memory
        .map(|index| match request.contents[index].parts.get(1) {
            Some(Part::Text { text }) => {
                serde_json::from_str::<serde_json::Value>(text).map_err(|_| error())
            }
            _ => Err(error()),
        })
        .transpose()?;
    loop {
        if units(request)? <= allowance.input_units {
            return Ok(());
        }
        let memories = evidence.as_ref().map_or(0, |value| {
            value["memories"].as_array().map_or(0, Vec::len)
                + value["sources"].as_array().map_or(0, Vec::len)
        });
        match ContextBudget::next_trim(marks.history.len(), memories) {
            Some(ContextTrim::OldestHistory) => {
                let index = marks.history.remove(0);
                request.contents.remove(index);
                marks.removed(index);
                if let Some(guide) = marks.history_guide {
                    request.contents[guide] = Content::new("system").with_text(OMITTED_HISTORY);
                }
            }
            Some(ContextTrim::LowestMemory) => {
                let data = evidence.as_mut().ok_or_else(error)?;
                if let Some(sources) = data["sources"]
                    .as_array_mut()
                    .filter(|sources| !sources.is_empty())
                {
                    sources.pop();
                } else {
                    data["memories"].as_array_mut().ok_or_else(error)?.pop();
                }
                if !data["notice"]
                    .as_str()
                    .is_some_and(|notice| notice.contains(NOTICE))
                {
                    data["notice"] = data["notice"]
                        .as_str()
                        .map_or_else(|| NOTICE.to_owned(), |old| format!("{old} {NOTICE}"))
                        .into();
                }
                let index = marks.memory.ok_or_else(error)?;
                request.contents[index].parts[1] = Part::Text {
                    text: serde_json::to_string(data).map_err(|_| error())?,
                };
            }
            None => {
                if let Some(index) = marks.memory.take() {
                    request.contents[index] = Content::new("system").with_text(OMITTED_MEMORY);
                    evidence = None;
                } else if let Some(index) = marks.history_guide.take() {
                    // The short notice survives even when all earlier messages were omitted.
                    if request.contents[index].parts
                        == vec![Part::Text {
                            text: OMITTED_HISTORY.into(),
                        }]
                    {
                        return Err(error());
                    }
                    request.contents[index] = Content::new("system").with_text(OMITTED_HISTORY);
                } else {
                    return Err(error());
                }
            }
        }
    }
}

/// Text uses UTF-8 byte units, including tool schemas and live tool results.
/// Media reservations are provider-independent estimates, not tokenizer claims.
fn media_units(mime: &str) -> u64 {
    sovra_core::media_input_units(mime, None)
}

pub(crate) fn media_annotations(duration_millis: Option<u64>) -> Option<serde_json::Value> {
    duration_millis.map(|duration| serde_json::json!({MEDIA_DURATION: duration}))
}

fn inline_units(mime: &str, annotations: Option<&serde_json::Value>) -> adk_core::Result<u64> {
    let duration = annotations
        .and_then(|value| value.get(MEDIA_DURATION))
        .map(|value| {
            value
                .as_u64()
                .filter(|duration| *duration > 0)
                .ok_or_else(error)
        })
        .transpose()?;
    Ok(sovra_core::media_input_units(mime, duration))
}

fn strip_media_hints(request: &mut LlmRequest) {
    fn strip(annotations: &mut Option<serde_json::Value>) {
        if let Some(values) = annotations
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
        {
            values.remove(MEDIA_DURATION);
            if values.is_empty() {
                *annotations = None;
            }
        }
    }
    for content in &mut request.contents {
        for part in &mut content.parts {
            match part {
                Part::InlineData { annotations, .. } | Part::FileData { annotations, .. } => {
                    strip(annotations);
                }
                Part::FunctionResponse {
                    function_response, ..
                } => {
                    for data in &mut function_response.inline_data {
                        strip(&mut data.annotations);
                    }
                    for data in &mut function_response.file_data {
                        strip(&mut data.annotations);
                    }
                }
                _ => {}
            }
        }
    }
}

fn units(request: &LlmRequest) -> adk_core::Result<u64> {
    let mut total =
        serialized_units(&request.config)?.saturating_add(serialized_units(&request.tools)?);
    total = total.saturating_add(request.model.len() as u64);
    for content in &request.contents {
        total = total
            .saturating_add(content.role.len() as u64)
            .saturating_add(32);
        for part in &content.parts {
            let cost = match part {
                Part::Text { text } => text.len() as u64,
                Part::InlineData {
                    mime_type,
                    annotations,
                    ..
                } => inline_units(mime_type, annotations.as_ref())?,
                Part::FileData { mime_type, .. } => media_units(mime_type),
                Part::FunctionResponse {
                    function_response,
                    id,
                    annotations,
                } => {
                    let mut cost = serialized_units(&(
                        &function_response.name,
                        &function_response.response,
                        id,
                        annotations,
                    ))?;
                    for data in &function_response.inline_data {
                        cost = cost.saturating_add(inline_units(
                            &data.mime_type,
                            data.annotations.as_ref(),
                        )?);
                    }
                    for data in &function_response.file_data {
                        cost = cost.saturating_add(media_units(&data.mime_type));
                    }
                    cost
                }
                _ => serialized_units(part)?,
            };
            total = total.saturating_add(cost).saturating_add(16);
        }
    }
    Ok(total)
}

fn serialized_units(value: &impl Serialize) -> adk_core::Result<u64> {
    struct Counter(u64);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len() as u64);
            if self.0 > 1_048_576 {
                return Err(std::io::Error::other("request exceeds admission cap"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(|_| error())?;
    Ok(counter.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{FunctionResponseData, LlmResponse};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn budget(window: u64) -> ContextBudget {
        ContextBudget {
            context_tokens: Some(window),
            max_output_tokens: Some(1024),
        }
    }

    #[test]
    fn original_sources_yield_to_the_current_question_under_a_small_budget() {
        let mut request = LlmRequest::new(
            "test",
            vec![Content::new("user").with_text("CURRENT QUESTION")],
        );
        mark_memory(&mut request, 0).unwrap();
        request.contents.insert(0, Content::new("system").with_text("Source guidance").with_text(
            json!({"memories":[], "sources":["FIRST".repeat(600), "SECOND".repeat(600), "LAST".repeat(600)], "notice":null}).to_string()));
        prepare(&mut request, budget(4096)).unwrap();
        assert!(units(&request).unwrap() <= budget(4096).allowance(None).unwrap().input_units);
        let text = serde_json::to_string(&request).unwrap();
        assert!(text.contains("CURRENT QUESTION"));
        assert!(!text.contains("LASTLAST"));
    }

    #[test]
    fn optional_context_yields_without_changing_current_input_or_protocol_extensions() {
        let mut request = LlmRequest::new(
            "test",
            vec![
                Content::new("system").with_text("Required instructions"),
                Content::new("user").with_text("CURRENT QUESTION sovra_optional_context"),
            ],
        );
        mark_history(&mut request, 1, 4).unwrap();
        request.contents.splice(
            1..1,
            [
                Content::new("system").with_text("Earlier conversation"),
                Content::new("user").with_text("oldest".repeat(140)),
                Content::new("model").with_text("older".repeat(160)),
                Content::new("user").with_text("recent".repeat(140)),
                Content::new("model").with_text("newest".repeat(140)),
            ],
        );
        mark_memory(&mut request, 0).unwrap();
        request.contents.insert(0, Content::new("system").with_text("Saved evidence guidance").with_text(json!({"memories": ["TOP".repeat(180), "SECOND".repeat(90), "LAST".repeat(140)], "notice":"Keyword fallback was used."}).to_string()));
        request
            .config
            .as_mut()
            .unwrap()
            .extensions
            .insert("provider_option".into(), json!({"value":1}));
        request.previous_response_id = Some("unmeasured history".into());
        prepare(&mut request, budget(4096)).unwrap();
        assert!(units(&request).unwrap() <= budget(4096).allowance(None).unwrap().input_units);
        let text = serde_json::to_string(&request).unwrap();
        assert!(text.contains("CURRENT QUESTION sovra_optional_context"));
        assert!(text.contains("Required instructions"));
        assert!(text.contains("Keyword fallback"));
        assert!(text.contains("TOPTOP"));
        assert!(!text.contains("LASTLAST"));
        assert!(!text.contains("oldestoldest"));
        assert!(text.contains("shortened to fit"));
        assert!(text.contains("omitted to fit"));
        assert!(request.previous_response_id.is_none());
        let config = request.config.unwrap();
        assert!(!config.extensions.contains_key(MARKS));
        assert_eq!(config.extensions["provider_option"], json!({"value":1}));
        assert_eq!(config.max_output_tokens, Some(1024));
    }

    #[test]
    fn tool_schemas_live_results_and_utf8_bytes_all_count_toward_admission() {
        let mut request = LlmRequest::new(
            "test",
            vec![Content::new("user").with_text("Short question")],
        );
        request.tools.insert(
            "big_tool".into(),
            json!({"description": "schema".repeat(400)}),
        );
        assert!(
            prepare(&mut request, budget(4096)).is_err(),
            "LlmRequest serialization skips tools, but admission must count them"
        );
        request.tools.clear();
        request.contents.push(Content {
            role: "tool".into(),
            parts: vec![Part::FunctionResponse {
                function_response: FunctionResponseData::new(
                    "read",
                    json!({"text": "result".repeat(500)}),
                ),
                id: Some("live".into()),
                annotations: None,
            }],
        });
        assert!(prepare(&mut request, budget(4096)).is_err());
        assert!(
            serde_json::to_string(&request.contents)
                .unwrap()
                .contains("resultresult"),
            "live tool results must not be cut into invalid exchanges"
        );
        let text = "茶".repeat(1000);
        let mut request =
            LlmRequest::new("test", vec![Content::new("user").with_text(text.clone())]);
        assert!(prepare(&mut request, budget(4096)).is_err());
        assert_eq!(
            request.contents[0].parts,
            Content::new("user").with_text(text).parts
        );
    }

    #[test]
    fn actual_media_is_retained_and_reserved_without_counting_binary_as_text() {
        let mut request = LlmRequest::new(
            "test",
            vec![
                Content::new("user")
                    .with_text("Describe this photo")
                    .with_inline_data("image/png", vec![0; 1024 * 1024]),
            ],
        );
        prepare(&mut request, budget(32_768)).unwrap();
        assert!(units(&request).unwrap() >= 8192);
        assert!(
            matches!(&request.contents[0].parts[1], Part::InlineData { data, .. } if data.len() == 1024 * 1024)
        );
        assert_eq!(media_units("audio/wav"), 32_768);
        assert_eq!(media_units("video/mp4"), 65_536);
        assert_eq!(media_units("application/unknown"), 1_048_576);
    }

    #[test]
    fn invalid_internal_marks_or_unmeasured_cached_context_fail_closed() {
        let mut request = LlmRequest::new("test", vec![Content::new("user").with_text("question")]);
        mark_history(&mut request, 100, 2).unwrap();
        assert!(prepare(&mut request, budget(8192)).is_err());
        request.config.as_mut().unwrap().cached_content = Some("unknown cache".into());
        assert!(prepare(&mut request, budget(8192)).is_err());
        request.config.as_mut().unwrap().cached_content = None;
        request.config.as_mut().unwrap().max_output_tokens = Some(-1);
        assert!(prepare(&mut request, budget(8192)).is_err());
    }

    #[test]
    fn admission_covers_recorded_selected_model_text_and_image_usage() {
        // Provider-reported counts from synthetic inputs, not estimated tokenizer truth.
        // The current media estimator reads MIME only, so image payload bytes do not
        // affect this check. A future dimensional estimator needs the PNG fixtures too.
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/context-usage.json"))
                .unwrap();
        assert_eq!(report["status"], "observations_complete");
        let observations = report["observations"].as_array().unwrap();
        assert_eq!(observations.len(), 9);
        for observation in observations {
            let mut content = Content::new("user").with_text(observation["text"].as_str().unwrap());
            for _ in observation["images"].as_array().unwrap() {
                content = content.with_inline_data("image/png", Vec::new());
            }
            let request = LlmRequest::new(report["model"].as_str().unwrap(), vec![content]);
            let estimate = units(&request).unwrap();
            let measured = observation["prompt_tokens"].as_u64().unwrap();
            assert!(
                estimate >= measured,
                "{}: admission {estimate} below recorded usage {measured}",
                observation["case"]
            );
        }
    }

    #[test]
    fn a_known_long_recording_cannot_use_the_short_clip_reservation() {
        let mut request = LlmRequest::new(
            "video-model",
            vec![Content::new("user").with_inline_data("video/mp4", vec![])],
        );
        if let Part::InlineData { annotations, .. } = &mut request.contents[0].parts[0] {
            *annotations = Some(json!({"sovra_media_duration_millis": 600_000}));
        }
        assert!(prepare(&mut request, budget(131_072)).is_err());
    }

    #[test]
    fn missing_call_ids_are_distinct_and_existing_provider_ids_are_preserved() {
        let call = |id| Part::FunctionCall {
            name: "search_saved_details".into(),
            args: json!({}),
            id,
            thought_signature: None,
        };
        let mut content = Content {
            role: "model".into(),
            parts: vec![
                call(None),
                call(Some(String::new())),
                call(Some("provider-kept".into())),
            ],
        };
        let mut used = std::collections::HashSet::from(["sovra_call_0".into()]);
        complete_call_ids(&mut content, &mut used);
        let ids = content
            .parts
            .iter()
            .map(|part| match part {
                Part::FunctionCall { id, .. } => id.as_ref().unwrap(),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        assert_ne!(ids[0], ids[1]);
        assert_ne!(ids[0], "sovra_call_0");
        assert_ne!(ids[1], "sovra_call_0");
        assert_eq!(ids[2], "provider-kept");
    }

    struct Model(AtomicUsize);
    #[async_trait::async_trait]
    impl Llm for Model {
        fn name(&self) -> &'static str {
            "admission-test"
        }
        async fn generate_content(
            &self,
            request: LlmRequest,
            _: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            self.0.fetch_add(1, Ordering::SeqCst);
            assert!(
                !serde_json::to_string(&request)
                    .unwrap()
                    .contains(MEDIA_DURATION)
            );
            assert!(
                !request
                    .config
                    .as_ref()
                    .unwrap()
                    .extensions
                    .contains_key(MARKS)
            );
            let mut response = LlmResponse::new(Content::new("model").with_text("ok"));
            response.interaction_id = Some("hidden-server-context".into());
            Ok(Box::pin(futures::stream::once(async { Ok(response) })))
        }
    }
    #[tokio::test]
    async fn duration_admission_precedes_provider_io_and_strips_private_hints() {
        let model = Arc::new(Model(AtomicUsize::new(0)));
        let mut request = LlmRequest::new(
            "test",
            vec![Content::new("user").with_inline_data("video/mp4", vec![1, 2, 3])],
        );
        if let Part::InlineData { annotations, .. } = &mut request.contents[0].parts[0] {
            *annotations = media_annotations(Some(600_000));
        }
        assert!(
            wrap(model.clone(), budget(131_072))
                .generate_content(request.clone(), false)
                .await
                .is_err()
        );
        assert_eq!(model.0.load(Ordering::SeqCst), 0);
        assert!(
            wrap(model.clone(), budget(1_048_576))
                .generate_content(request, false)
                .await
                .is_ok()
        );
        assert_eq!(model.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn tool_media_hints_are_counted_validated_and_removed_without_losing_annotations() {
        let mut response = FunctionResponseData::new("media", json!({}));
        response.inline_data.push(adk_core::InlineDataPart {
            mime_type: "audio/wav".into(),
            data: vec![1, 2],
            uri: None,
            annotations: Some(json!({MEDIA_DURATION:1_200_000, "title":"recording"})),
        });
        let mut content = Content::new("user");
        content.parts.push(Part::FunctionResponse {
            function_response: response,
            id: None,
            annotations: None,
        });
        let mut request = LlmRequest::new("test", vec![content]);
        assert!(prepare(&mut request.clone(), budget(65_536)).is_err());
        prepare(&mut request, budget(131_072)).unwrap();
        strip_media_hints(&mut request);
        let Part::FunctionResponse {
            function_response, ..
        } = &request.contents[0].parts[0]
        else {
            unreachable!()
        };
        assert_eq!(
            function_response.inline_data[0].annotations,
            Some(json!({"title":"recording"}))
        );
        for duration in [json!(0), json!(-1), json!("600000")] {
            assert!(inline_units("video/mp4", Some(&json!({MEDIA_DURATION:duration}))).is_err());
        }
        assert!(
            inline_units("video/mp4", Some(&json!({MEDIA_DURATION:u64::MAX}))).unwrap()
                >= 1_048_576
        );
    }

    #[tokio::test]
    async fn rejected_requests_do_not_call_the_provider_or_leave_a_continuation() {
        let model = Arc::new(Model(AtomicUsize::new(0)));
        let guarded = wrap(model.clone(), budget(8192));
        assert!(
            guarded
                .generate_content(
                    LlmRequest::new(
                        "test",
                        vec![Content::new("user").with_text("x".repeat(8192))]
                    ),
                    false
                )
                .await
                .is_err()
        );
        assert_eq!(model.0.load(Ordering::SeqCst), 0);
        let mut output = guarded
            .generate_content(
                LlmRequest::new("test", vec![Content::new("user").with_text("Hi")]),
                false,
            )
            .await
            .unwrap();
        assert!(
            output
                .next()
                .await
                .unwrap()
                .unwrap()
                .interaction_id
                .is_none()
        );
        assert_eq!(model.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn remembering_surfaces_the_same_context_limit_without_sending_evidence() {
        let model = Arc::new(Model(AtomicUsize::new(0)));
        let harness = crate::AdkHarness::with_llm_and_tools(
            &sovra_core::AgentConfig::default(),
            wrap(model.clone(), budget(8192)),
            vec![],
        )
        .unwrap();
        let source = sovra_core::Episode {
            id: sovra_core::EpisodeId("episode:large".into()),
            session_id: "test".into(),
            kind: sovra_core::EpisodeKind::UserMessage,
            content: "x".repeat(8192),
            occurred_at: sovra_core::Timestamp(1),
        };
        let failure = harness.extract_memories(&source, &[]).await.unwrap_err();
        assert!(failure.to_string().contains("input budget"));
        assert_eq!(model.0.load(Ordering::SeqCst), 0);
    }
}
