//! Model catalog, connection configuration and filtering. Network and credential IO
//! belong to provider adapters; this module is deterministic and backend independent.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

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
}

/// Light or dark app theme. Saved alongside model preferences so the choice survives restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
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
    /// Combines the base system instruction with the user's response style, safety
    /// confirmation policy, and standing `always_context`.
    pub fn compose_system_instruction(&self, base_instruction: &str) -> String {
        let mut out = format!(
            "{base_instruction}\n\nResponse style preference: {}",
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
    /// `default` keeps files written before this field existed loadable.
    #[serde(default)]
    pub appearance: Appearance,
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
    /// Fast, balanced everyday chat model for writing, email, and general questions.
    EverydaySmart,
    /// Step-by-step reasoning model for hard coding, math, and deep analysis.
    DeepReasoning,
}

impl RecommendationKind {
    pub fn badge(self) -> &'static str {
        match self {
            Self::FreeStarter => "FREE TO TRY",
            Self::EverydaySmart => "EVERYDAY CHAT",
            Self::DeepReasoning => "STEP-BY-STEP REASONING",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::FreeStarter => "Listed as free. Usage limits apply; availability can change.",
            Self::EverydaySmart => {
                "A chat model for writing and questions. Check its price before choosing."
            }
            Self::DeepReasoning => {
                "The service lists support for reasoning. Replies may take longer."
            }
        }
    }
}

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
                    .favorites
                    .retain(|k| k.connection_id != *id);
                if self
                    .preferences
                    .active
                    .as_ref()
                    .is_some_and(|k| k.connection_id == *id)
                {
                    self.preferences.active = None;
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

    /// Returns `true` when a chat model is selected and its connection has any required key.
    pub fn has_active_llm(&self) -> bool {
        let Some(active) = &self.preferences.active else {
            return false;
        };
        let Some(connection) = self
            .connections
            .iter()
            .find(|c| c.id == active.connection_id)
        else {
            return false;
        };
        connection.authentication == Authentication::None
            || connection.has_credential
            || connection.environment_credential
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
            if conn.authentication == Authentication::None
                || conn.has_credential
                || conn.environment_credential
            {
                return SetupStage::Ready {
                    active: active.clone(),
                };
            }
            return SetupStage::NeedsModelKey {
                model_id: active.model_id.clone(),
                connection_id: conn.id.clone(),
                connection_name: conn.name.clone(),
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
                model_id,
                connection_name,
                ..
            } => Some(format!(
                "You chose model “{model_id}”, but {connection_name} still needs an API key before it can reply."
            )),
            SetupStage::NeedsModelSelection {
                connected_service,
                key_preview,
            } => {
                let key_note = key_preview
                    .map_or_else(String::new, |preview| format!(" ({preview})"));
                Some(format!(
                    "Step 1 complete: {connected_service}{key_note} is connected! Now complete Step 2: click a recommended AI model below (or open Models) to start chatting."
                ))
            }
            SetupStage::NeedsConnection => Some(
                "Step 1 of 2: Connect your AI service. The easiest way is a free OpenRouter key (unlocks 300+ models), or use free local Ollama.".to_owned(),
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
            .filter(|c| {
                c.has_credential
                    || c.environment_credential
                    || c.authentication == Authentication::None
            })
            .filter_map(|c| self.catalogs.get(&c.id).map(|cat| (c, cat)))
            .flat_map(|(c, cat)| {
                cat.snapshot
                    .models
                    .iter()
                    .filter(|m| m.output_modalities.iter().any(|output| output == "text"))
                    .map(move |m| (c, m))
            })
            .collect();

        // Use live catalog costs, not provider names or an unverified quality ranking.
        let cost = |model: &ModelInfo| {
            model
                .token_price("prompt")
                .zip(model.token_price("completion"))
                .map(|(input, output)| input + output)
                .filter(|total| total.is_finite())
                .unwrap_or(f64::INFINITY)
        };
        let everyday = ready_models
            .iter()
            .copied()
            .filter(|(c, m)| !used_keys.contains(&m.key(&c.id)))
            .filter(|(_, m)| !m.supports("reasoning") && !m.supports("include_reasoning"))
            .min_by(|(left_connection, left), (right_connection, right)| {
                cost(left)
                    .total_cmp(&cost(right))
                    .then_with(|| left.id.cmp(&right.id))
                    .then_with(|| left_connection.id.cmp(&right_connection.id))
            });
        if let Some((conn, model)) = everyday {
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

        // Reasoning is an advertised capability, not a claim about answer quality.
        let reasoning = ready_models
            .iter()
            .copied()
            .filter(|(c, m)| {
                !used_keys.contains(&m.key(&c.id))
                    && (m.supports("reasoning") || m.supports("include_reasoning"))
            })
            .min_by(|(left_connection, left), (right_connection, right)| {
                cost(left)
                    .total_cmp(&cost(right))
                    .then_with(|| left.id.cmp(&right.id))
                    .then_with(|| left_connection.id.cmp(&right_connection.id))
            });
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
