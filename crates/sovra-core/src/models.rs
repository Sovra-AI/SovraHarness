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
            manual_models: Vec::new(),
        }
    }
}

fn default_api_key_header() -> String {
    "x-api-key".into()
}

/// A secret can cross the command channel, but never appears in Debug or JSON.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct ModelSecret(pub String);

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

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModelPreferences {
    pub favorites: BTreeSet<ModelKey>,
    pub active: Option<ModelKey>,
    /// `default` keeps files written before this field existed loadable.
    #[serde(default)]
    pub appearance: Appearance,
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
