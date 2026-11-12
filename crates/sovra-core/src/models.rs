//! Model catalog, connection configuration and filtering. Network and credential IO
//! belong to provider adapters; this module is deterministic and backend independent.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

mod destination;
mod language;
pub use destination::{ModelDestination, ModelWork, model_destinations};
pub use language::{
    ChromeKey, FirstRunKey, LanguageInfo, UiLanguage, chrome, first_run_text, world_languages,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ProviderKind {
    OpenRouter,
    Ollama,
    OpenAiCompatible,
    Anthropic,
}

impl ProviderKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenRouter => "OpenRouter",
            Self::Ollama => "Ollama",
            Self::OpenAiCompatible => "OpenAI compatible",
            Self::Anthropic => "Anthropic Messages",
        }
    }

    pub fn default_url(self) -> &'static str {
        match self {
            Self::OpenRouter => "https://openrouter.ai/api/v1",
            Self::Ollama => "http://localhost:11434",
            Self::OpenAiCompatible => "http://localhost:1234/v1",
            Self::Anthropic => "https://api.anthropic.com",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Authentication {
    None,
    Bearer,
    ApiKey,
}

/// Public connection metadata. Credentials are deliberately absent from this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelConnection {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub authentication: Authentication,
    #[serde(default = "default_api_key_header")]
    pub api_key_header: String,
    pub timeout_seconds: u64,
    pub organization: String,
    pub project: String,
    pub has_credential: bool,
    #[serde(skip)]
    pub environment_credential: bool,
    /// Masked preview of the active credential (e.g. `sk-or-••••1234`), populated in
    /// memory by the runtime/provider and never written to disk (`#[serde(skip)]`).
    #[serde(skip)]
    pub key_preview: String,
    #[serde(default)]
    pub manual_models: Vec<ModelInfo>,
}

impl ModelConnection {
    pub fn new(id: &str, kind: ProviderKind) -> Self {
        Self {
            id: id.to_owned(),
            name: kind.label().to_owned(),
            kind,
            base_url: kind.default_url().to_owned(),
            authentication: match kind {
                ProviderKind::OpenRouter => Authentication::Bearer,
                ProviderKind::Anthropic => Authentication::ApiKey,
                ProviderKind::Ollama | ProviderKind::OpenAiCompatible => Authentication::None,
            },
            api_key_header: default_api_key_header(),
            timeout_seconds: 60,
            organization: String::new(),
            project: String::new(),
            has_credential: false,
            environment_credential: false,
            key_preview: String::new(),
            manual_models: Vec::new(),
        }
    }

    /// Whether this connection can be used now: it needs no key, or one is available.
    pub fn is_ready(&self) -> bool {
        self.authentication == Authentication::None
            || self.has_credential
            || self.environment_credential
    }

    /// Returns a human-readable masked representation of the active API key when one is present.
    pub fn active_key_display(&self) -> Option<String> {
        if !self.key_preview.is_empty() {
            Some(self.key_preview.clone())
        } else if self.has_credential {
            Some(match self.kind {
                ProviderKind::OpenRouter => "sk-or-••••••••".to_owned(),
                _ => "••••••••".to_owned(),
            })
        } else if self.environment_credential {
            Some(match self.kind {
                ProviderKind::OpenRouter => "OPENROUTER_API_KEY (system)".to_owned(),
                ProviderKind::Ollama => "OLLAMA_API_KEY (system)".to_owned(),
                _ => "System environment key".to_owned(),
            })
        } else {
            None
        }
    }
}

fn default_api_key_header() -> String {
    "x-api-key".into()
}

/// A secret can cross the command channel, but never appears in Debug or JSON.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct ModelSecret(pub String);

impl ModelSecret {
    /// Produces a safe, non-reversible masked preview of the key (e.g. `sk-or-••••1234`
    /// or `te••••ey`) so the user can verify which key is active without exposing the secret.
    pub fn masked_preview(&self) -> String {
        let trimmed = self.0.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        let chars: Vec<char> = trimmed.chars().collect();
        if chars.len() <= 4 {
            return "••••••••".to_owned();
        }
        if chars.len() <= 10 {
            let prefix: String = chars[..2].iter().collect();
            let suffix: String = chars[chars.len() - 2..].iter().collect();
            return format!("{prefix}••••{suffix}");
        }
        let prefix_len = if trimmed.starts_with("sk-or-") { 6 } else { 4 };
        let prefix: String = chars[..prefix_len].iter().collect();
        let suffix: String = chars[chars.len() - 4..].iter().collect();
        format!("{prefix}••••{suffix}")
    }
}

impl std::fmt::Debug for ModelSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialChange {
    Keep,
    Replace(ModelSecret),
    Remove,
}

/// Identifiers are scoped to a connection; two servers can expose the same name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ModelKey {
    pub connection_id: String,
    pub model_id: String,
}

/// What a model produces, in the words a non-technical user would use. Only `Chat`
/// models can answer messages; the rest are listed so the catalog stays complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModelKind {
    Chat,
    Image,
    Video,
    Speech,
    Transcription,
    Embeddings,
    Other,
}

impl ModelKind {
    pub const ALL: [Self; 7] = [
        Self::Chat,
        Self::Image,
        Self::Video,
        Self::Speech,
        Self::Transcription,
        Self::Embeddings,
        Self::Other,
    ];

    /// Plural name for a "Show:" picker.
    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat models",
            Self::Image => "Picture makers",
            Self::Video => "Video makers",
            Self::Speech => "Voices (text to speech)",
            Self::Transcription => "Speech to text",
            Self::Embeddings => "Search helpers",
            Self::Other => "Other specialist tools",
        }
    }

    /// Short tag shown on each model.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Image => "Makes pictures",
            Self::Video => "Makes video",
            Self::Speech => "Speaks aloud",
            Self::Transcription => "Speech to text",
            Self::Embeddings => "Search helper",
            Self::Other => "Specialist tool",
        }
    }

    pub fn value(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Image => "image",
            Self::Video => "video",
            Self::Speech => "speech",
            Self::Transcription => "transcription",
            Self::Embeddings => "embeddings",
            Self::Other => "other",
        }
    }

    pub fn from_value(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.value() == value)
    }
}

/// Prices retain the provider's exact decimal strings. Unknown prices stay unknown.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub context_length: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    pub supported_parameters: Vec<String>,
    pub pricing: BTreeMap<String, String>,
    pub created: Option<u64>,
    pub knowledge_cutoff: Option<String>,
    pub expiration_date: Option<String>,
    pub tokenizer: Option<String>,
    pub family: Option<String>,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub details_loaded: bool,
    pub manual: bool,
}

impl ModelInfo {
    pub fn key(&self, connection_id: &str) -> ModelKey {
        ModelKey {
            connection_id: connection_id.to_owned(),
            model_id: self.id.clone(),
        }
    }

    pub fn supports(&self, capability: &str) -> bool {
        self.supported_parameters.iter().any(|p| p == capability)
    }

    pub fn can_chat(&self) -> bool {
        // Compatible servers often omit capability metadata. They may be tried
        // explicitly; known non-text models remain unavailable for chat.
        self.output_modalities.is_empty() || self.output_modalities.iter().any(|m| m == "text")
    }

    pub fn kind(&self) -> ModelKind {
        if self.can_chat() {
            return ModelKind::Chat;
        }
        let makes = |names: &[&str]| {
            self.output_modalities
                .iter()
                .any(|m| names.contains(&m.as_str()))
        };
        if makes(&["image"]) {
            ModelKind::Image
        } else if makes(&["video"]) {
            ModelKind::Video
        } else if makes(&["speech", "audio"]) {
            ModelKind::Speech
        } else if makes(&["transcription"]) {
            ModelKind::Transcription
        } else if makes(&["embeddings", "embedding"]) {
            ModelKind::Embeddings
        } else {
            ModelKind::Other
        }
    }

    pub fn token_price(&self, name: &str) -> Option<f64> {
        self.pricing
            .get(name)?
            .parse::<f64>()
            .ok()
            .filter(|p| p.is_finite() && *p >= 0.)
    }

    pub fn is_free(&self) -> bool {
        // Zero token prices do not make image/video/request-based models free.
        !self.pricing.is_empty()
            && self
                .pricing
                .values()
                .all(|v| v.parse::<f64>().is_ok_and(|n| n == 0.))
    }

    /// Estimated dollars for one ordinary chat message: a short question with some
    /// conversation history (~1,500 tokens read) and a medium reply (~500 written).
    /// People compare "about 1¢ a message"; per-million-token rates mean nothing to them.
    pub fn typical_message_cost(&self) -> Option<f64> {
        let cost = self.token_price("prompt")? * 1_500.0 + self.token_price("completion")? * 500.0;
        cost.is_finite().then_some(cost)
    }

    /// Price in plain words, for choosing a model without knowing what a token is.
    pub fn plain_price(&self) -> String {
        if self.is_free() {
            return "Free".into();
        }
        match self.typical_message_cost() {
            Some(cost) if cost < 0.001 => "Less than 0.1¢ a message".into(),
            Some(cost) if cost < 0.01 => format!("About {:.1}¢ a message", cost * 100.0),
            Some(cost) if cost < 1.0 => format!("About {:.0}¢ a message", cost * 100.0),
            Some(cost) => format!("About ${cost:.2} a message"),
            None => "Price not listed".into(),
        }
    }

    /// Everyday sentence from abilities the catalog lists.
    ///
    /// The model name and provider description are not abilities. When nothing
    /// listed maps to a plain phrase, this says so instead of guessing a use.
    pub fn plain_good_for(&self) -> String {
        let mut parts: Vec<&'static str> = Vec::new();
        if self.supports("reasoning") || self.supports("include_reasoning") {
            parts.push("working through a hard question step by step");
        }
        if self.supports("tools") {
            parts.push("using tools you already set up");
        }
        if self.lists_input("image") {
            parts.push("looking at pictures you share");
        }
        if self.lists_input("video") {
            parts.push("looking at videos you share");
        }
        if self.lists_input("audio") || self.lists_input("speech") {
            parts.push("listening to sound you share");
        }
        if self.supports("response_format") || self.supports("structured_outputs") {
            parts.push("giving an answer in a layout you ask for");
        }
        if self.lists_output("image") {
            parts.push("making pictures");
        }
        if self.lists_output("video") {
            parts.push("making videos");
        }
        if self.lists_output("audio") || self.lists_output("speech") {
            parts.push("speaking replies out loud");
        }
        match parts.as_slice() {
            [] => "What it is good for is not listed.".into(),
            [only] => format!("Good for {only}."),
            [first, second] => format!("Good for {first} and {second}."),
            [leading @ .., last] => format!("Good for {}, and {last}.", leading.join(", ")),
        }
    }

    fn lists_input(&self, modality: &str) -> bool {
        self.input_modalities.iter().any(|value| value == modality)
    }

    fn lists_output(&self, modality: &str) -> bool {
        self.output_modalities.iter().any(|value| value == modality)
    }
}

/// Light or dark app theme. Saved alongside model preferences so the choice survives restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

/// Interface size relative to the native 16px root (browser-style page zoom).
///
/// Every rem-based length and text role scales with it, independently of the
/// display's own scale factor. Fixed levels keep layouts testable; saved values
/// from elsewhere snap to the nearest level instead of failing to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(from = "u16", into = "u16")]
pub struct InterfaceScale(u16);

impl InterfaceScale {
    const LEVELS: [u16; 9] = [75, 80, 90, 100, 110, 125, 150, 175, 200];
    pub const SMALLEST: Self = Self(Self::LEVELS[0]);
    pub const LARGEST: Self = Self(Self::LEVELS[Self::LEVELS.len() - 1]);

    #[must_use]
    pub fn from_percent(percent: u16) -> Self {
        let nearest = Self::LEVELS
            .iter()
            .min_by_key(|level| level.abs_diff(percent))
            .copied()
            .unwrap_or(100);
        Self(nearest)
    }

    #[must_use]
    pub fn percent(self) -> u16 {
        self.0
    }

    /// Multiplier for the root font size.
    #[must_use]
    pub fn factor(self) -> f32 {
        f32::from(self.0) / 100.
    }

    #[must_use]
    pub fn larger(self) -> Self {
        self.step(1)
    }

    #[must_use]
    pub fn smaller(self) -> Self {
        self.step(-1)
    }

    fn step(self, by: isize) -> Self {
        let index = Self::LEVELS.iter().position(|&l| l == self.0).unwrap_or(3);
        let next = index.saturating_add_signed(by).min(Self::LEVELS.len() - 1);
        Self(Self::LEVELS[next])
    }
}

impl Default for InterfaceScale {
    fn default() -> Self {
        Self(100)
    }
}

impl From<u16> for InterfaceScale {
    fn from(percent: u16) -> Self {
        Self::from_percent(percent)
    }
}

impl From<InterfaceScale> for u16 {
    fn from(scale: InterfaceScale) -> Self {
        scale.0
    }
}

/// Default conversational tone and explanation style for the Personal AI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStyle {
    /// Warm, everyday language with zero technical jargon — ideal for non-technical users.
    #[default]
    FriendlySimple,
    /// Balanced, clear explanations with practical examples.
    BalancedClear,
    /// Brief, direct bullet points without extra filler.
    ConciseDirect,
    /// Patient, numbered step-by-step walkthroughs.
    StepByStep,
}

impl ResponseStyle {
    pub const ALL: [Self; 4] = [
        Self::FriendlySimple,
        Self::BalancedClear,
        Self::ConciseDirect,
        Self::StepByStep,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::FriendlySimple => "Simple & Friendly",
            Self::BalancedClear => "Balanced & Clear",
            Self::ConciseDirect => "Short & Direct",
            Self::StepByStep => "Step-by-Step Guide",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::FriendlySimple => {
                "Everyday language with no technical jargon — explains things like a patient friend."
            }
            Self::BalancedClear => "Clear, helpful replies with just the right amount of detail.",
            Self::ConciseDirect => {
                "Quick bullet points and brief answers that get straight to the point."
            }
            Self::StepByStep => "Walks you through tasks one small, numbered step at a time.",
        }
    }

    pub fn instruction_clause(self) -> &'static str {
        match self {
            Self::FriendlySimple => {
                "Use warm, friendly, everyday language. Avoid technical jargon unless asked, and explain concepts simply so anyone can follow."
            }
            Self::BalancedClear => {
                "Provide balanced, well-structured answers with clear explanations."
            }
            Self::ConciseDirect => {
                "Keep answers brief, direct, and easy to scan using short bullet points."
            }
            Self::StepByStep => {
                "Break explanations and instructions into clear, numbered steps, checking one step at a time."
            }
        }
    }
}

/// User-facing agentic AI settings: standing context ("always context"), response style,
/// cross-chat memory, helper tools, web search, and action confirmation guardrails.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent user-facing toggles for agentic AI capabilities"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// Standing instructions and personal context included in every conversation automatically.
    pub always_context: String,
    /// Default explanation style and tone.
    pub response_style: ResponseStyle,
    /// Whether Sovra remembers facts and episodes across conversations.
    pub memory_enabled: bool,
    /// Whether Sovra can render interactive AG-UI cards, tables, and checklists in chat.
    pub tools_enabled: bool,
    /// Whether Sovra can search the web for current information when supported.
    pub web_search_enabled: bool,
    /// Guardrail: whether Sovra asks before running important actions.
    pub confirm_actions: bool,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            always_context: String::new(),
            response_style: ResponseStyle::FriendlySimple,
            memory_enabled: true,
            tools_enabled: true,
            web_search_enabled: true,
            confirm_actions: true,
        }
    }
}

impl AgentSettings {
    /// Combines the base system instruction with the chosen reply language, the
    /// user's response style, safety confirmation policy, and standing `always_context`.
    pub fn compose_system_instruction(
        &self,
        base_instruction: &str,
        language: &UiLanguage,
    ) -> String {
        let mut out = format!(
            "{base_instruction}\n\n{}\n\nResponse style preference: {}",
            language.reply_instruction(),
            self.response_style.instruction_clause()
        );
        if self.confirm_actions {
            out.push_str(
                "\nSafety guardrail: Always ask for the user's confirmation before performing any external, destructive, or state-changing action.",
            );
        }
        let custom = self.always_context.trim();
        if !custom.is_empty() {
            out.push_str(
                "\n\nAlways-on context from the user (keep this in mind in every reply):\n",
            );
            out.push_str(custom);
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModelPreferences {
    pub favorites: BTreeSet<ModelKey>,
    pub active: Option<ModelKey>,
    /// Explicit media choices; documents use the chat model until assigned separately.
    #[serde(default)]
    pub purposes: BTreeMap<crate::files::ModelPurpose, ModelKey>,
    /// `default` keeps files written before this field existed loadable.
    #[serde(default)]
    pub appearance: Appearance,
    /// Chrome and agent reply language. Missing saved field loads as Arabic.
    #[serde(default)]
    pub language: UiLanguage,
    #[serde(default)]
    pub interface_scale: InterfaceScale,
    /// Standing context, tone, memory, tool, and safety preferences for the agent.
    #[serde(default)]
    pub agent_settings: AgentSettings,
    /// Whether the user has completed or dismissed the guided onboarding tutorial.
    #[serde(default)]
    pub onboarding_completed: bool,
}

/// Deterministic setup progression stage so the UI only asks for what is still missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupStage {
    /// Model connections and preferences are still loading from the runtime.
    Loading,
    /// Step 1: No connection has an active credential or available local models yet.
    NeedsConnection,
    /// Step 1b: A specific model was chosen, but its connection requires an API key.
    NeedsModelKey {
        model_id: String,
        connection_id: String,
        connection_name: String,
    },
    /// Step 2: A connection/key is ready, and the user only needs to pick a model.
    NeedsModelSelection {
        connected_service: String,
        key_preview: Option<String>,
    },
    /// Setup complete: a model is selected and its connection has required credentials.
    Ready { active: ModelKey },
}

/// Curated recommendation category for non-technical users choosing a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecommendationKind {
    /// Zero-cost starter model to try Sovra immediately.
    FreeStarter,
    /// Recent, tool-capable, moderately priced model for everyday conversation.
    EverydaySmart,
    /// Model that thinks before answering, for hard coding, maths and analysis.
    DeepReasoning,
}

impl RecommendationKind {
    pub fn badge(self) -> &'static str {
        match self {
            Self::FreeStarter => "FREE",
            Self::EverydaySmart => "EVERYDAY",
            Self::DeepReasoning => "THINKS LONGER",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::FreeStarter => {
                "Costs nothing. Good for trying Sovra; it can be slower and has daily limits."
            }
            Self::EverydaySmart => "A recent all-rounder for everyday chat, writing and questions.",
            Self::DeepReasoning => {
                "Thinks before it answers. Slower, but better for hard problems, maths and code."
            }
        }
    }
}

/// Typical-message ceilings that keep a suggestion affordable for daily use; the
/// newest qualifying model wins, so suggestions follow the catalog, not model names.
const EVERYDAY_MAX_MESSAGE_COST: f64 = 0.03;
const REASONING_MAX_MESSAGE_COST: f64 = 0.06;

/// A highlighted model recommendation card shown on the setup home screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecommendedModelCard {
    pub kind: RecommendationKind,
    pub key: ModelKey,
    pub name: String,
    pub author: String,
    pub connection_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CatalogSnapshot {
    pub models: Vec<ModelInfo>,
    pub fetched_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CatalogState {
    pub snapshot: CatalogSnapshot,
    pub loading: bool,
    pub cached: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CatalogScope {
    #[default]
    All,
    OpenRouter,
    Ollama,
    Custom,
    Favorites,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PriceFilter {
    #[default]
    Any,
    Free,
    Paid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelSort {
    #[default]
    Name,
    Newest,
    Context,
    InputPrice,
    OutputPrice,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelFilter {
    pub search: String,
    pub scope: CatalogScope,
    pub connection_id: Option<String>,
    pub author: Option<String>,
    /// `None` shows every kind. The library defaults to `Chat`.
    pub kind: Option<ModelKind>,
    pub input_modality: Option<String>,
    pub output_modality: Option<String>,
    pub min_context: u64,
    pub price: PriceFilter,
    pub tools: bool,
    pub reasoning: bool,
    pub structured_output: bool,
    pub sort: ModelSort,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CatalogFacets {
    pub authors: BTreeSet<String>,
    pub input_modalities: BTreeSet<String>,
    pub output_modalities: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelsState {
    pub connections: Vec<ModelConnection>,
    pub catalogs: BTreeMap<String, CatalogState>,
    pub preferences: ModelPreferences,
    pub loaded: bool,
    pub busy: bool,
    pub notice: Option<String>,
    pub error: Option<String>,
    pub pull_progress: Option<String>,
    /// Why the chosen model cannot answer right now, as reported by the runtime.
    pub active_problem: Option<String>,
}

impl ModelsState {
    pub fn apply(&mut self, event: &ModelEvent) {
        match event {
            ModelEvent::Loaded {
                connections,
                preferences,
            } => {
                self.connections.clone_from(connections);
                self.preferences.clone_from(preferences);
                self.loaded = true;
                self.busy = false;
            }
            ModelEvent::CatalogLoading { connection_id } => {
                let catalog = self.catalogs.entry(connection_id.clone()).or_default();
                catalog.loading = true;
                catalog.error = None;
            }
            ModelEvent::CatalogLoaded {
                connection_id,
                snapshot,
                cached,
            } => {
                self.catalogs.insert(
                    connection_id.clone(),
                    CatalogState {
                        snapshot: snapshot.clone(),
                        cached: *cached,
                        ..CatalogState::default()
                    },
                );
            }
            ModelEvent::CatalogFailed {
                connection_id,
                message,
            } => {
                let catalog = self.catalogs.entry(connection_id.clone()).or_default();
                catalog.loading = false;
                catalog.error = Some(message.clone());
            }
            ModelEvent::DetailsLoaded { key, model } => {
                if let Some(catalog) = self.catalogs.get_mut(&key.connection_id)
                    && let Some(existing) = catalog
                        .snapshot
                        .models
                        .iter_mut()
                        .find(|m| m.id == key.model_id)
                {
                    *existing = model.as_ref().clone();
                }
            }
            ModelEvent::ConnectionDeleted { id } => {
                self.connections.retain(|c| c.id != *id);
                self.catalogs.remove(id);
                self.preferences
                    .purposes
                    .retain(|_, key| key.connection_id != *id);
                self.preferences
                    .favorites
                    .retain(|k| k.connection_id != *id);
                if self
                    .preferences
                    .active
                    .as_ref()
                    .is_some_and(|k| k.connection_id == *id)
                {
                    self.preferences.active = None;
                    self.active_problem = None;
                }
            }
            ModelEvent::ConnectionSaved { .. } => self.busy = false,
            ModelEvent::PreferencesChanged(preferences) => self.preferences.clone_from(preferences),
            ModelEvent::OperationStarted => {
                self.busy = true;
                self.error = None;
                self.notice = None;
            }
            ModelEvent::Notice(message) => {
                self.busy = false;
                self.notice = Some(message.clone());
                self.error = None;
            }
            ModelEvent::Error(message) => {
                self.busy = false;
                self.error = Some(message.clone());
            }
            ModelEvent::PullProgress(message) => self.pull_progress.clone_from(message),
            ModelEvent::ActiveModelStatus { problem } => self.active_problem.clone_from(problem),
        }
    }

    pub fn model(&self, key: &ModelKey) -> Option<&ModelInfo> {
        self.catalogs
            .get(&key.connection_id)?
            .snapshot
            .models
            .iter()
            .find(|m| m.id == key.model_id)
    }

    /// Resolves a file turn without silently changing the user's chosen media models.
    pub fn model_for_files(
        &self,
        attachments: &[crate::files::Attachment],
    ) -> Result<ModelKey, crate::files::FileSetup> {
        use crate::files::{FileSetup, ModelPurpose};
        let purpose = attachments
            .iter()
            .map(|file| file.purpose)
            .max()
            .unwrap_or(ModelPurpose::Documents);
        let fail = |message: String| FileSetup { purpose, message };
        if !self.loaded {
            return Err(fail(
                "Your model settings are still loading. Your files are safe here.".into(),
            ));
        }
        for required in attachments.iter().map(|file| file.purpose) {
            if required != ModelPurpose::Documents
                && !self.preferences.purposes.contains_key(&required)
            {
                return Err(FileSetup {
                    purpose: required,
                    message: format!(
                        "Let's choose an AI to {}. Your files and message will stay here while you set it up.",
                        required.label().to_lowercase()
                    ),
                });
            }
        }
        let key = self.preferences.purposes.get(&purpose)
            .or_else(|| (purpose == ModelPurpose::Documents).then_some(self.preferences.active.as_ref()).flatten())
            .ok_or_else(|| fail(format!("Let's choose an AI to {}. Your files and message will stay here while you set it up.", purpose.label().to_lowercase())))?;
        let connection = self.connections.iter().find(|connection| connection.id == key.connection_id)
            .ok_or_else(|| fail("The service for this model is no longer connected. Choose another model to continue.".into()))?;
        if connection.authentication != Authentication::None
            && !connection.has_credential
            && !connection.environment_credential
        {
            return Err(fail(format!(
                "Add the key for {} so I can read this file. Your files and message will stay here.",
                connection.name
            )));
        }
        let model = self.model(key).ok_or_else(|| {
            fail(
                "This model is not available yet. Refresh its service or choose another model."
                    .into(),
            )
        })?;
        if !attachments
            .iter()
            .all(|file| file.purpose.accepts(model, connection.kind))
        {
            return Err(fail("These files need a model that can read all their formats. Choose a compatible model, or send the different kinds of files separately.".into()));
        }
        // OpenAI-compatible chat adapters can encode only WAV and MP3 audio.
        if connection.kind == ProviderKind::OpenAiCompatible
            && attachments.iter().any(|file| {
                file.purpose == ModelPurpose::Audio
                    && !matches!(
                        file.mime_type.as_str(),
                        "audio/wav" | "audio/x-wav" | "audio/mpeg" | "audio/mp3"
                    )
            })
        {
            return Err(fail("This service reads WAV and MP3 audio. Save the recording in one of those formats, or choose an audio model through OpenRouter.".into()));
        }
        Ok(key.clone())
    }

    /// Returns `true` when a chat model is selected, its connection has any required key,
    /// and the runtime has not reported that the model cannot answer.
    pub fn has_active_llm(&self) -> bool {
        matches!(self.setup_stage(), SetupStage::Ready { .. })
    }

    /// Returns the `OpenRouter` connection if one is configured.
    pub fn openrouter_connection(&self) -> Option<&ModelConnection> {
        self.connections
            .iter()
            .find(|c| c.kind == ProviderKind::OpenRouter)
    }

    /// Returns the deterministic setup progression stage so the UI only displays what is missing.
    pub fn setup_stage(&self) -> SetupStage {
        if !self.loaded {
            return SetupStage::Loading;
        }
        if let Some(active) = &self.preferences.active
            && let Some(conn) = self
                .connections
                .iter()
                .find(|c| c.id == active.connection_id)
        {
            if !conn.is_ready() {
                return SetupStage::NeedsModelKey {
                    model_id: active.model_id.clone(),
                    connection_id: conn.id.clone(),
                    connection_name: conn.name.clone(),
                };
            }
            // A saved choice the runtime cannot run is no choice at all: ask for a new
            // model rather than claiming setup is complete.
            if self.active_problem.is_none() {
                return SetupStage::Ready {
                    active: active.clone(),
                };
            }
            return SetupStage::NeedsModelSelection {
                connected_service: conn.name.clone(),
                key_preview: conn.active_key_display(),
            };
        }
        if let Some(router) = self
            .openrouter_connection()
            .filter(|c| c.has_credential || c.environment_credential)
        {
            return SetupStage::NeedsModelSelection {
                connected_service: router.name.clone(),
                key_preview: router.active_key_display(),
            };
        }
        if let Some(other) = self.connections.iter().find(|c| {
            c.has_credential
                || c.environment_credential
                || (c.authentication == Authentication::None
                    && self
                        .catalogs
                        .get(&c.id)
                        .is_some_and(|cat| !cat.snapshot.models.is_empty()))
        }) {
            return SetupStage::NeedsModelSelection {
                connected_service: other.name.clone(),
                key_preview: other.active_key_display(),
            };
        }
        SetupStage::NeedsConnection
    }

    /// Returns a plain-English explanation of what is still needed when no LLM is active.
    pub fn setup_status_hint(&self) -> Option<String> {
        match self.setup_stage() {
            SetupStage::Loading | SetupStage::Ready { .. } => None,
            SetupStage::NeedsModelKey {
                connection_name, ..
            } => Some(format!(
                "Your AI is chosen. {connection_name} just needs your key again so it can reply."
            )),
            SetupStage::NeedsModelSelection {
                connected_service, ..
            } => Some(self.active_problem.as_ref().map_or_else(
                || format!("{connected_service} is connected. Now pick the AI you want to talk to."),
                |problem| format!("The AI you chose can't answer right now: {problem} Please pick another one."),
            )),
            SetupStage::NeedsConnection => Some(
                "To start, connect an AI service. The easiest is a free OpenRouter key, or Ollama on this computer.".to_owned(),
            ),
        }
    }

    /// Suggests up to 3 curated model recommendation cards (`FreeStarter`, `EverydaySmart`, `DeepReasoning`)
    /// from ready connections so non-technical users never have to hunt through 300+ models.
    pub fn recommended_model_cards(&self) -> Vec<RecommendedModelCard> {
        let mut cards = Vec::new();
        let mut used_keys = BTreeSet::new();

        // 1. Best Free Starter (zero-cost online model)
        if let Some(free_key) = self.recommended_chat_model()
            && let Some(model) = self.model(&free_key)
        {
            let connection_name = self
                .connections
                .iter()
                .find(|c| c.id == free_key.connection_id)
                .map_or_else(|| "OpenRouter".to_owned(), |c| c.name.clone());
            used_keys.insert(free_key.clone());
            cards.push(RecommendedModelCard {
                kind: RecommendationKind::FreeStarter,
                key: free_key,
                name: model.name.clone(),
                author: model.author.clone(),
                connection_name,
            });
        }

        // Gather all chat-capable models from ready connections
        let ready_models: Vec<(&ModelConnection, &ModelInfo)> = self
            .connections
            .iter()
            .filter(|c| c.is_ready())
            .filter_map(|c| self.catalogs.get(&c.id).map(|cat| (c, cat)))
            .flat_map(|(c, cat)| {
                cat.snapshot
                    .models
                    .iter()
                    .filter(|m| m.kind() == ModelKind::Chat && !m.output_modalities.is_empty())
                    .map(move |m| (c, m))
            })
            .collect();

        // Live catalog data only: paid (the free pick is above), priced, tool-capable
        // (Sovra's chat features need tools), affordable, then newest first.
        let suggest = |used: &BTreeSet<ModelKey>, reasoning: bool, max_cost: f64| {
            ready_models
                .iter()
                .copied()
                .filter(|(c, m)| !used.contains(&m.key(&c.id)) && !m.is_free())
                .filter(|(_, m)| m.supports("tools"))
                .filter(|(_, m)| {
                    (m.supports("reasoning") || m.supports("include_reasoning")) == reasoning
                })
                .filter(|(_, m)| {
                    m.typical_message_cost()
                        .is_some_and(|cost| cost <= max_cost)
                })
                .max_by(|(left_connection, left), (right_connection, right)| {
                    left.created
                        .cmp(&right.created)
                        .then_with(|| {
                            right
                                .typical_message_cost()
                                .unwrap_or(f64::INFINITY)
                                .total_cmp(&left.typical_message_cost().unwrap_or(f64::INFINITY))
                        })
                        .then_with(|| right.id.cmp(&left.id))
                        .then_with(|| right_connection.id.cmp(&left_connection.id))
                })
        };
        if let Some((conn, model)) = suggest(&used_keys, false, EVERYDAY_MAX_MESSAGE_COST) {
            let key = model.key(&conn.id);
            used_keys.insert(key.clone());
            cards.push(RecommendedModelCard {
                kind: RecommendationKind::EverydaySmart,
                key,
                name: model.name.clone(),
                author: model.author.clone(),
                connection_name: conn.name.clone(),
            });
        }

        let reasoning = suggest(&used_keys, true, REASONING_MAX_MESSAGE_COST);
        if let Some((conn, model)) = reasoning {
            let key = model.key(&conn.id);
            used_keys.insert(key.clone());
            cards.push(RecommendedModelCard {
                kind: RecommendationKind::DeepReasoning,
                key,
                name: model.name.clone(),
                author: model.author.clone(),
                connection_name: conn.name.clone(),
            });
        }

        cards
    }

    /// Suggests a connected online chat model with explicitly zero catalog prices.
    /// Never recommends a paid or unknown-price model as the free starting choice.
    pub fn recommended_chat_model(&self) -> Option<ModelKey> {
        self.connections
            .iter()
            .filter(|connection| {
                connection.kind == ProviderKind::OpenRouter
                    && (connection.has_credential || connection.environment_credential)
            })
            .filter_map(|connection| {
                self.catalogs
                    .get(&connection.id)
                    .map(|catalog| (connection, catalog))
            })
            .flat_map(|(connection, catalog)| {
                catalog
                    .snapshot
                    .models
                    .iter()
                    .filter(|model| {
                        model
                            .output_modalities
                            .iter()
                            .any(|output| output == "text")
                            && model.is_free()
                            && model.pricing.contains_key("prompt")
                            && model.pricing.contains_key("completion")
                    })
                    .map(move |model| model.key(&connection.id))
            })
            .min_by_key(|key| {
                (
                    key.model_id != "openrouter/free",
                    key.model_id.clone(),
                    key.connection_id.clone(),
                )
            })
    }

    pub fn filtered(&self, filter: &ModelFilter) -> Vec<ModelKey> {
        let words: Vec<_> = filter
            .search
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let mut matches = Vec::new();
        for connection in &self.connections {
            if filter
                .connection_id
                .as_ref()
                .is_some_and(|id| id != &connection.id)
            {
                continue;
            }
            let scope_matches = match filter.scope {
                CatalogScope::All | CatalogScope::Favorites => true,
                CatalogScope::OpenRouter => connection.kind == ProviderKind::OpenRouter,
                CatalogScope::Ollama => connection.kind == ProviderKind::Ollama,
                CatalogScope::Custom => matches!(
                    connection.kind,
                    ProviderKind::OpenAiCompatible | ProviderKind::Anthropic
                ),
            };
            if !scope_matches {
                continue;
            }
            let Some(catalog) = self.catalogs.get(&connection.id) else {
                continue;
            };
            for model in &catalog.snapshot.models {
                let key = model.key(&connection.id);
                if filter.scope == CatalogScope::Favorites
                    && !self.preferences.favorites.contains(&key)
                {
                    continue;
                }
                let searchable = format!(
                    "{} {} {} {} {}",
                    model.id, model.name, model.author, model.description, connection.name
                )
                .to_lowercase();
                if !words.iter().all(|word| searchable.contains(word))
                    || filter.author.as_ref().is_some_and(|a| &model.author != a)
                    || filter.kind.is_some_and(|kind| model.kind() != kind)
                    || filter
                        .input_modality
                        .as_ref()
                        .is_some_and(|m| !model.input_modalities.contains(m))
                    || filter
                        .output_modality
                        .as_ref()
                        .is_some_and(|m| !model.output_modalities.contains(m))
                    || (filter.min_context > 0
                        && model.context_length.unwrap_or(0) < filter.min_context)
                    || (filter.tools && !model.supports("tools"))
                    || (filter.reasoning
                        && !model.supports("reasoning")
                        && !model.supports("include_reasoning"))
                    || (filter.structured_output
                        && !model.supports("structured_outputs")
                        && !model.supports("response_format"))
                    || (filter.price == PriceFilter::Free && !model.is_free())
                    || (filter.price == PriceFilter::Paid
                        && !model
                            .pricing
                            .values()
                            .any(|p| p.parse::<f64>().is_ok_and(|p| p > 0.)))
                {
                    continue;
                }
                matches.push((key, model));
            }
        }
        matches.sort_by(|(ka, a), (kb, b)| {
            let order = match filter.sort {
                ModelSort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                ModelSort::Newest => b.created.cmp(&a.created),
                ModelSort::Context => b.context_length.cmp(&a.context_length),
                ModelSort::InputPrice => {
                    compare_price(a.token_price("prompt"), b.token_price("prompt"))
                }
                ModelSort::OutputPrice => {
                    compare_price(a.token_price("completion"), b.token_price("completion"))
                }
            };
            order.then_with(|| ka.cmp(kb))
        });
        matches.into_iter().map(|(key, _)| key).collect()
    }

    pub fn facets(&self, scope: CatalogScope) -> CatalogFacets {
        let mut facets = CatalogFacets::default();
        for key in self.filtered(&ModelFilter {
            scope,
            ..ModelFilter::default()
        }) {
            if let Some(model) = self.model(&key) {
                if !model.author.is_empty() {
                    facets.authors.insert(model.author.clone());
                }
                facets.input_modalities.extend(
                    model
                        .input_modalities
                        .iter()
                        .filter(|m| !m.is_empty())
                        .cloned(),
                );
                facets.output_modalities.extend(
                    model
                        .output_modalities
                        .iter()
                        .filter(|m| !m.is_empty())
                        .cloned(),
                );
            }
        }
        facets
    }
}

fn compare_price(a: Option<f64>, b: Option<f64>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelCommand {
    Load,
    Refresh {
        connection_id: String,
    },
    SaveConnection {
        connection: ModelConnection,
        credential: CredentialChange,
    },
    TestConnection {
        connection: ModelConnection,
        credential: CredentialChange,
    },
    DeleteConnection {
        id: String,
    },
    AddManualModel {
        connection_id: String,
        model: Box<ModelInfo>,
    },
    RemoveManualModel {
        key: ModelKey,
    },
    SetFavorite {
        key: ModelKey,
        favorite: bool,
    },
    Activate {
        key: ModelKey,
    },
    AssignPurpose {
        purpose: crate::files::ModelPurpose,
        key: Option<ModelKey>,
    },
    FetchDetails {
        key: ModelKey,
    },
    PullOllama {
        connection_id: String,
        model: String,
    },
    CancelPull,
    SetAppearance {
        appearance: Appearance,
    },
    SetLanguage {
        language: UiLanguage,
    },
    SetInterfaceScale {
        scale: InterfaceScale,
    },
    SetAgentSettings {
        settings: AgentSettings,
    },
    SetOnboardingCompleted {
        completed: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelEvent {
    Loaded {
        connections: Vec<ModelConnection>,
        preferences: ModelPreferences,
    },
    CatalogLoading {
        connection_id: String,
    },
    CatalogLoaded {
        connection_id: String,
        snapshot: CatalogSnapshot,
        cached: bool,
    },
    CatalogFailed {
        connection_id: String,
        message: String,
    },
    DetailsLoaded {
        key: ModelKey,
        model: Box<ModelInfo>,
    },
    ConnectionDeleted {
        id: String,
    },
    ConnectionSaved {
        id: String,
    },
    PreferencesChanged(ModelPreferences),
    OperationStarted,
    Notice(String),
    Error(String),
    PullProgress(Option<String>),
    /// The runtime's verdict on the chosen model: `None` once it can answer, or a
    /// plain-language reason it cannot (for example, it left the catalog).
    ActiveModelStatus {
        problem: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ModelError(pub String);

/// Catalog adapters share one protocol-neutral interface. The runtime owns tasks.
#[async_trait::async_trait]
pub trait ModelProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    async fn list(
        &self,
        connection: &ModelConnection,
        secret: &ModelSecret,
    ) -> Result<Vec<ModelInfo>, ModelError>;
    async fn details(
        &self,
        connection: &ModelConnection,
        secret: &ModelSecret,
        model: &ModelInfo,
    ) -> Result<ModelInfo, ModelError>;
}

#[cfg(test)]
mod tests;
