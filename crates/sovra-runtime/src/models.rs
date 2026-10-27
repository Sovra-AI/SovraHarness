//! Model-library actor. Owns background discovery, persistence, cancellation and
//! the selected ADK harness; provider IO never blocks the UI or command loop.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_channel::{Receiver, Sender};
use sovra_adk::AdkHarness;
use sovra_core::files::ModelPurpose;
use sovra_core::models::{
    Authentication, CatalogSnapshot, CatalogState, CredentialChange, ModelCommand, ModelConnection,
    ModelError, ModelEvent, ModelInfo, ModelKey, ModelProvider, ModelSecret, ProviderKind,
};
use sovra_core::{AgentConfig, Event};
use sovra_providers::{ModelRepository, SavedModels, validate_connection};
use tokio::sync::RwLock;
use tokio::task::{AbortHandle, JoinSet};

/// The composition root supplies storage paths, credentials and concrete adapters.
pub struct ModelsConfig {
    pub repository: ModelRepository,
    pub providers: Vec<Arc<dyn ModelProvider>>,
}

impl std::fmt::Debug for ModelsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelsConfig")
            .field("repository", &self.repository)
            .field("providers", &self.providers.len())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ModelExecution {
    Legacy,
    Selected {
        // Boxed: `AdkHarness` dwarfs the other variants (`clippy::large_enum_variant`).
        harness: Box<AdkHarness>,
        timeout: Duration,
    },
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub(crate) struct FileModel {
    key: ModelKey,
    pub(crate) model: ModelInfo,
    pub(crate) provider: ProviderKind,
    pub(crate) execution: ModelExecution,
}

pub(crate) struct ModelsRuntime {
    commands: Sender<ModelCommand>,
    memory_control: crate::remembering::MemoryControl,
    pub(crate) execution: Arc<RwLock<ModelExecution>>,
    pub(crate) file_models: Arc<RwLock<BTreeMap<ModelPurpose, FileModel>>>,
    pub(crate) memory_enabled: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) web_search_enabled: Arc<std::sync::atomic::AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl ModelsRuntime {
    pub(crate) fn start(
        config: ModelsConfig,
        agent: AgentConfig,
        events: Sender<Event>,
        memory_control: crate::remembering::MemoryControl,
    ) -> Self {
        let (commands, receiver) = async_channel::unbounded();
        let execution = Arc::new(RwLock::new(ModelExecution::Unavailable(
            "Model settings are still loading.".into(),
        )));
        let file_models = Arc::new(RwLock::new(BTreeMap::new()));
        let memory_enabled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let web_search_enabled = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let actor = ModelActor {
            memory_control: memory_control.clone(),
            repository: config.repository,
            providers: config
                .providers
                .into_iter()
                .map(|p| (p.kind(), p))
                .collect(),
            saved: SavedModels::default(),
            catalogs: BTreeMap::new(),
            loaded: false,
            generations: BTreeMap::new(),
            refreshes: BTreeMap::new(),
            jobs: JoinSet::new(),
            pull: None,
            pull_generation: 0,
            pull_progress: async_channel::bounded(16),
            agent,
            execution: execution.clone(),
            file_models: file_models.clone(),
            memory_enabled: memory_enabled.clone(),
            web_search_enabled: web_search_enabled.clone(),
            events,
        };
        Self {
            memory_control,
            commands,
            execution,
            file_models,
            memory_enabled,
            web_search_enabled,
            // sovra-runtime owns the executor; this actor is the sanctioned spawn point
            // (owned JoinHandle, aborted on drop). The workspace-wide disallow steers
            // other crates here.
            #[expect(
                clippy::disallowed_methods,
                reason = "runtime-owned actor with abort-on-drop"
            )]
            task: tokio::spawn(actor.run(receiver)),
        }
    }

    pub(crate) fn send(&self, command: ModelCommand) {
        if let ModelCommand::SetAgentSettings { settings } = &command {
            self.memory_control.request(settings.memory_enabled);
            if !settings.memory_enabled {
                self.memory_enabled
                    .store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _ = self.commands.try_send(command);
    }
}

impl Drop for ModelsRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

enum Completed {
    Catalog {
        connection: ModelConnection,
        generation: u64,
        result: Result<Vec<ModelInfo>, ModelError>,
    },
    Details {
        key: ModelKey,
        generation: u64,
        // Boxed: ModelInfo dwarfs the other variants (clippy::large_enum_variant).
        result: Result<Box<ModelInfo>, ModelError>,
    },
    Test(Result<String, ModelError>),
    Pull {
        connection_id: String,
        generation: u64,
        result: Result<(), ModelError>,
    },
}

struct RunningPull {
    connection_id: String,
    generation: u64,
    task: AbortHandle,
}

type PullProgressChannel = (Sender<(u64, String)>, Receiver<(u64, String)>);

struct ModelActor {
    memory_control: crate::remembering::MemoryControl,
    repository: ModelRepository,
    providers: BTreeMap<ProviderKind, Arc<dyn ModelProvider>>,
    saved: SavedModels,
    catalogs: BTreeMap<String, CatalogState>,
    loaded: bool,
    generations: BTreeMap<String, u64>,
    refreshes: BTreeMap<String, AbortHandle>,
    jobs: JoinSet<Completed>,
    pull: Option<RunningPull>,
    pull_generation: u64,
    pull_progress: PullProgressChannel,
    agent: AgentConfig,
    execution: Arc<RwLock<ModelExecution>>,
    file_models: Arc<RwLock<BTreeMap<ModelPurpose, FileModel>>>,
    memory_enabled: Arc<std::sync::atomic::AtomicBool>,
    web_search_enabled: Arc<std::sync::atomic::AtomicBool>,
    events: Sender<Event>,
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ModelError> + Send + 'static,
) -> Result<T, ModelError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| ModelError("The settings operation could not complete.".into()))?
}

impl ModelActor {
    async fn publish_memory(&self) {
        let harness = match self.execution.read().await.clone() {
            ModelExecution::Legacy => crate::build_harness(&self.agent),
            ModelExecution::Selected { harness, .. } => Some(*harness),
            ModelExecution::Unavailable(_) => None,
        };
        self.memory_control.publish(
            self.saved.preferences.agent_settings.memory_enabled,
            harness,
        );
    }
    async fn emit(&self, event: ModelEvent) {
        let _ = self.events.send(Event::Models(event)).await;
    }

    async fn run(mut self, commands: Receiver<ModelCommand>) {
        if let Err(error) = self.load().await {
            self.emit(ModelEvent::Error(error.0)).await;
        }
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Ok(command) => if let Err(error) = self.command(command).await { self.emit(ModelEvent::Error(error.0)).await; },
                    Err(_) => break,
                },
                progress = self.pull_progress.1.recv() => {
                    if let Ok((generation, message)) = progress
                        && self.pull.as_ref().is_some_and(|pull| pull.generation == generation)
                    {
                        self.emit(ModelEvent::PullProgress(Some(message))).await;
                    }
                }
                completed = self.jobs.join_next(), if !self.jobs.is_empty() => {
                    if let Some(Ok(completed)) = completed { self.completed(completed).await; }
                }
            }
        }
        self.jobs.abort_all();
    }

    async fn load(&mut self) -> Result<(), ModelError> {
        let repository = self.repository.clone();
        self.saved = blocking(move || repository.load()).await?;
        self.memory_enabled.store(
            self.saved.preferences.agent_settings.memory_enabled,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.web_search_enabled.store(
            self.saved.preferences.agent_settings.web_search_enabled,
            std::sync::atomic::Ordering::Relaxed,
        );
        for index in 0..self.saved.connections.len() {
            let mut connection = validate_connection(&self.saved.connections[index])?;
            connection.environment_credential = self.repository.has_environment_key(&connection);
            match self.secret(&connection).await {
                Ok(secret) if connection.has_credential && secret.0.is_empty() => {
                    // The settings file says a key was saved, but this credential store
                    // has none (a reset keychain, or a dev build reading its separate
                    // dev store). Say so, so setup asks for the key instead of claiming
                    // to be ready and failing on the first message.
                    tracing::warn!(id = %connection.id, "saved key is missing from the credential store");
                    connection.has_credential = false;
                    connection.key_preview = self
                        .secret(&connection)
                        .await
                        .unwrap_or_default()
                        .masked_preview();
                }
                Ok(secret) => connection.key_preview = secret.masked_preview(),
                // A locked keychain must not block loading every other setting.
                Err(error) => {
                    tracing::warn!(id = %connection.id, %error, "could not read saved key");
                }
            }
            self.saved.connections[index] = connection;
        }
        self.loaded = true;
        self.publish().await;
        for connection in self.saved.connections.clone() {
            let repository = self.repository.clone();
            let id = connection.id.clone();
            if let Ok(Some(mut snapshot)) = blocking(move || repository.load_catalog(&id)).await {
                merge_manual(&mut snapshot.models, &connection.manual_models);
                self.catalogs.insert(
                    connection.id.clone(),
                    CatalogState {
                        snapshot: snapshot.clone(),
                        cached: true,
                        ..CatalogState::default()
                    },
                );
                self.emit(ModelEvent::CatalogLoaded {
                    connection_id: connection.id.clone(),
                    snapshot,
                    cached: true,
                })
                .await;
            }
            let fresh = self
                .catalogs
                .get(&connection.id)
                .is_some_and(|c| now().saturating_sub(c.snapshot.fetched_at) < 900);
            if (!fresh || connection.kind != ProviderKind::OpenRouter)
                && let Err(error) = self.refresh(&connection.id).await
            {
                self.catalogs
                    .entry(connection.id.clone())
                    .or_default()
                    .error = Some(error.0.clone());
                self.emit(ModelEvent::CatalogFailed {
                    connection_id: connection.id.clone(),
                    message: error.0,
                })
                .await;
            }
        }
        if let Some(key) = self.saved.preferences.active.clone() {
            // Restore from a cached/manual model while refreshing metadata in the background.
            if let Err(error) = self.activate(&key, false).await {
                self.mark_unavailable(error.0).await;
            }
        } else if self.agent.has_api_key() {
            *self.execution.write().await = ModelExecution::Legacy;
        } else {
            *self.execution.write().await = ModelExecution::Unavailable(
                "No AI model is active yet. Add an OpenRouter key in Settings, set up free local Ollama, or connect an OpenAI-compatible URL, then pick a model in Models.".into(),
            );
        }
        self.publish_memory().await;
        self.rebuild_file_models().await;
        Ok(())
    }

    /// The chosen model cannot answer: stop using it and tell the UI why, so setup
    /// asks for a new choice instead of showing "ready".
    async fn mark_unavailable(&self, message: String) {
        *self.execution.write().await = ModelExecution::Unavailable(message.clone());
        self.publish_memory().await;
        self.emit(ModelEvent::ActiveModelStatus {
            problem: Some(message),
        })
        .await;
    }

    async fn publish(&self) {
        self.emit(ModelEvent::Loaded {
            connections: self.saved.connections.clone(),
            preferences: self.saved.preferences.clone(),
        })
        .await;
    }

    fn connection(&self, id: &str) -> Result<ModelConnection, ModelError> {
        self.saved
            .connections
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| ModelError("This connection no longer exists.".into()))
    }

    fn provider(&self, kind: ProviderKind) -> Result<Arc<dyn ModelProvider>, ModelError> {
        self.providers
            .get(&kind)
            .cloned()
            .ok_or_else(|| ModelError("This provider protocol is unavailable.".into()))
    }

    async fn secret(&self, connection: &ModelConnection) -> Result<ModelSecret, ModelError> {
        let repository = self.repository.clone();
        let connection = connection.clone();
        blocking(move || repository.credential(&connection)).await
    }

    async fn persist(&self, saved: SavedModels) -> Result<(), ModelError> {
        let repository = self.repository.clone();
        blocking(move || repository.save(&saved)).await
    }

    async fn refresh(&mut self, id: &str) -> Result<(), ModelError> {
        let connection = self.connection(id)?;
        let provider = self.provider(connection.kind)?;
        let secret = self.secret(&connection).await?;
        if let Some(previous) = self.refreshes.remove(id) {
            previous.abort();
        }
        let generation = self.generations.entry(id.to_owned()).or_default();
        *generation += 1;
        let generation = *generation;
        let catalog = self.catalogs.entry(id.to_owned()).or_default();
        catalog.loading = true;
        catalog.error = None;
        self.emit(ModelEvent::CatalogLoading {
            connection_id: id.to_owned(),
        })
        .await;
        let job = self.jobs.spawn(async move {
            let result = provider.list(&connection, &secret).await;
            Completed::Catalog {
                connection,
                generation,
                result,
            }
        });
        self.refreshes.insert(id.to_owned(), job);
        Ok(())
    }

    async fn command(&mut self, command: ModelCommand) -> Result<(), ModelError> {
        let rebuild_files = matches!(
            &command,
            ModelCommand::SaveConnection { .. }
                | ModelCommand::DeleteConnection { .. }
                | ModelCommand::RemoveManualModel { .. }
                | ModelCommand::SetAgentSettings { .. }
        );
        if !self.loaded {
            if matches!(command, ModelCommand::Load) {
                return self.load().await;
            }
            return Err(ModelError(
                "Model settings could not be loaded. Retry loading before saving changes.".into(),
            ));
        }
        match command {
            ModelCommand::Load => {
                self.publish().await;
                for (id, catalog) in &self.catalogs {
                    self.emit(ModelEvent::CatalogLoaded {
                        connection_id: id.clone(),
                        snapshot: catalog.snapshot.clone(),
                        cached: catalog.cached,
                    })
                    .await;
                    if catalog.loading {
                        self.emit(ModelEvent::CatalogLoading {
                            connection_id: id.clone(),
                        })
                        .await;
                    }
                    if let Some(message) = &catalog.error {
                        self.emit(ModelEvent::CatalogFailed {
                            connection_id: id.clone(),
                            message: message.clone(),
                        })
                        .await;
                    }
                }
            }
            ModelCommand::Refresh { connection_id } => self.refresh(&connection_id).await?,
            ModelCommand::SaveConnection {
                connection,
                credential,
            } => self.save_connection(connection, credential).await?,
            ModelCommand::TestConnection {
                connection,
                credential,
            } => {
                let connection = validate_connection(&connection)?;
                let secret = self.draft_secret(&connection, credential).await?;
                let provider = self.provider(connection.kind)?;
                self.emit(ModelEvent::OperationStarted).await;
                self.jobs.spawn(async move {
                    Completed::Test(sovra_providers::test_connection(provider.as_ref(), &connection, &secret).await.map(|count| format!("It works! Sovra can reach this service · {count} AI models available.")))
                });
            }
            ModelCommand::DeleteConnection { id } => self.delete_connection(&id).await?,
            ModelCommand::AddManualModel {
                connection_id,
                mut model,
            } => {
                let trimmed_id = model.id.trim().to_owned();
                model.id = trimmed_id;
                if model.id.is_empty()
                    || model.id.len() > 256
                    || model.id.chars().any(char::is_control)
                {
                    return Err(ModelError(
                        "Enter the exact model ID accepted by your server.".into(),
                    ));
                }
                let trimmed_name = model.name.trim().to_owned();
                model.name = trimmed_name;
                if model.name.is_empty() {
                    model.name.clone_from(&model.id);
                }
                model.manual = true;
                model.details_loaded = true;
                let mut connection = self.connection(&connection_id)?;
                if connection.kind == ProviderKind::OpenRouter {
                    return Err(ModelError(
                        "OpenRouter models come from its live catalog.".into(),
                    ));
                }
                connection.manual_models.retain(|m| m.id != model.id);
                connection.manual_models.push(*model);
                self.save_connection(connection, CredentialChange::Keep)
                    .await?;
            }
            ModelCommand::RemoveManualModel { key } => {
                let mut connection = self.connection(&key.connection_id)?;
                connection.manual_models.retain(|m| m.id != key.model_id);
                self.save_connection(connection, CredentialChange::Keep)
                    .await?;
            }
            ModelCommand::SetFavorite { key, favorite } => {
                let mut saved = self.saved.clone();
                if favorite {
                    saved.preferences.favorites.insert(key);
                } else {
                    saved.preferences.favorites.remove(&key);
                }
                self.persist(saved.clone()).await?;
                self.saved = saved;
                self.emit(ModelEvent::PreferencesChanged(
                    self.saved.preferences.clone(),
                ))
                .await;
            }
            ModelCommand::SetAppearance { appearance } => {
                let mut saved = self.saved.clone();
                saved.preferences.appearance = appearance;
                self.persist(saved.clone()).await?;
                self.saved = saved;
                self.emit(ModelEvent::PreferencesChanged(
                    self.saved.preferences.clone(),
                ))
                .await;
            }
            ModelCommand::SetInterfaceScale { scale } => {
                let mut saved = self.saved.clone();
                saved.preferences.interface_scale = scale;
                self.persist(saved.clone()).await?;
                self.saved = saved;
                self.emit(ModelEvent::PreferencesChanged(
                    self.saved.preferences.clone(),
                ))
                .await;
            }
            ModelCommand::SetAgentSettings { settings } => {
                let mut saved = self.saved.clone();
                saved.preferences.agent_settings = settings;
                self.persist(saved.clone()).await?;
                self.saved = saved;
                self.file_models.write().await.clear();
                self.memory_enabled.store(
                    self.saved.preferences.agent_settings.memory_enabled,
                    std::sync::atomic::Ordering::Relaxed,
                );
                self.web_search_enabled.store(
                    self.saved.preferences.agent_settings.web_search_enabled,
                    std::sync::atomic::Ordering::Relaxed,
                );
                self.publish_memory().await;
                if let Some(active) = self.saved.preferences.active.clone() {
                    let _ = self.activate(&active, false).await;
                }
                self.emit(ModelEvent::PreferencesChanged(
                    self.saved.preferences.clone(),
                ))
                .await;
                self.emit(ModelEvent::Notice(
                    "Saved your Personal AI instructions and behavior settings.".into(),
                ))
                .await;
            }
            ModelCommand::SetOnboardingCompleted { completed } => {
                let mut saved = self.saved.clone();
                saved.preferences.onboarding_completed = completed;
                self.persist(saved.clone()).await?;
                self.saved = saved;
                self.emit(ModelEvent::PreferencesChanged(
                    self.saved.preferences.clone(),
                ))
                .await;
            }
            ModelCommand::Activate { key } => self.activate(&key, true).await?,
            ModelCommand::AssignPurpose { purpose, key } => {
                self.assign_purpose(purpose, key).await?;
            }
            ModelCommand::FetchDetails { key } => {
                let connection = self.connection(&key.connection_id)?;
                let model = self.find_model(&key)?;
                if !model.details_loaded {
                    let provider = self.provider(connection.kind)?;
                    let secret = self.secret(&connection).await?;
                    let generation = *self.generations.get(&connection.id).unwrap_or(&0);
                    self.jobs.spawn(async move {
                        Completed::Details {
                            key,
                            generation,
                            result: provider
                                .details(&connection, &secret, &model)
                                .await
                                .map(Box::new),
                        }
                    });
                }
            }
            ModelCommand::PullOllama {
                connection_id,
                model,
            } => {
                if self.pull.is_some() {
                    return Err(ModelError("An Ollama download is already running.".into()));
                }
                let connection = self.connection(&connection_id)?;
                if connection.kind != ProviderKind::Ollama {
                    return Err(ModelError("Downloads require an Ollama connection.".into()));
                }
                let secret = self.secret(&connection).await?;
                let progress = self.pull_progress.0.clone();
                self.pull_generation += 1;
                let generation = self.pull_generation;
                let label = format!("{} · {model}", connection.name);
                self.emit(ModelEvent::PullProgress(Some(format!(
                    "{label} · Connecting to Ollama…"
                ))))
                .await;
                let id = connection_id.clone();
                let task = self.jobs.spawn(async move {
                    let result = sovra_providers::pull_ollama(
                        &connection,
                        &secret,
                        &model,
                        move |message| {
                            let progress = progress.clone();
                            let message = format!("{label} · {message}");
                            Box::pin(async move {
                                let _ = progress.send((generation, message)).await;
                            })
                        },
                    )
                    .await;
                    Completed::Pull {
                        connection_id,
                        generation,
                        result,
                    }
                });
                self.pull = Some(RunningPull {
                    connection_id: id,
                    generation,
                    task,
                });
            }
            ModelCommand::CancelPull => {
                if let Some(pull) = self.pull.take() {
                    pull.task.abort();
                }
                self.emit(ModelEvent::PullProgress(None)).await;
                self.emit(ModelEvent::Notice(
                    "Download connection closed. Ollama may keep partial layers for a later retry."
                        .into(),
                ))
                .await;
            }
        }
        if rebuild_files {
            self.publish_memory().await;
            self.rebuild_file_models().await;
        }
        Ok(())
    }

    fn find_model(&self, key: &ModelKey) -> Result<ModelInfo, ModelError> {
        self.catalogs.get(&key.connection_id).and_then(|c| c.snapshot.models.iter().find(|m| m.id == key.model_id)).cloned()
            .or_else(|| self.saved.connections.iter().find(|c| c.id == key.connection_id)?.manual_models.iter().find(|m| m.id == key.model_id).cloned())
            .ok_or_else(|| ModelError("This model is no longer in the catalog. Refresh the connection and select a model.".into()))
    }

    async fn configure_model(
        &self,
        key: &ModelKey,
    ) -> Result<(AdkHarness, ModelInfo, ModelConnection), ModelError> {
        let connection = self.connection(&key.connection_id)?;
        let model = self.find_model(key)?;
        if !model.can_chat() {
            return Err(ModelError("This model produces a non-chat output. Its information is available in the catalog, but it cannot be used as the chat assistant.".into()));
        }
        let secret = self.secret(&connection).await?;
        if connection.authentication != Authentication::None && secret.0.is_empty() {
            return Err(ModelError(format!(
                "Add an API key to {} before using this model.",
                connection.name
            )));
        }
        let mut effective_agent = self.agent.clone();
        effective_agent.instruction = self
            .saved
            .preferences
            .agent_settings
            .compose_system_instruction(&self.agent.instruction);
        let mut effective_model = model.clone();
        if !self.saved.preferences.agent_settings.tools_enabled {
            effective_model
                .supported_parameters
                .retain(|p| p != "tools");
        }
        let harness =
            AdkHarness::from_selected_model(&effective_agent, &connection, &effective_model, &secret)
                .map_err(|_| {
                    ModelError(
                        "Could not configure this model. Check its connection URL and authentication settings."
                            .into(),
                    )
                })?;
        Ok((harness, effective_model, connection))
    }

    async fn activate(&mut self, key: &ModelKey, persist: bool) -> Result<(), ModelError> {
        let (harness, model, connection) = self.configure_model(key).await?;
        if persist {
            let mut saved = self.saved.clone();
            saved.preferences.active = Some(key.clone());
            self.persist(saved.clone()).await?;
            self.saved = saved;
        }
        *self.execution.write().await = ModelExecution::Selected {
            harness: Box::new(harness),
            timeout: Duration::from_secs(connection.timeout_seconds),
        };
        self.publish_memory().await;
        self.rebuild_file_models().await;
        self.emit(ModelEvent::PreferencesChanged(
            self.saved.preferences.clone(),
        ))
        .await;
        self.emit(ModelEvent::ActiveModelStatus { problem: None })
            .await;
        if persist {
            self.emit(ModelEvent::Notice(format!(
                "{} is now the chat model. New model sessions start with fresh provider history.",
                model.name
            )))
            .await;
        }
        Ok(())
    }

    async fn rebuild_file_models(&self) {
        let previous = self.file_models.read().await.clone();
        let mut executions = BTreeMap::new();
        for purpose in ModelPurpose::ALL {
            let key = self.saved.preferences.purposes.get(&purpose).or_else(|| {
                (purpose == ModelPurpose::Documents)
                    .then_some(self.saved.preferences.active.as_ref())
                    .flatten()
            });
            let Some(key) = key else {
                continue;
            };
            if let Ok((harness, model, connection)) = self.configure_model(key).await
                && purpose.accepts(&model, connection.kind)
            {
                executions.insert(
                    purpose,
                    FileModel {
                        key: key.clone(),
                        model: model.clone(),
                        provider: connection.kind,
                        execution: if self.saved.preferences.active.as_ref() == Some(key) {
                            self.execution.read().await.clone()
                        } else if let Some(prior) = previous.get(&purpose).filter(|prior| {
                            prior.key == *key
                                && prior.model == model
                                && prior.provider == connection.kind
                        }) {
                            prior.execution.clone()
                        } else {
                            ModelExecution::Selected {
                                harness: Box::new(harness),
                                timeout: Duration::from_secs(connection.timeout_seconds),
                            }
                        },
                    },
                );
            }
        }
        *self.file_models.write().await = executions;
    }

    async fn assign_purpose(
        &mut self,
        purpose: ModelPurpose,
        key: Option<ModelKey>,
    ) -> Result<(), ModelError> {
        if let Some(key) = &key {
            let (_, model, connection) = self.configure_model(key).await?;
            if !purpose.accepts(&model, connection.kind) {
                return Err(ModelError(format!(
                    "Choose a model that can {}. This model or connection cannot handle that kind of input.",
                    purpose.label().to_lowercase()
                )));
            }
        }
        let mut saved = self.saved.clone();
        match key {
            Some(key) => {
                saved.preferences.purposes.insert(purpose, key);
            }
            None => {
                saved.preferences.purposes.remove(&purpose);
            }
        }
        self.persist(saved.clone()).await?;
        self.saved = saved;
        self.publish_memory().await;
        self.rebuild_file_models().await;
        self.emit(ModelEvent::PreferencesChanged(
            self.saved.preferences.clone(),
        ))
        .await;
        self.emit(ModelEvent::Notice(format!(
            "Saved: {}. Your chat and attachments are waiting for you.",
            purpose.label()
        )))
        .await;
        Ok(())
    }

    async fn draft_secret(
        &self,
        connection: &ModelConnection,
        credential: CredentialChange,
    ) -> Result<ModelSecret, ModelError> {
        match credential {
            CredentialChange::Replace(secret) => {
                if secret.0.chars().any(char::is_control) {
                    return Err(ModelError(
                        "API keys cannot contain control characters.".into(),
                    ));
                }
                Ok(ModelSecret(secret.0.trim().to_owned()))
            }
            CredentialChange::Remove => Ok(ModelSecret::default()),
            CredentialChange::Keep => {
                if let Ok(existing) = self.connection(&connection.id) {
                    if existing.kind != connection.kind
                        || existing.base_url != connection.base_url
                        || existing.authentication != connection.authentication
                        || existing.api_key_header != connection.api_key_header
                    {
                        if existing.has_credential || existing.environment_credential {
                            return Err(ModelError("Re-enter the API key for the new destination, or clear the saved key. Existing credentials are not forwarded to a changed server.".into()));
                        }
                        return Ok(ModelSecret::default());
                    }
                    self.secret(&existing).await
                } else {
                    Ok(ModelSecret::default())
                }
            }
        }
    }

    async fn save_connection(
        &mut self,
        connection: ModelConnection,
        change: CredentialChange,
    ) -> Result<(), ModelError> {
        let mut connection = validate_connection(&connection)?;
        let secret = self.draft_secret(&connection, change.clone()).await?;
        if matches!(change, CredentialChange::Replace(_)) {
            sovra_providers::verify_key(&connection, &secret).await?;
        }
        let old = self
            .saved
            .connections
            .iter()
            .find(|c| c.id == connection.id)
            .cloned();
        let old_secret = if let Some(old) = &old {
            self.secret(old).await?
        } else {
            ModelSecret::default()
        };
        connection.has_credential = match &change {
            CredentialChange::Keep => old.as_ref().is_some_and(|c| c.has_credential),
            CredentialChange::Replace(_) => !secret.0.is_empty(),
            CredentialChange::Remove => false,
        };
        connection.environment_credential = self.repository.has_environment_key(&connection);
        let preview_secret = if !secret.0.is_empty() {
            secret.clone()
        } else if connection.environment_credential {
            self.secret(&connection).await.unwrap_or_default()
        } else {
            ModelSecret::default()
        };
        connection.key_preview = preview_secret.masked_preview();
        let mut saved = self.saved.clone();
        if let Some(existing) = saved.connections.iter_mut().find(|c| c.id == connection.id) {
            *existing = connection.clone();
        } else {
            saved.connections.push(connection.clone());
        }
        let repository = self.repository.clone();
        let id = connection.id.clone();
        let rollback = old.clone();
        let saved_copy = saved.clone();
        blocking(move || {
            if !matches!(change, CredentialChange::Keep) {
                if secret.0.is_empty() {
                    repository.delete_credential(&id)?;
                } else {
                    repository.set_credential(&id, &secret)?;
                }
            }
            if let Err(error) = repository.save(&saved_copy) {
                if !matches!(change, CredentialChange::Keep) {
                    if rollback.is_some_and(|c| c.has_credential) {
                        let _ = repository.set_credential(&id, &old_secret);
                    } else {
                        let _ = repository.delete_credential(&id);
                    }
                }
                return Err(error);
            }
            Ok(())
        })
        .await?;
        self.saved = saved;
        self.file_models.write().await.clear();
        if let Some(job) = self.refreshes.remove(&connection.id) {
            job.abort();
        }
        *self.generations.entry(connection.id.clone()).or_default() += 1;
        self.cancel_connection_pull(&connection.id).await;
        // Invalidate results for a changed destination before publishing new metadata.
        if old
            .as_ref()
            .is_some_and(|c| c.base_url != connection.base_url || c.kind != connection.kind)
        {
            self.catalogs.remove(&connection.id);
            let repository = self.repository.clone();
            let id = connection.id.clone();
            blocking(move || repository.delete_catalog(&id)).await?;
        }
        let catalog = self.catalogs.entry(connection.id.clone()).or_default();
        catalog.loading = false;
        catalog.error = None;
        catalog.snapshot.models.retain(|m| !m.manual);
        merge_manual(&mut catalog.snapshot.models, &connection.manual_models);
        let snapshot = catalog.snapshot.clone();
        let cached = catalog.cached;
        // Re-activate before announcing the saved key: the UI may send a waiting
        // message the moment it sees the key, and that must reach a ready model.
        if let Some(active) = self.saved.preferences.active.clone()
            && active.connection_id == connection.id
            && let Err(error) = self.activate(&active, false).await
        {
            self.mark_unavailable(error.0).await;
        }
        self.publish().await;
        self.emit(ModelEvent::CatalogLoaded {
            connection_id: connection.id.clone(),
            snapshot,
            cached,
        })
        .await;
        self.emit(ModelEvent::ConnectionSaved {
            id: connection.id.clone(),
        })
        .await;
        self.emit(ModelEvent::Notice(
            "Saved. Any key is kept safely in this computer's keychain.".into(),
        ))
        .await;
        self.refresh(&connection.id).await
    }

    async fn delete_connection(&mut self, id: &str) -> Result<(), ModelError> {
        let mut saved = self.saved.clone();
        saved.connections.retain(|c| c.id != id);
        saved
            .preferences
            .purposes
            .retain(|_, key| key.connection_id != id);
        saved
            .preferences
            .favorites
            .retain(|k| k.connection_id != id);
        let removed_active = saved
            .preferences
            .active
            .as_ref()
            .is_some_and(|k| k.connection_id == id);
        if removed_active {
            saved.preferences.active = None;
        }
        self.persist(saved.clone()).await?;
        self.saved = saved;
        if removed_active {
            *self.execution.write().await = ModelExecution::Unavailable(
                "The active connection was removed. Choose another model in Models.".into(),
            );
        }
        self.publish_memory().await;
        if let Some(job) = self.refreshes.remove(id) {
            job.abort();
        }
        self.generations.remove(id);
        self.cancel_connection_pull(id).await;
        self.catalogs.remove(id);
        let repository = self.repository.clone();
        let id_owned = id.to_owned();
        let cleanup = blocking(move || {
            repository.delete_catalog(&id_owned)?;
            repository.delete_credential(&id_owned)
        })
        .await;
        self.emit(ModelEvent::ConnectionDeleted { id: id.to_owned() })
            .await;
        self.publish().await;
        cleanup?;
        self.emit(ModelEvent::Notice(
            "Connection removed. Nothing was deleted from the service itself.".into(),
        ))
        .await;
        Ok(())
    }

    async fn cancel_connection_pull(&mut self, id: &str) {
        if self
            .pull
            .as_ref()
            .is_some_and(|pull| pull.connection_id == id)
        {
            if let Some(pull) = self.pull.take() {
                pull.task.abort();
            }
            self.emit(ModelEvent::PullProgress(None)).await;
        }
    }

    async fn completed(&mut self, completed: Completed) {
        match completed {
            Completed::Catalog {
                connection,
                generation,
                result,
            } => {
                if self.generations.get(&connection.id) != Some(&generation) {
                    return;
                }
                self.refreshes.remove(&connection.id);
                match result {
                    Ok(mut models) => {
                        merge_manual(&mut models, &connection.manual_models);
                        let snapshot = CatalogSnapshot {
                            models,
                            fetched_at: now(),
                        };
                        self.catalogs.insert(
                            connection.id.clone(),
                            CatalogState {
                                snapshot: snapshot.clone(),
                                ..CatalogState::default()
                            },
                        );
                        self.emit(ModelEvent::CatalogLoaded {
                            connection_id: connection.id.clone(),
                            snapshot: snapshot.clone(),
                            cached: false,
                        })
                        .await;
                        let repository = self.repository.clone();
                        let id = connection.id.clone();
                        if let Err(error) =
                            blocking(move || repository.save_catalog(&id, &snapshot)).await
                        {
                            self.emit(ModelEvent::Error(error.0)).await;
                        }
                        if let Some(active) = self.saved.preferences.active.clone()
                            && active.connection_id == connection.id
                        {
                            let unavailable = matches!(
                                *self.execution.read().await,
                                ModelExecution::Unavailable(_)
                            );
                            if self.find_model(&active).is_err() {
                                self.mark_unavailable(
                                    "it is no longer offered by this service.".to_owned(),
                                )
                                .await;
                            } else if unavailable
                                && let Err(error) = self.activate(&active, false).await
                            {
                                self.mark_unavailable(error.0).await;
                            }
                        }
                    }
                    Err(error) => {
                        let catalog = self.catalogs.entry(connection.id.clone()).or_default();
                        catalog.loading = false;
                        catalog.error = Some(error.0.clone());
                        self.emit(ModelEvent::CatalogFailed {
                            connection_id: connection.id,
                            message: error.0,
                        })
                        .await;
                    }
                }
            }
            Completed::Details {
                key,
                generation,
                result,
            } => {
                if self
                    .generations
                    .get(&key.connection_id)
                    .copied()
                    .unwrap_or(0)
                    != generation
                    || self.connection(&key.connection_id).is_err()
                {
                    return;
                }
                match result {
                    Ok(model) => {
                        if let Some(snapshot) = self.catalogs.get_mut(&key.connection_id)
                            && let Some(existing) = snapshot
                                .snapshot
                                .models
                                .iter_mut()
                                .find(|m| m.id == key.model_id)
                        {
                            *existing = (*model).clone();
                        }
                        // Background consumers resolve capabilities from the saved
                        // catalogue. Publish the choice only after saving its details.
                        if let Some(catalog) = self.catalogs.get(&key.connection_id) {
                            let snapshot = catalog.snapshot.clone();
                            let id = key.connection_id.clone();
                            let repository = self.repository.clone();
                            if let Err(error) =
                                blocking(move || repository.save_catalog(&id, &snapshot)).await
                            {
                                self.emit(ModelEvent::Error(error.0)).await;
                                return;
                            }
                        }
                        self.emit(ModelEvent::DetailsLoaded { key, model }).await;
                    }
                    Err(error) => self.emit(ModelEvent::Error(error.0)).await,
                }
            }
            Completed::Test(result) => match result {
                Ok(message) => self.emit(ModelEvent::Notice(message)).await,
                Err(error) => self.emit(ModelEvent::Error(error.0)).await,
            },
            Completed::Pull {
                connection_id,
                generation,
                result,
            } => {
                if self
                    .pull
                    .as_ref()
                    .is_none_or(|pull| pull.generation != generation)
                {
                    return;
                }
                // The completion and progress channels can become ready together.
                // Deliver queued progress for this download before clearing it.
                while let Ok((progress_generation, message)) = self.pull_progress.1.try_recv() {
                    if progress_generation == generation {
                        self.emit(ModelEvent::PullProgress(Some(message))).await;
                    }
                }
                self.pull = None;
                self.emit(ModelEvent::PullProgress(None)).await;
                match result {
                    Ok(()) => {
                        self.emit(ModelEvent::Notice(
                            "Model downloaded. Refreshing Ollama…".into(),
                        ))
                        .await;
                        if let Err(error) = self.refresh(&connection_id).await {
                            self.emit(ModelEvent::Error(error.0)).await;
                        }
                    }
                    Err(error) => self.emit(ModelEvent::Error(error.0)).await,
                }
            }
        }
        self.publish_memory().await;
        self.rebuild_file_models().await;
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn merge_manual(models: &mut Vec<ModelInfo>, manual: &[ModelInfo]) {
    for model in manual {
        models.retain(|m| m.id != model.id);
        models.push(model.clone());
    }
}
