//! Translate Sovra's [`ThinkingLevel`] into each service's own setting.
//!
//! - `OpenRouter`: the request's `reasoning.effort` (`none`…`max`), only for models
//!   whose catalog lists the `reasoning` parameter.
//! - Ollama: `reasoning_effort` (`none`/`low`/`medium`/`high`; `max` becomes `high`),
//!   only for models whose details list thinking. Ollama refuses thinking settings
//!   for models that cannot think.
//! - OpenAI-compatible servers: `reasoning_effort`; they advertise no capabilities,
//!   so the person's explicit choice is sent as is.
//! - Anthropic: adaptive thinking with `output_config.effort` on current models, a
//!   token budget on models from before adaptive thinking, only for models listing
//!   thinking. Set when the client is built ([`anthropic_config`]), because the
//!   client holds it.
//!
//! Structured requests (memory notes, file summaries) carry a response schema and
//! are left alone: they gain nothing from deep thinking and would pay for it.

use std::sync::Arc;

use adk_core::{Llm, LlmRequest, LlmResponseStream};
use adk_model::anthropic::{AnthropicConfig, Effort, ThinkingMode};
use serde_json::{Map, Value, json};
use sovra_core::ai_tiers::ThinkingLevel;
use sovra_core::models::{ModelInfo, ProviderKind};

/// Request-level fields one service needs for one level, or `None` to send nothing.
fn request_fields(
    kind: ProviderKind,
    model: &ModelInfo,
    level: ThinkingLevel,
) -> Option<(&'static str, Map<String, Value>)> {
    let effort = level.wire_name()?;
    match kind {
        ProviderKind::OpenRouter => {
            if !model.supports("reasoning") {
                return None;
            }
            // OpenRouter rejects `none` for Claude, whose newer models always
            // think; `minimal` is its lightest accepted level.
            let effort = if effort == "none" && model.id.starts_with("anthropic/") {
                "minimal"
            } else {
                effort
            };
            let mut fields = Map::new();
            fields.insert("reasoning".into(), json!({ "effort": effort }));
            Some((
                adk_model::openrouter::OPENROUTER_EXTENSION_NAMESPACE,
                fields,
            ))
        }
        ProviderKind::Ollama => {
            if !model.supports("reasoning") {
                return None;
            }
            let effort = if effort == "max" { "high" } else { effort };
            let mut fields = Map::new();
            fields.insert("reasoning_effort".into(), json!(effort));
            Some(("openai", fields))
        }
        ProviderKind::OpenAiCompatible => {
            let mut fields = Map::new();
            fields.insert("reasoning_effort".into(), json!(effort));
            Some(("openai", fields))
        }
        // Configured on the client instead.
        ProviderKind::Anthropic => None,
    }
}

struct ThinkingModel {
    inner: Arc<dyn Llm>,
    namespace: &'static str,
    fields: Map<String, Value>,
}

/// Adds the level's request fields to every chat request this model sends.
pub(crate) fn wrap(
    inner: Arc<dyn Llm>,
    kind: ProviderKind,
    model: &ModelInfo,
    level: ThinkingLevel,
) -> Arc<dyn Llm> {
    match request_fields(kind, model, level) {
        Some((namespace, fields)) => Arc::new(ThinkingModel {
            inner,
            namespace,
            fields,
        }),
        None => inner,
    }
}

fn apply(request: &mut LlmRequest, namespace: &str, fields: &Map<String, Value>) {
    let config = request.config.get_or_insert_default();
    if config.response_schema.is_some() {
        return;
    }
    let entry = config
        .extensions
        .entry(namespace)
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(existing) = entry.as_object_mut() {
        for (key, value) in fields {
            // A caller that already chose (for example, reasoning replay) wins.
            existing.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}

#[async_trait::async_trait]
impl Llm for ThinkingModel {
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
        apply(&mut request, self.namespace, &self.fields);
        self.inner.generate_content(request, stream).await
    }
}

/// Claude models released before adaptive thinking take a token budget instead.
fn needs_budget(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();
    id.contains("claude-3")
        || ["-4-0", "-4-1", "-4-5", "opus-4-2025", "sonnet-4-2025"]
            .iter()
            .any(|marker| id.contains(marker))
}

/// Thinking budget for a budget-only Claude model; always below `max_tokens`.
fn budget(level: ThinkingLevel, max_output: Option<u64>) -> Option<u32> {
    let wanted: u32 = match level {
        ThinkingLevel::ModelDefault | ThinkingLevel::Off => return None,
        ThinkingLevel::Low => 2_048,
        ThinkingLevel::Medium => 8_192,
        ThinkingLevel::High => 16_000,
        ThinkingLevel::Max => 32_000,
    };
    // Leave room for the visible answer inside the output limit.
    let ceiling = max_output
        .and_then(|limit| u32::try_from(limit).ok())
        .map_or(wanted, |limit| limit.saturating_sub(4_096));
    let budget = wanted.min(ceiling);
    (budget >= 1_024).then_some(budget)
}

/// Applies a level to an Anthropic client configuration.
pub(crate) fn anthropic_config(
    mut config: AnthropicConfig,
    model: &ModelInfo,
    level: ThinkingLevel,
) -> AnthropicConfig {
    if !model.supports("reasoning") {
        return config;
    }
    if needs_budget(&model.id) {
        if let Some(budget) = budget(level, model.max_output_tokens) {
            config = config
                .with_thinking_mode(ThinkingMode::Enabled {
                    budget_tokens: budget,
                })
                .with_max_tokens(budget.saturating_add(4_096));
        }
        return config;
    }
    let effort = match level {
        ThinkingLevel::ModelDefault => return config,
        // Newer Claude models cannot switch thinking off; the lowest effort is
        // the closest to "answer straight away".
        ThinkingLevel::Off | ThinkingLevel::Low => Effort::Low,
        ThinkingLevel::Medium => Effort::Medium,
        ThinkingLevel::High => Effort::High,
        ThinkingLevel::Max => Effort::Max,
    };
    if level == ThinkingLevel::Off {
        return config.with_effort(effort);
    }
    config
        .with_thinking_mode(ThinkingMode::Adaptive)
        .with_effort(effort)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, params: &[&str]) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            supported_parameters: params.iter().map(|p| (*p).to_owned()).collect(),
            ..ModelInfo::default()
        }
    }

    fn sent(kind: ProviderKind, model: &ModelInfo, level: ThinkingLevel) -> Option<Value> {
        let (namespace, fields) = request_fields(kind, model, level)?;
        let mut request = LlmRequest::new("m", Vec::new());
        apply(&mut request, namespace, &fields);
        request.config?.extensions.get(namespace).cloned()
    }

    #[test]
    fn model_default_sends_nothing_anywhere() {
        let reasoning = model("openai/gpt-5", &["reasoning"]);
        for kind in [
            ProviderKind::OpenRouter,
            ProviderKind::Ollama,
            ProviderKind::OpenAiCompatible,
            ProviderKind::Anthropic,
        ] {
            assert_eq!(sent(kind, &reasoning, ThinkingLevel::ModelDefault), None);
        }
    }

    #[test]
    fn openrouter_uses_reasoning_effort_only_for_reasoning_models() {
        let reasoning = model("openai/gpt-5", &["reasoning", "tools"]);
        assert_eq!(
            sent(ProviderKind::OpenRouter, &reasoning, ThinkingLevel::High),
            Some(json!({ "reasoning": { "effort": "high" } }))
        );
        assert_eq!(
            sent(ProviderKind::OpenRouter, &reasoning, ThinkingLevel::Off),
            Some(json!({ "reasoning": { "effort": "none" } }))
        );
        let plain = model("meta/llama-3", &["tools"]);
        assert_eq!(
            sent(ProviderKind::OpenRouter, &plain, ThinkingLevel::Max),
            None
        );
        let claude = model("anthropic/claude-sonnet-5", &["reasoning"]);
        assert_eq!(
            sent(ProviderKind::OpenRouter, &claude, ThinkingLevel::Off),
            Some(json!({ "reasoning": { "effort": "minimal" } }))
        );
    }

    #[test]
    fn local_servers_get_reasoning_effort_in_the_body() {
        let any = model("qwen3:8b", &["reasoning"]);
        assert_eq!(
            sent(ProviderKind::Ollama, &any, ThinkingLevel::Max),
            Some(json!({ "reasoning_effort": "high" }))
        );
        assert_eq!(
            sent(ProviderKind::OpenAiCompatible, &any, ThinkingLevel::Max),
            Some(json!({ "reasoning_effort": "max" }))
        );
        assert_eq!(
            sent(ProviderKind::Ollama, &any, ThinkingLevel::Off),
            Some(json!({ "reasoning_effort": "none" }))
        );
    }

    #[test]
    fn existing_choices_and_structured_requests_are_respected() {
        let (namespace, fields) = request_fields(
            ProviderKind::OpenAiCompatible,
            &model("m", &[]),
            ThinkingLevel::High,
        )
        .expect("fields");
        let mut replay = LlmRequest::new("m", Vec::new());
        replay.config.get_or_insert_default().extensions.insert(
            "openai".into(),
            json!({ "reasoning_effort": "low", "max_tokens": 10 }),
        );
        apply(&mut replay, namespace, &fields);
        assert_eq!(
            replay.config.expect("config").extensions["openai"],
            json!({ "reasoning_effort": "low", "max_tokens": 10 })
        );

        let mut structured = LlmRequest::new("m", Vec::new());
        structured.config.get_or_insert_default().response_schema =
            Some(json!({ "type": "object" }));
        apply(&mut structured, namespace, &fields);
        assert!(structured.config.expect("config").extensions.is_empty());
    }

    #[test]
    fn anthropic_uses_adaptive_effort_on_new_models_and_budgets_on_old_ones() {
        let base = || AnthropicConfig::new("key", "m");
        let new = model("claude-opus-5-5", &["reasoning"]);
        let deep = anthropic_config(base(), &new, ThinkingLevel::High);
        assert_eq!(deep.thinking, Some(ThinkingMode::Adaptive));
        assert_eq!(deep.effort, Some(Effort::High));
        let off = anthropic_config(base(), &new, ThinkingLevel::Off);
        assert_eq!((off.thinking, off.effort), (None, Some(Effort::Low)));
        let default = anthropic_config(base(), &new, ThinkingLevel::ModelDefault);
        assert_eq!((default.thinking, default.effort), (None, None));

        let old = ModelInfo {
            max_output_tokens: Some(64_000),
            ..model("claude-sonnet-4-5-20250929", &["reasoning"])
        };
        let budgeted = anthropic_config(base(), &old, ThinkingLevel::Max);
        assert_eq!(
            budgeted.thinking,
            Some(ThinkingMode::Enabled {
                budget_tokens: 32_000
            })
        );
        assert!(budgeted.max_tokens > 32_000);
        assert_eq!(budgeted.effort, None);
        let off = anthropic_config(base(), &old, ThinkingLevel::Off);
        assert_eq!(off.thinking, None);
    }

    #[test]
    fn models_that_cannot_think_get_nothing_from_ollama_or_anthropic() {
        let plain = model("qwen2.5-coder:7b", &["tools"]);
        assert_eq!(
            sent(ProviderKind::Ollama, &plain, ThinkingLevel::High),
            None
        );
        assert_eq!(sent(ProviderKind::Ollama, &plain, ThinkingLevel::Off), None);
        let haiku = model("claude-3-haiku-20240307", &[]);
        let config = anthropic_config(AnthropicConfig::new("key", "m"), &haiku, ThinkingLevel::Max);
        assert_eq!((config.thinking, config.effort), (None, None));
        // Compatible servers advertise nothing, so an explicit choice is still sent.
        assert_eq!(
            sent(ProviderKind::OpenAiCompatible, &plain, ThinkingLevel::Low),
            Some(json!({ "reasoning_effort": "low" }))
        );
    }

    #[test]
    fn budgets_stay_inside_small_output_limits() {
        assert_eq!(budget(ThinkingLevel::Max, Some(8_192)), Some(4_096));
        assert_eq!(budget(ThinkingLevel::Low, Some(4_096)), None);
        assert_eq!(budget(ThinkingLevel::High, None), Some(16_000));
    }
}
