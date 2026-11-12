//! Owned output capture: durable source receipts without interpreting helper text as facts.

use std::sync::{Arc, Mutex};

use async_channel::Sender;
use sovra_core::{
    EpisodeKind, Event, MAX_EPISODE_BYTES, MemoryEvent, MemorySourceAdmission, MemorySourceAuthor,
    MemorySourceProvenance, MemorySourceScope, MemoryStatus, MemoryStore, NewEpisode,
    TeamMemoryCapture,
};

use crate::remembering::{MemoryControl, source_key};

pub(crate) struct Capture {
    pub(crate) inputs: Arc<Inputs>,
    saved: Option<tokio::sync::oneshot::Receiver<sovra_core::EpisodeId>>,
    reply_to: Option<String>,
}

impl Capture {
    pub(crate) fn new(
        context: Option<&sovra_core::ConversationContext>,
        saved: Option<tokio::sync::oneshot::Receiver<sovra_core::EpisodeId>>,
    ) -> Self {
        Self {
            inputs: Arc::new(Inputs(Mutex::new(
                sovra_core::MemoryDependencyTracker::new(context),
            ))),
            saved,
            reply_to: context
                .and_then(sovra_core::ConversationContext::current)
                .map(|input| input.id.clone()),
        }
    }
}

pub(crate) struct Inputs(Mutex<sovra_core::MemoryDependencyTracker>);
impl Inputs {
    fn state(&self) -> std::sync::MutexGuard<'_, sovra_core::MemoryDependencyTracker> {
        self.0.lock().unwrap_or_else(|error| {
            let mut state = error.into_inner();
            state.untracked();
            state
        })
    }
    pub(crate) fn reader(
        self: &Arc<Self>,
        reader: Option<Arc<dyn sovra_core::MemoryReader>>,
    ) -> Option<Arc<dyn sovra_core::MemoryReader>> {
        reader.map(|reader| {
            Arc::new(TrackedReader {
                reader,
                inputs: self.clone(),
            }) as Arc<dyn sovra_core::MemoryReader>
        })
    }
    pub(crate) fn untracked(&self) {
        self.state().untracked();
    }
}
impl sovra_core::ReplyInputObserver for Inputs {
    fn model_input(&self, untracked: bool) {
        self.state().model_input(untracked);
    }
}

struct TrackedReader {
    reader: Arc<dyn sovra_core::MemoryReader>,
    inputs: Arc<Inputs>,
}
#[async_trait::async_trait]
impl sovra_core::MemoryReader for TrackedReader {
    async fn search(
        &self,
        query: sovra_core::MemorySearch,
    ) -> Result<sovra_core::MemoryContext, sovra_core::MemoryError> {
        let context = self.reader.search(query).await?;
        self.inputs.state().memory(&context);
        Ok(context)
    }
    async fn search_batch(
        &self,
        queries: Vec<sovra_core::MemorySearch>,
    ) -> Result<Vec<sovra_core::MemoryContext>, sovra_core::MemoryError> {
        let contexts = self.reader.search_batch(queries).await?;
        for context in &contexts {
            self.inputs.state().memory(context);
        }
        Ok(contexts)
    }
}

pub(crate) fn tap(
    tasks: &mut tokio::task::JoinSet<()>,
    store: Arc<dyn MemoryStore>,
    events: Sender<Event>,
    control: MemoryControl,
    mut input: Capture,
) -> Sender<Event> {
    let (tapped, stream) = async_channel::bounded::<Event>(64);
    let epoch = control.pause_epoch();
    tasks.spawn(async move {
        let policy = match store.memory_policy().await {
            Ok(policy) => Some(policy),
            Err(error) => {
                tracing::warn!(%error, "could not authorize reply capture");
                let _ = events.send(failed()).await;
                None
            }
        };
        let mut capture = policy.as_ref().is_some_and(|policy| policy.enabled) && control.enabled();
        let mut transcript = String::new();
        let mut team = TeamMemoryCapture::default();
        let mut resolved_input = false;
        while let Ok(event) = stream.recv().await {
            if capture && (!control.enabled() || control.pause_epoch() != epoch) {
                capture = false;
                transcript.clear();
                team = TeamMemoryCapture::default();
            }
            let source = if capture {
                match &event {
                    Event::AgentChunk { text, .. } => {
                        if transcript.len().saturating_add(text.len()) > MAX_EPISODE_BYTES {
                            transcript.clear();
                            capture = false;
                            Err(sovra_core::MemoryError::InvalidInput(
                                "Reply exceeds the source size limit.".into(),
                            ))
                        } else {
                            transcript.push_str(text);
                            Ok(None)
                        }
                    }
                    Event::AgentCompleted { session_id } if !transcript.trim().is_empty() => {
                        Ok(Some(MemorySourceAdmission {
                            dependencies: None,
                            expected_generation: None,
                            key: source_key(),
                            version: 1,
                            author: MemorySourceAuthor::Assistant { speaker: None },
                            scope: MemorySourceScope::Private,
                            provenance: MemorySourceProvenance::Conversation,
                            episode: NewEpisode {
                                session_id: session_id.clone(),
                                kind: EpisodeKind::AgentMessage,
                                content: std::mem::take(&mut transcript),
                                occurred_at: crate::now(),
                            },
                        }))
                    }
                    Event::AgUi { session_id, event } => {
                        team.apply(session_id, event, crate::now())
                    }
                    // Only the public reply protocols carry admissible evidence.
                    Event::Auth(_)
                    | Event::Memory(_)
                    | Event::Models(_)
                    | Event::Skills(_)
                    | Event::Teams(_)
                    | Event::Artifacts(_)
                    | Event::Connections(_)
                    | Event::RemoteControl(_)
                    | Event::Development(_)
                    | Event::Google(_)
                    | Event::GitHub(_)
                    | Event::Mcp(_)
                    | Event::Terminal(_)
                    | Event::Schedule(_)
                    | Event::Group(_)
                    | Event::FilePrepared(_)
                    | Event::FileIndex(_)
                    | Event::ChatHistoryLoaded { .. }
                    | Event::Ready { .. }
                    | Event::Pong
                    | Event::RouteEvaluated { .. }
                    | Event::AgentThinkingChunk { .. }
                    | Event::AgentResponseStarted { .. }
                    | Event::AgUiSurface { .. }
                    | Event::AgentError { .. }
                    | Event::ChatTitleGenerated { .. }
                    | Event::Stopped
                    | Event::AgentCompleted { .. } => Ok(None),
                }
            } else {
                Ok(None)
            };
            match source {
                Ok(Some(mut source)) => {
                    if !resolved_input {
                        let saved = match input.saved.take() {
                            Some(receipt) => {
                                tokio::time::timeout(std::time::Duration::from_secs(2), receipt)
                                    .await
                                    .ok()
                                    .and_then(Result::ok)
                            }
                            None => None,
                        };
                        match saved {
                            Some(source) => input.inputs.state().source(source),
                            None => input.inputs.untracked(),
                        }
                        resolved_input = true;
                    }
                    source.dependencies = input.inputs.state().snapshot();
                    tracing::debug!(
                        complete = source.dependencies.is_some(),
                        sources = source
                            .dependencies
                            .as_ref()
                            .map_or(0, |inputs| inputs.sources.len()),
                        facts = source
                            .dependencies
                            .as_ref()
                            .map_or(0, |inputs| inputs.facts.len()),
                        "reply input tracking completed"
                    );
                    source.expected_generation = policy.as_ref().map(|policy| policy.generation);
                    let session_id = source.episode.session_id.clone();
                    let message = Some(sovra_core::MemorySourceMessage {
                        id: match &event {
                            Event::AgUi {
                                event: sovra_core::AgUiEvent::TextMessageEnd { message_id },
                                ..
                            } => Some(message_id.clone()),
                            _ => None,
                        },
                        in_reply_to: input.reply_to.clone(),
                        role: sovra_core::MessageRole::Assistant,
                        text: source.episode.content.clone(),
                    });
                    let result = match store.admit_memory_source(source).await {
                        Ok(receipt) => {
                            input.inputs.state().source(receipt.source_id.clone());
                            Event::Memory(MemoryEvent::SourceSaved {
                                session_id,
                                message,
                                receipt,
                            })
                        }
                        Err(error) => {
                            input.inputs.untracked();
                            tracing::warn!(%error, "failed to save assistant evidence");
                            failed()
                        }
                    };
                    if events.send(result).await.is_err() {
                        return;
                    }
                }
                Err(error) => {
                    input.inputs.untracked();
                    tracing::warn!(%error, "failed to assemble assistant evidence");
                    if events.send(failed()).await.is_err() {
                        return;
                    }
                }
                Ok(None) => {}
            }
            if events.send(event).await.is_err() {
                return;
            }
        }
    });
    tapped
}

fn failed() -> Event {
    Event::Memory(MemoryEvent::Status(MemoryStatus::Failed(
        "This reply could not be saved in memory.".into(),
    )))
}
