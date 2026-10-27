//! One ordered owner for chat snapshots, soft deletion and reload barriers.

use std::sync::Arc;

use async_channel::Sender;
use sovra_core::{ChatThread, Event, MemoryStore, Timestamp};

enum Action {
    Load,
    Save(Box<ChatThread>),
    Delete { session: String, at: Timestamp },
}

pub(crate) struct ChatHistoryRuntime {
    commands: Sender<Action>,
    task: tokio::task::JoinHandle<()>,
}

impl ChatHistoryRuntime {
    pub(crate) fn start(
        store: Option<Arc<dyn MemoryStore>>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        // Like Client's command queue, accepted edits must not be dropped when IO
        // is busy. One worker prevents older snapshots from overwriting newer ones.
        let (commands, receiver) = async_channel::unbounded();
        let task = handle.spawn(async move {
            while let Ok(action) = receiver.recv().await {
                match action {
                    Action::Load => {
                        let result = match &store {
                            Some(store) => store.list_chat_threads(50).await,
                            None => Ok(Vec::new()),
                        };
                        match result {
                            Ok(threads) => {
                                let _ = events.send(Event::ChatHistoryLoaded { threads }).await;
                            }
                            Err(error) => tracing::warn!(%error, "failed to load chat history"),
                        }
                    }
                    Action::Save(thread) => {
                        if let Some(store) = &store
                            && let Err(error) = store.save_chat_thread(&thread).await
                        {
                            tracing::warn!(%error, "failed to save chat thread");
                        }
                    }
                    Action::Delete { session, at } => {
                        if let Some(store) = &store
                            && let Err(error) = store.delete_chat_thread(&session, at).await
                        {
                            tracing::warn!(%error, "failed to delete chat thread");
                        }
                    }
                }
            }
        });
        Self { commands, task }
    }

    pub(crate) fn load(&self) {
        let _ = self.commands.try_send(Action::Load);
    }

    pub(crate) fn save(&self, mut thread: Box<ChatThread>) {
        if thread.updated_at.millis() <= 1_000_000 {
            let current = crate::now();
            if thread.created_at.millis() <= 1_000_000 {
                thread.created_at = current;
            }
            thread.updated_at = current;
        }
        let _ = self.commands.try_send(Action::Save(thread));
    }

    pub(crate) fn delete(&self, session: String) {
        let _ = self.commands.try_send(Action::Delete {
            session,
            at: crate::now(),
        });
    }

    pub(crate) async fn stop(&mut self) {
        self.commands.close();
        let _ = (&mut self.task).await;
    }
}

impl Drop for ChatHistoryRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}
