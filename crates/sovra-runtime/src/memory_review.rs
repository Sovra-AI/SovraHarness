//! Serialized, bounded user memory actions. Shutdown drains accepted edits.

use std::sync::Arc;

use async_channel::Sender;
use sovra_core::{
    Event, MemoryAction, MemoryCommand, MemoryDetail, MemoryError, MemoryEvent, MemoryReviewEvent,
    MemoryReviewResult, MemoryStore,
};

pub(crate) struct MemoryReviewRuntime {
    commands: Sender<(MemoryCommand, u64)>,
    events: Sender<Event>,
    task: tokio::task::JoinHandle<()>,
    semantics: Option<Arc<crate::memory_semantic::SemanticMemory>>,
}

impl MemoryReviewRuntime {
    pub(crate) fn start(
        store: Option<Arc<dyn MemoryStore>>,
        control: crate::remembering::MemoryControl,
        semantics: Option<Arc<crate::memory_semantic::SemanticMemory>>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (commands, receiver) = async_channel::bounded::<(MemoryCommand, u64)>(16);
        let output = events.clone();
        let semantic_service = semantics.clone();
        let task = handle.spawn(async move {
            while let Ok((command, token)) = receiver.recv().await {
                let result = match &store {
                    Some(store) => {
                        execute(
                            store.as_ref(),
                            semantic_service.as_deref(),
                            command.action,
                            token,
                            &control,
                        )
                        .await
                    }
                    None => Err(MemoryError::Backend(
                        "Memory is unavailable in this session.".into(),
                    )),
                };
                if matches!(result, Ok(MemoryReviewResult::Changed(_))) {
                    control.invalidate_recall();
                }
                let _ = output
                    .send(Event::Memory(MemoryEvent::Review(MemoryReviewEvent {
                        request_id: command.request_id,
                        result: result.map_err(|error| error.to_string()),
                    })))
                    .await;
            }
        });
        Self {
            commands,
            events,
            task,
            semantics,
        }
    }

    pub(crate) fn send(&self, command: MemoryCommand) {
        let token = if matches!(command.action, MemoryAction::ConfigureSemantic(_)) {
            self.semantics
                .as_ref()
                .map_or(0, |service| service.revoke())
        } else {
            0
        };
        let request_id = command.request_id;
        if self.commands.try_send((command, token)).is_err() {
            let _ = self
                .events
                .try_send(Event::Memory(MemoryEvent::Review(MemoryReviewEvent {
                    request_id,
                    result: Err("Too many memory requests. Wait briefly and retry.".into()),
                })));
        }
    }

    pub(crate) async fn stop(&mut self) {
        self.commands.close();
        let _ = (&mut self.task).await;
    }
}

impl Drop for MemoryReviewRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn execute(
    store: &dyn MemoryStore,
    semantics: Option<&crate::memory_semantic::SemanticMemory>,
    action: MemoryAction,
    token: u64,
    control: &crate::remembering::MemoryControl,
) -> Result<MemoryReviewResult, MemoryError> {
    match action {
        MemoryAction::ConfigureSemantic(target) => {
            let service = semantics.ok_or_else(|| {
                MemoryError::Backend("Set up a search helper in Models first.".into())
            })?;
            service
                .configure(target, token)
                .await
                .map(MemoryReviewResult::Semantics)
        }
        MemoryAction::List(query) => store
            .list_memories(query)
            .await
            .map(MemoryReviewResult::Page),
        MemoryAction::Inspect(id) => {
            let fact = store.memory_fact(&id).await?;
            let source = match &fact.source {
                Some(id) => Some(store.episode(id).await?),
                None => None,
            };
            Ok(MemoryReviewResult::Detail(Box::new(MemoryDetail {
                fact,
                source,
            })))
        }
        MemoryAction::Correct(correction) => store
            .correct_memory(correction)
            .await
            .map(MemoryReviewResult::Changed),
        MemoryAction::Remember(note) => {
            note.validate()?;
            store
                .remember_note(note)
                .await
                .map(MemoryReviewResult::Changed)
        }
        MemoryAction::RetryFailed => {
            if !control.enabled() {
                return Err(MemoryError::InvalidInput(
                    "Turn on remembering before retrying messages.".into(),
                ));
            }
            store
                .retry_failed_memories()
                .await
                .map(MemoryReviewResult::Retried)
        }
        MemoryAction::StopUsing(id) => {
            store.stop_memory(&id, super::now()).await?;
            Ok(MemoryReviewResult::Changed(id))
        }
    }
}
