//! Translate Sovra's [`ThinkingLevel`] into each service's own setting.
//!
//! - `OpenRouter`: the request's `reasoning.effort` (`none`…`max`), only for models
//!   whose catalog lists the `reasoning` parameter. Claude's `none` is sent as
//!   `minimal`.
//! - Ollama: `reasoning_effort` (`none`/`low`/`medium`/`high`; `max` becomes `high`),
//!   only for models whose details list thinking. Ollama refuses thinking settings
//!   for models that cannot think.
//! - OpenAI-compatible servers: `reasoning_effort` named `none`, `low`, `medium`,
//!   `high`, or `max`. They advertise no capabilities, so an explicit choice is sent.
//! - Grok and xAI ids, on `OpenRouter` and on compatible servers: follow the current
//!   reasoning guide. `grok-3`, `grok-4`, `grok-4-fast-reasoning`, and a multi-agent
//!   Grok get no parameter (it errors, or it means how many agents). `grok-4.3` can
//!   use `none`. `grok-4.5` and later cannot turn thinking off, so "No thinking" is
//!   `low`. "Think deeply" is `high` until `grok-4.6`, then `xhigh`. Any other id
//!   that contains `grok` stays on `low` / `medium` / `high`.
//! - Anthropic: adaptive thinking with `output_config.effort` on current models, a
//!   token budget on models from before adaptive thinking, only for models listing
//!   thinking. Set when the client is built ([`anthropic_config`]), because the
//!   client holds it. "No thinking" is the lowest effort, not a switch-off.
//!
//! [`ThinkingLevel::ModelDefault`] sends nothing. Structured requests (memory notes,
//! file summaries) carry a response schema and are left alone: they gain nothing
//! from deep thinking and would pay for it.

use std::sync::Arc;

use adk_core::{Llm, LlmRequest, LlmResponseStream};
use adk_model::anthropic::{AnthropicConfig, Effort, ThinkingMode};
use serde_json::{Map, Value, json};
use sovra_core::ai_tiers::ThinkingLevel;
use sovra_core::models::{ModelInfo, ProviderKind};

/// What a Grok id does with one explicit effort name (`none`…`max`).
enum GrokWire {
    /// Not a Grok id; the caller's own rule applies.
    NotGrok,
    /// Leave the parameter off. Sending it errors, or it does not mean depth.
    Omit,
    /// Send this value, which this id accepts.
    Send(&'static str),
}

/// Maps one explicit effort onto the current xAI reasoning guide.
///
/// `grok-4.6` and later means 4.6 through 4.9 and any later major (`grok-5`).
/// A two-digit minor such as `grok-4.20` is a different line: multi-agent ids
/// omit the parameter, and any other such id stays on `low` / `medium` / `high`
/// so `none` and `max` are not sent to a model the guide does not list.
fn grok_wire(model_id: &str, effort: &str) -> GrokWire {
    let id = model_id.to_ascii_lowercase();
    if !id.contains("grok") {
        return GrokWire::NotGrok;
    }
    if id.contains("multi-agent") || id.contains("multi_agent") || id.contains("multiagent") {
        return GrokWire::Omit;
    }
    if id.contains("grok-4-fast-reasoning") || is_plain_grok(&id, "grok-3") || is_plain_grok_4(&id)
    {
        return GrokWire::Omit;
    }
    let (none, max) = match grok_release(&id) {
        Some((4, 3)) => ("none", "high"),
        Some((4, 5)) => ("low", "high"),
        Some((4, minor)) if (6..=9).contains(&minor) => ("low", "xhigh"),
        Some((major, _)) if major > 4 => ("low", "xhigh"),
        _ => ("low", "high"),
    };
    GrokWire::Send(match effort {
        "none" => none,
        "low" => "low",
        "medium" => "medium",
        "high" => "high",
        "max" => max,
        _ => return GrokWire::Omit,
    })
}

/// `grok-3` and its aliases (`grok-3-latest`, `grok-3-fast`), not `grok-3-mini`.
fn is_plain_grok(id: &str, family: &str) -> bool {
    let Some(after) = family_tail(id, family) else {
        return false;
    };
    if after.starts_with("-mini") {
        return false;
    }
    after.is_empty() || after.starts_with('-')
}

/// `grok-4` and dated aliases such as `grok-4-0709`. Not `grok-4.x` or `grok-4-fast`.
fn is_plain_grok_4(id: &str) -> bool {
    let Some(after) = family_tail(id, "grok-4") else {
        return false;
    };
    if after.starts_with('.') || after.starts_with("-fast") || after.starts_with("-1") {
        return false;
    }
    after.is_empty() || after.starts_with('-')
}

fn family_tail<'a>(id: &'a str, family: &str) -> Option<&'a str> {
    let at = id.find(family)?;
    if at > 0 && id.as_bytes()[at - 1].is_ascii_alphanumeric() {
        return None;
    }
    Some(&id[at + family.len()..])
}

/// `(major, minor)` from the first `grok-N` or `grok-N.M` in `id`.
fn grok_release(id: &str) -> Option<(u32, u32)> {
    let rest = family_tail(id, "grok-")?;
    let (major, rest) = take_digits(rest)?;
    let minor = if let Some(rest) = rest.strip_prefix('.') {
        take_digits(rest)?.0
    } else {
        0
    };
    Some((major, minor))
}

fn take_digits(text: &str) -> Option<(u32, &str)> {
    let end = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    if end == 0 {
        return None;
    }
    Some((text[..end].parse().ok()?, &text[end..]))
}

/// The effort string this service should send, or `None` to leave the parameter off.
fn effort_name(
    kind: ProviderKind,
    model: &ModelInfo,
    level: ThinkingLevel,
) -> Option<&'static str> {
    let effort = level.wire_name()?;
    match kind {
        ProviderKind::OpenRouter => {
            if !model.supports("reasoning") {
                return None;
            }
            match grok_wire(&model.id, effort) {
                GrokWire::NotGrok => {
                    // OpenRouter rejects `none` for Claude, whose newer models always
                    // think; `minimal` is its lightest accepted level.
                    Some(if effort == "none" && model.id.starts_with("anthropic/") {
                        "minimal"
                    } else {
                        effort
                    })
                }
                GrokWire::Omit => None,
                GrokWire::Send(effort) => Some(effort),
            }
        }
        ProviderKind::Ollama => {
            if !model.supports("reasoning") {
                return None;
            }
            Some(if effort == "max" { "high" } else { effort })
        }
        ProviderKind::OpenAiCompatible => match grok_wire(&model.id, effort) {
            GrokWire::NotGrok => Some(effort),
            GrokWire::Omit => None,
            GrokWire::Send(effort) => Some(effort),
        },
        // Configured on the client instead.
        ProviderKind::Anthropic => None,
    }
}

/// Request-level fields one service needs for one level, or `None` to send nothing.
fn request_fields(
    kind: ProviderKind,
    model: &ModelInfo,
    level: ThinkingLevel,
) -> Option<(&'static str, Map<String, Value>)> {
    let effort = effort_name(kind, model, level)?;
    let mut fields = Map::new();
    match kind {
        ProviderKind::OpenRouter => {
            fields.insert("reasoning".into(), json!({ "effort": effort }));
            Some((
                adk_model::openrouter::OPENROUTER_EXTENSION_NAMESPACE,
                fields,
            ))
        }
        ProviderKind::Ollama | ProviderKind::OpenAiCompatible => {
            fields.insert("reasoning_effort".into(), json!(effort));
            Some(("openai", fields))
        }
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

    fn reasoning(kind: ProviderKind, id: &str, level: ThinkingLevel) -> Option<Value> {
        sent(kind, &model(id, &["reasoning"]), level)
    }

    fn openrouter(id: &str, level: ThinkingLevel) -> Option<Value> {
        reasoning(ProviderKind::OpenRouter, id, level)
    }

    fn compatible(id: &str, level: ThinkingLevel) -> Option<Value> {
        reasoning(ProviderKind::OpenAiCompatible, id, level)
    }

    fn effort(value: &str) -> Value {
        json!({ "reasoning": { "effort": value } })
    }

    fn body(value: &str) -> Value {
        json!({ "reasoning_effort": value })
    }

    #[test]
    fn thinking_effort_matches_each_providers_current_rules() {
        let reasoning_model = model("openai/gpt-5", &["reasoning"]);
        for kind in [
            ProviderKind::OpenRouter,
            ProviderKind::Ollama,
            ProviderKind::OpenAiCompatible,
            ProviderKind::Anthropic,
        ] {
            assert_eq!(
                sent(kind, &reasoning_model, ThinkingLevel::ModelDefault),
                None,
                "AI decides sends nothing for {kind:?}"
            );
        }

        assert_eq!(
            openrouter("openai/gpt-5", ThinkingLevel::Off),
            Some(effort("none"))
        );
        assert_eq!(
            openrouter("openai/gpt-5", ThinkingLevel::Low),
            Some(effort("low"))
        );
        assert_eq!(
            openrouter("openai/gpt-5", ThinkingLevel::Medium),
            Some(effort("medium"))
        );
        assert_eq!(
            openrouter("openai/gpt-5", ThinkingLevel::High),
            Some(effort("high"))
        );
        assert_eq!(
            openrouter("openai/gpt-5", ThinkingLevel::Max),
            Some(effort("max"))
        );
        assert_eq!(
            sent(
                ProviderKind::OpenRouter,
                &model("meta/llama-3", &["tools"]),
                ThinkingLevel::Max
            ),
            None
        );
        assert_eq!(
            openrouter("anthropic/claude-sonnet-5", ThinkingLevel::Off),
            Some(effort("minimal"))
        );
        assert_eq!(
            openrouter("anthropic/claude-sonnet-5", ThinkingLevel::Max),
            Some(effort("max"))
        );

        let adaptive = model("claude-opus-5-5", &["reasoning"]);
        let base = || AnthropicConfig::new("key", "m");
        let off = anthropic_config(base(), &adaptive, ThinkingLevel::Off);
        assert_eq!((off.thinking, off.effort), (None, Some(Effort::Low)));
        let low = anthropic_config(base(), &adaptive, ThinkingLevel::Low);
        assert_eq!(
            (low.thinking, low.effort),
            (Some(ThinkingMode::Adaptive), Some(Effort::Low))
        );
        let medium = anthropic_config(base(), &adaptive, ThinkingLevel::Medium);
        assert_eq!(
            (medium.thinking, medium.effort),
            (Some(ThinkingMode::Adaptive), Some(Effort::Medium))
        );
        let high = anthropic_config(base(), &adaptive, ThinkingLevel::High);
        assert_eq!(
            (high.thinking, high.effort),
            (Some(ThinkingMode::Adaptive), Some(Effort::High))
        );
        let deep = anthropic_config(base(), &adaptive, ThinkingLevel::Max);
        assert_eq!(
            (deep.thinking, deep.effort),
            (Some(ThinkingMode::Adaptive), Some(Effort::Max))
        );
        let untouched = anthropic_config(base(), &adaptive, ThinkingLevel::ModelDefault);
        assert_eq!((untouched.thinking, untouched.effort), (None, None));
        let legacy = ModelInfo {
            max_output_tokens: Some(64_000),
            ..model("claude-3-7-sonnet", &["reasoning"])
        };
        assert_eq!(
            anthropic_config(base(), &legacy, ThinkingLevel::ModelDefault).thinking,
            None
        );
        assert_eq!(
            anthropic_config(base(), &legacy, ThinkingLevel::Off).thinking,
            None
        );
        assert_eq!(
            anthropic_config(base(), &legacy, ThinkingLevel::Low).thinking,
            Some(ThinkingMode::Enabled {
                budget_tokens: 2_048
            })
        );

        assert_eq!(
            reasoning(ProviderKind::Ollama, "qwen3:8b", ThinkingLevel::Off),
            Some(body("none"))
        );
        assert_eq!(
            reasoning(ProviderKind::Ollama, "qwen3:8b", ThinkingLevel::Max),
            Some(body("high"))
        );
        assert_eq!(
            reasoning(ProviderKind::Ollama, "qwen3:8b", ThinkingLevel::Medium),
            Some(body("medium"))
        );
        assert_eq!(
            sent(
                ProviderKind::Ollama,
                &model("qwen2.5:7b", &["tools"]),
                ThinkingLevel::High
            ),
            None
        );
        // Ollama keeps its own names even when the model id contains grok.
        assert_eq!(
            reasoning(ProviderKind::Ollama, "grok-4.6", ThinkingLevel::Off),
            Some(body("none"))
        );
        assert_eq!(
            reasoning(ProviderKind::Ollama, "grok-4.6", ThinkingLevel::Max),
            Some(body("high"))
        );

        for (level, name) in [
            (ThinkingLevel::Off, "none"),
            (ThinkingLevel::Low, "low"),
            (ThinkingLevel::Medium, "medium"),
            (ThinkingLevel::High, "high"),
            (ThinkingLevel::Max, "max"),
        ] {
            assert_eq!(
                compatible("local-model", level),
                Some(body(name)),
                "a compatible server keeps the level name {name}"
            );
        }

        let omitted = [
            "grok-3",
            "grok-3-latest",
            "grok-3-fast",
            "x-ai/grok-4",
            "grok-4-0709",
            "grok-4-latest",
            "grok-4-fast-reasoning",
            "x-ai/grok-4-fast-reasoning",
            "grok-4.20-multi-agent",
            "x-ai/grok-4.20-multi-agent",
            "grok-4.20-multi_agent",
        ];
        for id in omitted {
            for kind in [ProviderKind::OpenRouter, ProviderKind::OpenAiCompatible] {
                for level in ThinkingLevel::ALL {
                    assert_eq!(
                        reasoning(kind, id, level),
                        None,
                        "{kind:?} {id} {level:?} must not send a thinking parameter"
                    );
                }
            }
        }

        let cases = [
            ("grok-4.3", ThinkingLevel::Off, "none"),
            ("grok-4.3", ThinkingLevel::Low, "low"),
            ("grok-4.3", ThinkingLevel::Medium, "medium"),
            ("grok-4.3", ThinkingLevel::High, "high"),
            ("grok-4.3-latest", ThinkingLevel::Max, "high"),
            ("grok-4.5", ThinkingLevel::Off, "low"),
            ("grok-4.5", ThinkingLevel::Medium, "medium"),
            ("x-ai/grok-4.5", ThinkingLevel::Max, "high"),
            ("grok-4.6", ThinkingLevel::Off, "low"),
            ("grok-4.6", ThinkingLevel::Low, "low"),
            ("grok-4.6", ThinkingLevel::Medium, "medium"),
            ("grok-4.6", ThinkingLevel::High, "high"),
            ("grok-4.6", ThinkingLevel::Max, "xhigh"),
            ("x-ai/grok-4.7", ThinkingLevel::Off, "low"),
            ("grok-4.7", ThinkingLevel::Max, "xhigh"),
            ("grok-5", ThinkingLevel::Off, "low"),
            ("grok-5", ThinkingLevel::Max, "xhigh"),
            ("grok-3-mini", ThinkingLevel::Off, "low"),
            ("grok-3-mini", ThinkingLevel::Medium, "medium"),
            ("grok-3-mini", ThinkingLevel::Max, "high"),
            ("grok-4-fast-non-reasoning", ThinkingLevel::Off, "low"),
            ("grok-4-fast-non-reasoning", ThinkingLevel::Max, "high"),
            ("grok-4.20", ThinkingLevel::Off, "low"),
            ("grok-4.20", ThinkingLevel::Max, "high"),
        ];
        for (id, level, name) in cases {
            assert_eq!(
                openrouter(id, level),
                Some(effort(name)),
                "OpenRouter {id} {level:?}"
            );
            assert_eq!(
                compatible(id, level),
                Some(body(name)),
                "compatible {id} {level:?}"
            );
            assert_eq!(
                openrouter(id, ThinkingLevel::ModelDefault),
                None,
                "AI decides stays empty for {id}"
            );
        }
        assert_eq!(
            sent(
                ProviderKind::OpenRouter,
                &model("x-ai/grok-4.6", &["tools"]),
                ThinkingLevel::Max
            ),
            None,
            "OpenRouter still requires the reasoning capability"
        );

        let (namespace, fields) = request_fields(
            ProviderKind::OpenAiCompatible,
            &model("grok-4.6", &[]),
            ThinkingLevel::Max,
        )
        .expect("an explicit grok level has a field");
        let mut structured = LlmRequest::new("m", Vec::new());
        structured.config.get_or_insert_default().response_schema =
            Some(json!({ "type": "object" }));
        apply(&mut structured, namespace, &fields);
        assert!(
            structured.config.expect("config").extensions.is_empty(),
            "a structured request keeps no thinking setting"
        );
    }
}
