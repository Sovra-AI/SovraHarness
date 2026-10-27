//! Pure configuration types for the agent engine, populated in `sovra-app` and
//! passed down through `sovra-runtime` to `sovra-adk`.

/// Default Google Gemini model identifier for ADK-Rust agents.
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-2.5-flash";

/// Default system instruction for the Sovra assistant agent.
pub const DEFAULT_SYSTEM_INSTRUCTION: &str = "You are Sovra, a helpful conversational assistant. Respond naturally to what the user says and answer their question directly. Match the level of detail to the request. Use Markdown when it improves readability, with fenced code blocks for code. Be accurate and clear about uncertainty. Ask for clarification only when needed to give a useful answer. Do not assume the user wants an engineering task, an agent workflow, or a setup process. Never claim to have performed an action without a real tool result.";

/// Pure configuration for initializing an ADK-backed agent runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    /// Application identifier registered with the ADK session service.
    pub app_name: String,
    /// Agent identifier within the ADK runner.
    pub agent_name: String,
    /// Gemini model identifier (e.g. `"gemini-2.5-flash"`).
    pub model: String,
    /// System instruction passed to `LlmAgentBuilder`.
    pub instruction: String,
    /// Optional Google / Gemini API key read once at process startup.
    pub api_key: Option<String>,
    /// Optional self-hosted `SearXNG` origin (`https://host`) read once at startup.
    /// Web search works without it; when set, it is one more source beside the
    /// built-in ones. The search client adds `/search`.
    pub searxng_base_url: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            app_name: "sovra-harness".to_owned(),
            agent_name: "sovra-assistant".to_owned(),
            model: DEFAULT_GEMINI_MODEL.to_owned(),
            instruction: DEFAULT_SYSTEM_INSTRUCTION.to_owned(),
            api_key: None,
            searxng_base_url: None,
        }
    }
}

impl AgentConfig {
    /// Creates a default configuration with the given optional Gemini API key.
    pub fn with_api_key(api_key: Option<String>) -> Self {
        let normalized = api_key
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty());
        Self {
            api_key: normalized,
            ..Self::default()
        }
    }

    /// Returns `true` when a non-empty API key is present.
    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_defaults_and_no_key() {
        let cfg = AgentConfig::default();
        assert_eq!(cfg.model, DEFAULT_GEMINI_MODEL);
        assert!(!cfg.has_api_key());
    }

    #[test]
    fn with_api_key_trims_and_ignores_blank_keys() {
        assert!(!AgentConfig::with_api_key(Some("   ".into())).has_api_key());
        let cfg = AgentConfig::with_api_key(Some("  test-key  ".into()));
        assert!(cfg.has_api_key());
        assert_eq!(cfg.api_key.as_deref(), Some("test-key"));
    }
}
