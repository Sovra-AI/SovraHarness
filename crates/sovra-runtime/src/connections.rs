//! Drives the personal messaging link (`WhatsApp`) behind the core `MessagingChannel`
//! port: starts and resumes the link, turns adapter events into UI events, persists the
//! state, and answers incoming messages by running them through the same prompt path
//! as the chat window. One task owns everything, so there are no locks.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use async_channel::{Receiver, Sender};
use sovra_core::connections::{
    ANSWER_FAILED_REPLY, ChannelEvent, ChannelSession, ChannelStartOptions, ConnectionCommand,
    ConnectionEvent, ConnectionsState, IncomingMessage, LinkedDevice, MessagingChannel,
    NO_MODEL_REPLY, WhatsAppBridgedMessage, WhatsAppQrTicket, WhatsAppStatus, is_whatsapp_session,
    normalize_phone_number, should_answer, should_record, whatsapp_session_id,
};
use sovra_core::{Command, Event, MemoryStore};

/// Message ids this computer sent recently; the phone may mirror them back.
const SENT_IDS_REMEMBERED: usize = 200;

enum Input {
    /// The memory store, once the runtime has opened it.
    Store(Option<Arc<dyn MemoryStore>>),
    Command(ConnectionCommand),
    Channel(ChannelEvent),
    Agent(Event),
}

/// Handle the runtime loop uses to hand work to the connections task.
#[derive(Clone)]
pub(crate) struct ConnectionsRuntime {
    inbox: Sender<Input>,
}

impl ConnectionsRuntime {
    pub(crate) fn start(
        channel: Option<Arc<dyn MessagingChannel>>,
        events: Sender<Event>,
        commands: Sender<Command>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (inbox, rx) = async_channel::unbounded();
        // Weak: the task owns the sink, so a strong sender would keep its own inbox open
        // and the task alive after the runtime drops its handle at shutdown.
        let sink_inbox = inbox.downgrade();
        let actor = Actor {
            store: None,
            channel,
            events,
            commands,
            sink: Arc::new(move |event| {
                if let Some(inbox) = sink_inbox.upgrade() {
                    let _ = inbox.try_send(Input::Channel(event));
                }
            }),
            state: ConnectionsState::default(),
            session: None,
            pending_code: None,
            replies: BTreeMap::new(),
            sent_ids: VecDeque::new(),
        };
        drop(handle.spawn(actor.run(rx)));
        Self { inbox }
    }

    /// The runtime opens its store after the event plumbing exists; saved state loads and
    /// a saved link resumes once it arrives.
    pub(crate) fn attach_store(&self, store: Option<Arc<dyn MemoryStore>>) {
        let _ = self.inbox.try_send(Input::Store(store));
    }

    pub(crate) fn handle(&self, command: ConnectionCommand) {
        let _ = self.inbox.try_send(Input::Command(command));
    }

    /// Returns the sender the rest of the runtime emits on. Events for sessions that
    /// answer the phone come here; everything else goes on to `ui_events` in order.
    pub(crate) fn route_events(
        &self,
        ui_events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Sender<Event> {
        let (tx, rx) = async_channel::unbounded::<Event>();
        let inbox = self.inbox.clone();
        drop(handle.spawn(async move {
            while let Ok(event) = rx.recv().await {
                if event.session_id().is_some_and(is_whatsapp_session) {
                    let _ = inbox.try_send(Input::Agent(event));
                } else if ui_events.send(event).await.is_err() {
                    break;
                }
            }
        }));
        tx
    }
}

/// An answer being produced for one incoming message.
struct PendingReply {
    chat_jid: String,
    message_id: String,
    text: String,
}

struct Actor {
    store: Option<Arc<dyn MemoryStore>>,
    channel: Option<Arc<dyn MessagingChannel>>,
    events: Sender<Event>,
    commands: Sender<Command>,
    sink: sovra_core::connections::ChannelSink,
    state: ConnectionsState,
    session: Option<Arc<dyn ChannelSession>>,
    /// A phone-number code that arrived before the first QR code.
    pending_code: Option<String>,
    replies: BTreeMap<String, PendingReply>,
    sent_ids: VecDeque<String>,
}

impl Actor {
    async fn run(mut self, inbox: Receiver<Input>) {
        while let Ok(input) = inbox.recv().await {
            match input {
                Input::Store(store) => {
                    self.store = store;
                    self.load().await;
                    self.resume_saved_link().await;
                }
                Input::Command(command) => self.command(command).await,
                Input::Channel(event) => self.channel_event(event).await,
                Input::Agent(event) => self.agent_event(event).await,
            }
        }
        if let Some(session) = self.session.take() {
            session.stop().await;
        }
    }

    async fn load(&mut self) {
        let Some(store) = &self.store else { return };
        match store.load_connections().await {
            Ok(Some(saved)) => {
                self.state = saved;
                self.state.qr_modal_open = false;
                self.state.notice = None;
                // Only a saved link survives a restart; anything else starts over.
                if let WhatsAppStatus::Connected { device, .. } = &self.state.whatsapp {
                    self.state.whatsapp = WhatsAppStatus::Connected {
                        device: device.clone(),
                        online: false,
                    };
                } else {
                    self.state.whatsapp = WhatsAppStatus::Disconnected;
                }
            }
            Ok(None) => {}
            Err(error) => {
                // A record from an older shape (the mock pairing screen) cannot be read;
                // replace it once so the warning does not repeat on every launch.
                tracing::warn!(%error, "saved connections state unreadable; starting over");
                self.save().await;
            }
        }
    }

    /// A link saved by an earlier run reconnects in the background.
    async fn resume_saved_link(&mut self) {
        if !self.state.whatsapp.is_linked() {
            return;
        }
        let Some(channel) = self.channel.clone() else {
            return;
        };
        if !channel.has_saved_link() {
            self.set_status(WhatsAppStatus::Disconnected).await;
            self.save().await;
            return;
        }
        self.start_session(&channel, ChannelStartOptions::default(), false)
            .await;
    }

    async fn command(&mut self, command: ConnectionCommand) {
        match command {
            ConnectionCommand::Load => {
                self.emit(ConnectionEvent::Loaded(Box::new(self.state.clone())))
                    .await;
            }
            ConnectionCommand::StartWhatsAppPairing | ConnectionCommand::RefreshWhatsAppQr => {
                self.start_pairing(None).await;
            }
            ConnectionCommand::RequestPairingCode { phone_number } => {
                match normalize_phone_number(&phone_number) {
                    Ok(digits) => self.start_pairing(Some(digits)).await,
                    Err(message) => self.emit(ConnectionEvent::Notice(message)).await,
                }
            }
            ConnectionCommand::CancelPairing => {
                self.stop_session().await;
                self.forget();
                self.set_status(WhatsAppStatus::Disconnected).await;
                self.save().await;
            }
            ConnectionCommand::UpdateWhatsAppSettings { settings } => {
                self.state.settings = settings;
                self.emit(ConnectionEvent::SettingsUpdated(settings)).await;
                self.save().await;
            }
            ConnectionCommand::SendTestWhatsAppMessage { text } => {
                self.send_test_message(text.trim()).await;
            }
            ConnectionCommand::DisconnectWhatsApp => {
                if let Some(session) = self.session.take() {
                    session.logout().await;
                }
                self.forget();
                self.replies.clear();
                self.set_status(WhatsAppStatus::Disconnected).await;
                self.emit(ConnectionEvent::Notice(
                    "WhatsApp is unlinked from this computer.".into(),
                ))
                .await;
                self.save().await;
            }
        }
    }

    async fn start_pairing(&mut self, phone_number: Option<String>) {
        let Some(channel) = self.channel.clone() else {
            self.set_status(WhatsAppStatus::Error {
                message: "WhatsApp linking is not available in this build.".into(),
            })
            .await;
            return;
        };
        self.stop_session().await;
        // A new link always starts from fresh keys; stale ones make WhatsApp refuse.
        self.forget();
        self.pending_code = None;
        self.start_session(&channel, ChannelStartOptions { phone_number }, true)
            .await;
    }

    async fn start_session(
        &mut self,
        channel: &Arc<dyn MessagingChannel>,
        options: ChannelStartOptions,
        pairing: bool,
    ) {
        if pairing {
            self.set_status(WhatsAppStatus::Connecting { pairing: true })
                .await;
        }
        match channel.start(options, self.sink.clone()).await {
            Ok(session) => self.session = Some(session),
            Err(error) => {
                tracing::warn!(%error, "whatsapp link failed to start");
                if pairing {
                    self.set_status(WhatsAppStatus::Error {
                        message: error.to_string(),
                    })
                    .await;
                } else {
                    self.emit(ConnectionEvent::Notice(format!(
                        "WhatsApp could not reconnect: {error}"
                    )))
                    .await;
                }
            }
        }
    }

    async fn stop_session(&mut self) {
        if let Some(session) = self.session.take() {
            session.stop().await;
        }
    }

    fn forget(&self) {
        if let Some(channel) = &self.channel
            && let Err(error) = channel.forget()
        {
            tracing::warn!(%error, "failed to remove saved whatsapp credentials");
        }
    }

    async fn send_test_message(&mut self, text: &str) {
        if text.is_empty() {
            self.emit(ConnectionEvent::Notice("Type a message first.".into()))
                .await;
            return;
        }
        let Some(session) = self.session.clone() else {
            self.emit(ConnectionEvent::Notice(
                "WhatsApp is not connected right now.".into(),
            ))
            .await;
            return;
        };
        let Some(chat) = session.own_chat_jid() else {
            self.emit(ConnectionEvent::Notice(
                "WhatsApp is still finishing the link. Try again in a moment.".into(),
            ))
            .await;
            return;
        };
        match session.send_text(&chat, text).await {
            Ok(id) => {
                self.remember_sent(id);
                self.emit(ConnectionEvent::Notice(
                    "Sent. Open WhatsApp on your phone and look in the chat with yourself.".into(),
                ))
                .await;
            }
            Err(error) => {
                self.emit(ConnectionEvent::Notice(format!("Could not send: {error}")))
                    .await;
            }
        }
    }

    fn remember_sent(&mut self, id: String) {
        self.sent_ids.push_back(id);
        while self.sent_ids.len() > SENT_IDS_REMEMBERED {
            self.sent_ids.pop_front();
        }
    }

    async fn channel_event(&mut self, event: ChannelEvent) {
        match event {
            ChannelEvent::Connecting => {}
            ChannelEvent::QrCode {
                payload,
                matrix,
                expires_in_secs,
            } => {
                let pairing_code = match &self.state.whatsapp {
                    WhatsAppStatus::AwaitingQrScan { ticket } => ticket.pairing_code.clone(),
                    WhatsAppStatus::Disconnected
                    | WhatsAppStatus::Connecting { .. }
                    | WhatsAppStatus::VerifyingHandshake { .. }
                    | WhatsAppStatus::Connected { .. }
                    | WhatsAppStatus::Error { .. } => None,
                }
                .or_else(|| self.pending_code.take());
                self.set_status(WhatsAppStatus::AwaitingQrScan {
                    ticket: WhatsAppQrTicket {
                        qr_payload: payload,
                        qr_matrix: matrix,
                        expires_in_secs,
                        pairing_code,
                    },
                })
                .await;
            }
            ChannelEvent::PairingCode { code, .. } => {
                let status = match &self.state.whatsapp {
                    WhatsAppStatus::AwaitingQrScan { ticket } => WhatsAppStatus::AwaitingQrScan {
                        ticket: WhatsAppQrTicket {
                            pairing_code: Some(code),
                            ..ticket.clone()
                        },
                    },
                    WhatsAppStatus::Connecting { .. } => WhatsAppStatus::AwaitingQrScan {
                        ticket: WhatsAppQrTicket {
                            pairing_code: Some(code),
                            ..WhatsAppQrTicket::pending()
                        },
                    },
                    WhatsAppStatus::Disconnected
                    | WhatsAppStatus::VerifyingHandshake { .. }
                    | WhatsAppStatus::Connected { .. }
                    | WhatsAppStatus::Error { .. } => {
                        self.pending_code = Some(code);
                        return;
                    }
                };
                self.set_status(status).await;
            }
            ChannelEvent::PairingCodeFailed { message } => {
                self.emit(ConnectionEvent::Notice(message)).await;
            }
            ChannelEvent::Scanned { device_label } => {
                self.set_status(WhatsAppStatus::VerifyingHandshake { device_label })
                    .await;
            }
            ChannelEvent::Connected { device } => {
                let freshly_linked = !self.state.whatsapp.is_linked();
                let device = LinkedDevice {
                    linked_at: self
                        .state
                        .whatsapp
                        .linked_device()
                        .map_or_else(crate::now, |saved| saved.linked_at),
                    ..device
                };
                self.set_status(WhatsAppStatus::Connected {
                    device,
                    online: true,
                })
                .await;
                if freshly_linked {
                    self.emit(ConnectionEvent::Notice(
                        "WhatsApp is linked. Message yourself on WhatsApp to talk to Sovra from your phone."
                            .into(),
                    ))
                    .await;
                }
                self.save().await;
            }
            ChannelEvent::Offline { reason } => {
                tracing::info!(%reason, "whatsapp link offline");
                if let WhatsAppStatus::Connected { device, .. } = &self.state.whatsapp {
                    let device = device.clone();
                    self.set_status(WhatsAppStatus::Connected {
                        device,
                        online: false,
                    })
                    .await;
                }
            }
            ChannelEvent::LoggedOut { reason } => {
                tracing::info!(%reason, "whatsapp link removed");
                self.stop_session().await;
                self.forget();
                self.replies.clear();
                self.set_status(WhatsAppStatus::Disconnected).await;
                self.emit(ConnectionEvent::Notice(
                    "WhatsApp unlinked this computer. Click Connect WhatsApp to link again.".into(),
                ))
                .await;
                self.save().await;
            }
            ChannelEvent::Failed { message } => {
                tracing::warn!(%message, "whatsapp link failed");
                self.stop_session().await;
                if self.state.whatsapp.is_linked() {
                    self.emit(ConnectionEvent::Notice(message)).await;
                } else {
                    self.set_status(WhatsAppStatus::Error { message }).await;
                }
            }
            ChannelEvent::Message(message) => self.incoming(message).await,
        }
    }

    async fn incoming(&mut self, message: IncomingMessage) {
        if self.sent_ids.contains(&message.id) || !should_record(&message) {
            return;
        }
        let answer = should_answer(&message, &self.state.settings);
        let bridged = WhatsAppBridgedMessage {
            id: message.id.clone(),
            chat_jid: message.chat_jid.clone(),
            sender_name: if message.is_self_chat() {
                "You (WhatsApp)".to_owned()
            } else if message.sender_name.is_empty() {
                message.sender_jid.clone()
            } else {
                message.sender_name.clone()
            },
            sender_jid: message.sender_jid.clone(),
            text: message.text.clone(),
            reply_text: None,
            received_at: crate::now(),
        };
        self.state
            .apply(&ConnectionEvent::MessageBridged(bridged.clone()));
        self.emit(ConnectionEvent::MessageBridged(bridged)).await;
        self.save().await;
        if !answer {
            return;
        }
        let session_id = whatsapp_session_id(&message.chat_jid);
        self.replies.insert(
            session_id.clone(),
            PendingReply {
                chat_jid: message.chat_jid,
                message_id: message.id,
                text: String::new(),
            },
        );
        let _ = self
            .commands
            .send(Command::RunPrompt {
                context: None,
                session_id,
                prompt: message.text,
            })
            .await;
    }

    async fn agent_event(&mut self, event: Event) {
        let Some(session_id) = event.session_id().map(str::to_owned) else {
            return;
        };
        match event {
            Event::AgentChunk { text, .. } => {
                if let Some(reply) = self.replies.get_mut(&session_id) {
                    reply.text.push_str(&text);
                }
            }
            Event::AgentCompleted { .. } => {
                let Some(reply) = self.replies.remove(&session_id) else {
                    return;
                };
                let text = if reply.text.trim().is_empty() {
                    NO_MODEL_REPLY.to_owned()
                } else {
                    reply.text.trim().to_owned()
                };
                self.send_reply(&reply.chat_jid, &reply.message_id, text)
                    .await;
            }
            Event::AgentError { message, .. } => {
                tracing::warn!(%message, "whatsapp answer failed");
                self.emit(ConnectionEvent::Notice(format!(
                    "Could not answer a WhatsApp message: {message}"
                )))
                .await;
                if let Some(reply) = self.replies.remove(&session_id) {
                    self.send_reply(
                        &reply.chat_jid,
                        &reply.message_id,
                        ANSWER_FAILED_REPLY.to_owned(),
                    )
                    .await;
                }
            }
            Event::AgUi { .. }
            | Event::AgentResponseStarted { .. }
            | Event::AgentThinkingChunk { .. }
            | Event::AgUiSurface { .. }
            | Event::ChatTitleGenerated { .. }
            | Event::FilePrepared(_)
            | Event::Memory(_)
            | Event::Models(_)
            | Event::FileIndex(_)
            | Event::Skills(_)
            | Event::Teams(_)
            | Event::Artifacts(_)
            | Event::Connections(_)
            | Event::ChatHistoryLoaded { .. }
            | Event::Ready { .. }
            | Event::Pong
            | Event::RouteEvaluated { .. }
            | Event::Stopped => {}
        }
    }

    async fn send_reply(&mut self, chat_jid: &str, message_id: &str, text: String) {
        let Some(session) = self.session.clone() else {
            return;
        };
        match session.send_text(chat_jid, &text).await {
            Ok(id) => {
                self.remember_sent(id);
                let event = ConnectionEvent::ReplySent {
                    message_id: message_id.to_owned(),
                    reply: text,
                };
                self.state.apply(&event);
                self.emit(event).await;
                self.save().await;
            }
            Err(error) => {
                tracing::warn!(%error, "whatsapp reply not sent");
                self.emit(ConnectionEvent::Notice(format!(
                    "Could not send a reply over WhatsApp: {error}"
                )))
                .await;
            }
        }
    }

    async fn set_status(&mut self, status: WhatsAppStatus) {
        let event = ConnectionEvent::StatusChanged(status);
        self.state.apply(&event);
        self.emit(event).await;
    }

    async fn emit(&self, event: ConnectionEvent) {
        let _ = self.events.send(Event::Connections(event)).await;
    }

    async fn save(&self) {
        if let Some(store) = &self.store
            && let Err(error) = store.save_connections(&self.state).await
        {
            tracing::warn!(%error, "failed to persist connections state");
        }
    }
}
