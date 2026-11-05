//! Persist, tick, and dispatch local schedules. Core stays clock-free; this
//! module owns the sleep-until-next loop and the JSON file.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use serde_json::json;
use sovra_adk::ScheduleTools;
use sovra_core::connections::ConnectionCommand;
use sovra_core::{
    Command, DueFire, Event, ScheduleBook, ScheduleCommand, ScheduleError, ScheduleEvent,
    ScheduleStore, Timestamp, complete_run, create_in_book, delete_in_book, fail_stale_runs,
    pause_in_book, resume_in_book, run_now_in_book, snapshot_event, tick,
};
use tokio::sync::oneshot;

enum Message {
    Command {
        command: ScheduleCommand,
        reply: Option<oneshot::Sender<Result<ScheduleBook, String>>>,
    },
    RunFinished {
        session_id: String,
        succeeded: bool,
    },
}

/// JSON file behind [`ScheduleStore`]. Missing files start empty.
#[derive(Debug)]
pub struct JsonScheduleStore {
    path: PathBuf,
}

impl JsonScheduleStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl ScheduleStore for JsonScheduleStore {
    fn load(&self) -> Result<ScheduleBook, ScheduleError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| ScheduleError(format!("Could not read saved schedules: {error}"))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ScheduleBook::default())
            }
            Err(error) => Err(ScheduleError(format!(
                "Could not open saved schedules: {error}"
            ))),
        }
    }

    fn save(&self, book: &ScheduleBook) -> Result<(), ScheduleError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                ScheduleError(format!("Could not prepare the schedules folder: {error}"))
            })?;
        }
        let bytes = serde_json::to_vec_pretty(book)
            .map_err(|error| ScheduleError(format!("Could not save schedules: {error}")))?;
        let tmp = tmp_path(&self.path);
        std::fs::write(&tmp, bytes)
            .map_err(|error| ScheduleError(format!("Could not write schedules: {error}")))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|error| ScheduleError(format!("Could not replace saved schedules: {error}")))
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.to_owned();
    tmp.set_extension("json.tmp");
    tmp
}

/// In-memory store used by tests and sessions without a file.
#[derive(Debug, Default)]
pub struct MemoryScheduleStore {
    book: std::sync::Mutex<ScheduleBook>,
}

impl ScheduleStore for MemoryScheduleStore {
    fn load(&self) -> Result<ScheduleBook, ScheduleError> {
        Ok(self
            .book
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone())
    }

    fn save(&self, book: &ScheduleBook) -> Result<(), ScheduleError> {
        *self
            .book
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = book.clone();
        Ok(())
    }
}

pub(crate) struct SchedulesRuntime {
    messages: Sender<Message>,
    events: Sender<Event>,
    task: tokio::task::JoinHandle<()>,
    tap: tokio::task::JoinHandle<()>,
}

impl SchedulesRuntime {
    pub(crate) fn start(
        store: Option<Arc<dyn ScheduleStore>>,
        events: Sender<Event>,
        commands_tx: Sender<Command>,
        tz_offset_minutes: i32,
        handle: &tokio::runtime::Handle,
    ) -> (Self, Sender<Event>) {
        let store = store.unwrap_or_else(|| Arc::new(MemoryScheduleStore::default()));
        let (messages, inbox) = async_channel::bounded(64);
        let (tapped_tx, tapped_rx) = async_channel::unbounded();
        let worker = Worker {
            store,
            book: ScheduleBook::default(),
            events: events.clone(),
            commands_tx,
            tz_offset_minutes,
        };
        let task = handle.spawn(worker.run(inbox));
        let tap_inbox = messages.clone();
        let tap = handle.spawn(tap_agent_lifecycle(tapped_rx, events, tap_inbox));
        (
            Self {
                messages,
                events: tapped_tx.clone(),
                task,
                tap,
            },
            tapped_tx,
        )
    }

    pub(crate) fn run_finished(&self, session_id: String, succeeded: bool) {
        let _ = self.messages.try_send(Message::RunFinished {
            session_id,
            succeeded,
        });
    }

    pub(crate) fn send(&self, command: ScheduleCommand) {
        if self
            .messages
            .try_send(Message::Command {
                command,
                reply: None,
            })
            .is_err()
        {
            let _ = self.events.try_send(Event::Schedule(ScheduleEvent::Error(
                "Too many schedule updates are pending. Wait briefly and try again.".into(),
            )));
        }
    }

    pub(crate) fn tools(&self) -> ScheduleTools {
        let messages = self.messages.clone();
        ScheduleTools::from_fn(move |command| {
            let messages = messages.clone();
            async move {
                let (reply, wait) = oneshot::channel();
                messages
                    .send(Message::Command {
                        command,
                        reply: Some(reply),
                    })
                    .await
                    .map_err(|_| "Schedules are not running in this session.".to_owned())?;
                let book = wait
                    .await
                    .map_err(|_| "The schedule helper stopped unexpectedly.".to_owned())??;
                Ok(json!({
                    "ok": true,
                    "schedules": book.schedules.iter().map(|item| json!({
                        "id": item.id,
                        "name": item.name,
                        "cadence": item.cadence_label(),
                        "status": item.status.label(),
                        "next_run": item.next_run_label(now()),
                        "prompt": item.prompt,
                        "send": item.send.as_ref().map(|send| json!({
                            "channel": send.channel,
                            "recipient": send.recipient,
                            "text": send.text,
                        })),
                    })).collect::<Vec<_>>(),
                }))
            }
        })
    }
}

impl Drop for SchedulesRuntime {
    fn drop(&mut self) {
        self.task.abort();
        self.tap.abort();
    }
}

async fn tap_agent_lifecycle(
    incoming: Receiver<Event>,
    outgoing: Sender<Event>,
    inbox: Sender<Message>,
) {
    while let Ok(event) = incoming.recv().await {
        match &event {
            Event::AgentCompleted { session_id } => {
                let _ = inbox
                    .send(Message::RunFinished {
                        session_id: session_id.clone(),
                        succeeded: true,
                    })
                    .await;
            }
            Event::AgentError { session_id, .. } => {
                let _ = inbox
                    .send(Message::RunFinished {
                        session_id: session_id.clone(),
                        succeeded: false,
                    })
                    .await;
            }
            _ => {}
        }
        if outgoing.send(event).await.is_err() {
            break;
        }
    }
}

struct Worker {
    store: Arc<dyn ScheduleStore>,
    book: ScheduleBook,
    events: Sender<Event>,
    commands_tx: Sender<Command>,
    tz_offset_minutes: i32,
}

impl Worker {
    async fn run(mut self, inbox: Receiver<Message>) {
        match tokio::task::spawn_blocking({
            let store = self.store.clone();
            move || store.load()
        })
        .await
        {
            Ok(Ok(book)) => self.book = book,
            Ok(Err(error)) => {
                let _ = self
                    .events
                    .send(Event::Schedule(ScheduleEvent::Error(error.to_string())))
                    .await;
            }
            Err(_) => {
                let _ = self
                    .events
                    .send(Event::Schedule(ScheduleEvent::Error(
                        "Could not load saved schedules.".into(),
                    )))
                    .await;
            }
        }
        if fail_stale_runs(&mut self.book, now()) {
            self.persist().await;
        }
        loop {
            let wait = next_wake(&self.book, now());
            tokio::select! {
                message = inbox.recv() => {
                    let Ok(message) = message else { break };
                    self.handle(message).await;
                }
                () = tokio::time::sleep(wait) => {
                    self.apply_tick(now()).await;
                }
            }
        }
    }

    async fn handle(&mut self, message: Message) {
        match message {
            Message::RunFinished {
                session_id,
                succeeded,
            } => {
                if complete_run(&mut self.book, &session_id, succeeded, None, now()) {
                    self.persist_and_snapshot().await;
                }
            }
            Message::Command { command, reply } => {
                let result = self.apply_command(command).await;
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
        }
    }

    async fn apply_command(&mut self, command: ScheduleCommand) -> Result<ScheduleBook, String> {
        let at = now();
        match command {
            ScheduleCommand::Load => {}
            ScheduleCommand::Create {
                name,
                cadence,
                prompt,
                send,
            } => {
                create_in_book(
                    &mut self.book,
                    &name,
                    &cadence,
                    &prompt,
                    send,
                    at,
                    self.tz_offset_minutes,
                )
                .map_err(|error| error.to_string())?;
            }
            ScheduleCommand::Pause { id } => {
                pause_in_book(&mut self.book, &id).map_err(|error| error.to_string())?;
            }
            ScheduleCommand::Resume { id } => {
                resume_in_book(&mut self.book, &id, at).map_err(|error| error.to_string())?;
            }
            ScheduleCommand::Delete { id } => {
                let _ = delete_in_book(&mut self.book, &id);
            }
            ScheduleCommand::RunNow { id, session_id } => {
                match run_now_in_book(&mut self.book, &id, session_id, at) {
                    Ok(Some(fire)) => {
                        self.persist_and_snapshot().await;
                        self.dispatch_fire(fire).await;
                        return Ok(self.book.clone());
                    }
                    Ok(None) => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            ScheduleCommand::Tick { now } => {
                self.apply_tick(now).await;
                return Ok(self.book.clone());
            }
            ScheduleCommand::FinishRun {
                session_id,
                succeeded,
                error,
            } => {
                complete_run(&mut self.book, &session_id, succeeded, error, at);
            }
        }
        self.persist_and_snapshot().await;
        Ok(self.book.clone())
    }

    async fn apply_tick(&mut self, at: Timestamp) {
        let effect = tick(&mut self.book, at);
        if effect.fire.is_empty() && effect.skipped.is_empty() {
            return;
        }
        self.persist_and_snapshot().await;
        for fire in effect.fire {
            self.dispatch_fire(fire).await;
        }
    }

    async fn dispatch_fire(&mut self, fire: DueFire) {
        let _ = self
            .events
            .send(Event::Schedule(ScheduleEvent::Fired {
                schedule_id: fire.schedule_id,
                session_id: fire.session_id.clone(),
                name: fire.name,
            }))
            .await;
        let command = match fire.send {
            Some(send) => Command::Connections(ConnectionCommand::SendScheduled {
                session_id: fire.session_id,
                channel: send.channel,
                recipient: send.recipient,
                text: send.text,
            }),
            None => Command::RunPrompt {
                context: None,
                session_id: fire.session_id,
                prompt: fire.prompt,
            },
        };
        let _ = self.commands_tx.send(command).await;
    }

    async fn persist(&self) {
        let store = self.store.clone();
        let book = self.book.clone();
        let persist = tokio::task::spawn_blocking(move || store.save(&book)).await;
        match persist {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = self
                    .events
                    .send(Event::Schedule(ScheduleEvent::Error(error.to_string())))
                    .await;
            }
            Err(_) => {
                let _ = self
                    .events
                    .send(Event::Schedule(ScheduleEvent::Error(
                        "Could not save schedules.".into(),
                    )))
                    .await;
            }
        }
    }

    async fn persist_and_snapshot(&self) {
        self.persist().await;
        let _ = self
            .events
            .send(Event::Schedule(snapshot_event(&self.book)))
            .await;
    }
}

fn next_wake(book: &ScheduleBook, at: Timestamp) -> Duration {
    let next = book
        .schedules
        .iter()
        .filter(|item| item.status == sovra_core::ScheduleStatus::Active)
        .map(|item| item.next_run_at.0)
        .min();
    match next {
        Some(due) if due <= at.0 => Duration::from_millis(50),
        Some(due) => {
            let wait = u64::try_from((due - at.0).min(60_000)).unwrap_or(60_000);
            Duration::from_millis(wait.max(50))
        }
        None => Duration::from_secs(60),
    }
}

fn now() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| {
            i64::try_from(time.as_millis()).unwrap_or(i64::MAX)
        });
    Timestamp(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_loads_an_empty_book() {
        let dir = std::env::temp_dir().join(format!(
            "sovra-sched-store-{}-{}",
            std::process::id(),
            now().0
        ));
        let path = dir.join("schedules.json");
        let store = JsonScheduleStore::new(&path);
        let book = store.load().expect("empty");
        assert_eq!(book.schedules, Vec::new());
        let mut written = ScheduleBook::default();
        create_in_book(
            &mut written,
            "Weekday briefing",
            "every weekday at 9",
            "Say hello",
            None,
            Timestamp(0),
            0,
        )
        .expect("create");
        store.save(&written).expect("save");
        let loaded = JsonScheduleStore::new(&path).load().expect("reload");
        assert_eq!(loaded.schedules.len(), 1);
        assert_eq!(loaded.schedules[0].name, "Weekday briefing");
        let _ = std::fs::remove_dir_all(dir);
    }
}
