//! Drives the personal messaging links (`WhatsApp`, Telegram) behind the core
//! `MessagingChannel` port: starts and resumes each link, turns adapter events into UI
//! events, persists the state, and answers incoming messages by running them through the
//! same prompt path as the chat window. One task owns everything, so there are no locks.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use async_channel::{Receiver, Sender};
use sovra_core::connections::{
    ANSWER_FAILED_REPLY, BridgedMessage, ChannelError, ChannelEvent, ChannelKind, ChannelSession,
    ChannelSink, ChannelStartOptions, ConnectionCommand, ConnectionEvent, ConnectionsState,
    IncomingMessage, LinkedDevice, MessagingChannel, NO_MODEL_REPLY, PollOutcome,
    TELEGRAM_ASK_FOR_CODE_REPLY, TELEGRAM_LINKED_REPLY, TelegramAction, TelegramStatus,
    WhatsAppQrTicket, WhatsAppStatus, channel_of_session, normalize_bot_token,
    normalize_phone_number, should_answer, should_record, telegram_action, telegram_link_code,
};
use sovra_core::{Command, Event, MemoryStore};

/// Message ids this computer sent recently; a service may mirror them back.
const SENT_IDS_REMEMBERED: usize = 200;

enum Input {
    /// The memory store, once the runtime has opened it.
    Store(Option<Arc<dyn MemoryStore>>),
    Command(ConnectionCommand),
    Channel(ChannelKind, ChannelEvent),
    Agent(Event),
}

/// The adapters the composition root supplies; any may be missing in a build.
#[derive(Default, Clone)]
pub(crate) struct Channels {
    pub(crate) whatsapp: Option<Arc<dyn MessagingChannel>>,
    pub(crate) telegram: Option<Arc<dyn MessagingChannel>>,
}

/// Handle the runtime loop uses to hand work to the connections task.
#[derive(Clone)]
pub(crate) struct ConnectionsRuntime {
    inbox: Sender<Input>,
}

impl ConnectionsRuntime {
    pub(crate) fn start(
        channels: Channels,
        events: Sender<Event>,
        commands: Sender<Command>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (inbox, rx) = async_channel::unbounded();
        let actor = Actor {
            store: None,
            events,
            commands,
            handle: handle.clone(),
            state: ConnectionsState::default(),
            whatsapp: Link::new(ChannelKind::WhatsApp, channels.whatsapp, &inbox),
            telegram: Link::new(ChannelKind::Telegram, channels.telegram, &inbox),
            pending_code: None,
            telegram_code: String::new(),
            replies: BTreeMap::new(),
        };
        drop(handle.spawn(actor.run(rx)));
        Self { inbox }
    }

    /// The runtime opens its store after the event plumbing exists; saved state loads and
    /// saved links resume once it arrives.
    pub(crate) fn attach_store(&self, store: Option<Arc<dyn MemoryStore>>) {
        let _ = self.inbox.try_send(Input::Store(store));
    }

    pub(crate) fn handle(&self, command: ConnectionCommand) {
        let _ = self.inbox.try_send(Input::Command(command));
    }

    /// Returns the sender the rest of the runtime emits on. Events for sessions that
    /// answer a linked service come here; everything else goes on to `ui_events` in order.
    pub(crate) fn route_events(
        &self,
        ui_events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Sender<Event> {
        let (tx, rx) = async_channel::unbounded::<Event>();
        let inbox = self.inbox.clone();
        drop(handle.spawn(async move {
            while let Ok(event) = rx.recv().await {
                if event
                    .session_id()
                    .is_some_and(|id| channel_of_session(id).is_some())
                {
                    let _ = inbox.try_send(Input::Agent(event));
                } else if ui_events.send(event).await.is_err() {
                    break;
                }
            }
        }));
        tx
    }
}

/// One service's adapter and its open session.
struct Link {
    kind: ChannelKind,
    channel: Option<Arc<dyn MessagingChannel>>,
    session: Option<Arc<dyn ChannelSession>>,
    sink: ChannelSink,
    sent_ids: VecDeque<String>,
}

impl Link {
    fn new(
        kind: ChannelKind,
        channel: Option<Arc<dyn MessagingChannel>>,
        inbox: &Sender<Input>,
    ) -> Self {
        // Weak: the task owns the sink, so a strong sender would keep its own inbox open
        // and the task alive after the runtime drops its handle at shutdown.
        let weak = inbox.downgrade();
        Self {
            kind,
            channel,
            session: None,
            sink: Arc::new(move |event| {
                if let Some(inbox) = weak.upgrade() {
                    let _ = inbox.try_send(Input::Channel(kind, event));
                }
            }),
            sent_ids: VecDeque::new(),
        }
    }

    fn has_saved_link(&self) -> bool {
        self.channel
            .as_ref()
            .is_some_and(|channel| channel.has_saved_link())
    }

    async fn stop(&mut self) {
        if let Some(session) = self.session.take() {
            session.stop().await;
        }
    }

    async fn logout(&mut self) {
        if let Some(session) = self.session.take() {
            session.logout().await;
        }
    }

    fn forget(&self) {
        if let Some(channel) = &self.channel
            && let Err(error) = channel.forget()
        {
            tracing::warn!(%error, service = self.kind.label(), "failed to remove saved credentials");
        }
    }

    fn remember_sent(&mut self, id: String) {
        self.sent_ids.push_back(id);
        while self.sent_ids.len() > SENT_IDS_REMEMBERED {
            self.sent_ids.pop_front();
        }
    }

    /// Opens the link and keeps its queue drained for services that poll.
    async fn start(
        &mut self,
        options: ChannelStartOptions,
        handle: &tokio::runtime::Handle,
    ) -> Result<(), ChannelError> {
        let Some(channel) = self.channel.clone() else {
            return Err(ChannelError::Unavailable(format!(
                "{} linking is not available in this build.",
                self.kind.label()
            )));
        };
        let session = channel.start(options, self.sink.clone()).await?;
        let poller = session.clone();
        drop(handle.spawn(async move {
            loop {
                match poller.poll().await {
                    PollOutcome::Again => {}
                    PollOutcome::Wait(pause) => tokio::time::sleep(pause).await,
                    PollOutcome::Finished => break,
                }
            }
        }));
        self.session = Some(session);
        Ok(())
    }
}

/// An answer being produced for one incoming message.
struct PendingReply {
    kind: ChannelKind,
    chat: String,
    message_id: String,
    text: String,
}

struct Actor {
    store: Option<Arc<dyn MemoryStore>>,
    events: Sender<Event>,
    commands: Sender<Command>,
    handle: tokio::runtime::Handle,
    state: ConnectionsState,
    whatsapp: Link,
    telegram: Link,
    /// A `WhatsApp` phone-number code that arrived before the first QR code.
    pending_code: Option<String>,
    /// The code the Telegram bot's owner must send; set when a token is connected.
    telegram_code: String,
    replies: BTreeMap<String, PendingReply>,
}

impl Actor {
    async fn run(mut self, inbox: Receiver<Input>) {
        while let Ok(input) = inbox.recv().await {
            match input {
                Input::Store(store) => {
                    self.store = store;
                    self.load().await;
                    self.resume_saved_links().await;
                }
                Input::Command(command) => self.command(command).await,
                Input::Channel(ChannelKind::WhatsApp, event) => self.whatsapp_event(event).await,
                Input::Channel(ChannelKind::Telegram, event) => self.telegram_event(event).await,
                Input::Agent(event) => self.agent_event(event).await,
            }
        }
        self.whatsapp.stop().await;
        self.telegram.stop().await;
    }

    fn link(&mut self, kind: ChannelKind) -> &mut Link {
        match kind {
            ChannelKind::WhatsApp => &mut self.whatsapp,
            ChannelKind::Telegram => &mut self.telegram,
        }
    }

    async fn load(&mut self) {
        let Some(store) = &self.store else { return };
        match store.load_connections().await {
            Ok(Some(saved)) => {
                self.state = saved;
                self.state.qr_modal_open = false;
                self.state.notice = None;
                // Only saved links survive a restart; anything else starts over.
                self.state.whatsapp = match &self.state.whatsapp {
                    WhatsAppStatus::Connected { device, .. } => WhatsAppStatus::Connected {
                        device: device.clone(),
                        online: false,
                    },
                    WhatsAppStatus::Disconnected
                    | WhatsAppStatus::Connecting { .. }
                    | WhatsAppStatus::AwaitingQrScan { .. }
                    | WhatsAppStatus::VerifyingHandshake { .. }
                    | WhatsAppStatus::Error { .. } => WhatsAppStatus::Disconnected,
                };
                self.state.telegram = match &self.state.telegram {
                    TelegramStatus::Connected {
                        bot_username,
                        owner_chat_id,
                        owner_name,
                        linked_at,
                        ..
                    } => TelegramStatus::Connected {
                        bot_username: bot_username.clone(),
                        owner_chat_id: owner_chat_id.clone(),
                        owner_name: owner_name.clone(),
                        linked_at: *linked_at,
                        online: false,
                    },
                    TelegramStatus::AwaitingOwner { code, .. } => {
                        self.telegram_code.clone_from(code);
                        self.state.telegram.clone()
                    }
                    TelegramStatus::Disconnected
                    | TelegramStatus::Connecting
                    | TelegramStatus::Error { .. } => TelegramStatus::Disconnected,
                };
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

    /// Links saved by an earlier run reconnect in the background.
    async fn resume_saved_links(&mut self) {
        if self.state.whatsapp.is_linked() {
            if self.whatsapp.has_saved_link() {
                self.resume(ChannelKind::WhatsApp).await;
            } else {
                self.set_whatsapp(WhatsAppStatus::Disconnected).await;
                self.save().await;
            }
        }
        if self.state.telegram.is_linked() {
            if self.telegram.has_saved_link() {
                self.resume(ChannelKind::Telegram).await;
            } else {
                self.set_telegram(TelegramStatus::Disconnected).await;
                self.save().await;
            }
        }
    }

    async fn resume(&mut self, kind: ChannelKind) {
        let handle = self.handle.clone();
        if let Err(error) = self
            .link(kind)
            .start(ChannelStartOptions::default(), &handle)
            .await
        {
            tracing::warn!(%error, service = kind.label(), "link failed to resume");
            self.emit(ConnectionEvent::Notice(format!(
                "{} could not reconnect: {error}",
                kind.label()
            )))
            .await;
        }
    }

    async fn command(&mut self, command: ConnectionCommand) {
        match command {
            ConnectionCommand::Load => {
                self.emit(ConnectionEvent::Loaded(Box::new(self.state.clone())))
                    .await;
            }
            ConnectionCommand::StartWhatsAppPairing | ConnectionCommand::RefreshWhatsAppQr => {
                self.start_whatsapp_pairing(None).await;
            }
            ConnectionCommand::RequestPairingCode { phone_number } => {
                match normalize_phone_number(&phone_number) {
                    Ok(digits) => self.start_whatsapp_pairing(Some(digits)).await,
                    Err(message) => self.emit(ConnectionEvent::Notice(message)).await,
                }
            }
            ConnectionCommand::CancelPairing => {
                self.whatsapp.stop().await;
                self.whatsapp.forget();
                self.set_whatsapp(WhatsAppStatus::Disconnected).await;
                self.save().await;
            }
            ConnectionCommand::UpdateWhatsAppSettings { settings } => {
                self.state.settings = settings;
                self.emit(ConnectionEvent::SettingsUpdated(settings)).await;
                self.save().await;
            }
            ConnectionCommand::SendTestWhatsAppMessage { text } => {
                let chat = self
                    .whatsapp
                    .session
                    .as_ref()
                    .and_then(|session| session.own_chat_jid());
                self.send_test_message(ChannelKind::WhatsApp, chat, text.trim())
                    .await;
            }
            ConnectionCommand::DisconnectWhatsApp => {
                self.whatsapp.logout().await;
                self.whatsapp.forget();
                self.replies
                    .retain(|_, reply| reply.kind != ChannelKind::WhatsApp);
                self.set_whatsapp(WhatsAppStatus::Disconnected).await;
                self.emit(ConnectionEvent::Notice(
                    "WhatsApp is unlinked from this computer.".into(),
                ))
                .await;
                self.save().await;
            }
            ConnectionCommand::ConnectTelegram { token } => match normalize_bot_token(&token) {
                Ok(token) => self.connect_telegram(token).await,
                Err(message) => self.emit(ConnectionEvent::Notice(message)).await,
            },
            ConnectionCommand::UpdateTelegramSettings { settings } => {
                self.state.telegram_settings = settings;
                self.emit(ConnectionEvent::TelegramSettingsUpdated(settings))
                    .await;
                self.save().await;
            }
            ConnectionCommand::SendTestTelegramMessage { text } => {
                let chat = self.state.telegram.owner_chat_id().map(str::to_owned);
                self.send_test_message(ChannelKind::Telegram, chat, text.trim())
                    .await;
            }
            ConnectionCommand::DisconnectTelegram => {
                self.telegram.logout().await;
                self.telegram.forget();
                self.replies
                    .retain(|_, reply| reply.kind != ChannelKind::Telegram);
                self.set_telegram(TelegramStatus::Disconnected).await;
                self.emit(ConnectionEvent::Notice(
                    "The Telegram bot is disconnected and its token forgotten.".into(),
                ))
                .await;
                self.save().await;
            }
        }
    }

    async fn start_whatsapp_pairing(&mut self, phone_number: Option<String>) {
        self.whatsapp.stop().await;
        // A new link always starts from fresh keys; stale ones make WhatsApp refuse.
        self.whatsapp.forget();
        self.pending_code = None;
        self.set_whatsapp(WhatsAppStatus::Connecting { pairing: true })
            .await;
        let options = ChannelStartOptions {
            phone_number,
            token: None,
        };
        let handle = self.handle.clone();
        if let Err(error) = self.whatsapp.start(options, &handle).await {
            tracing::warn!(%error, "whatsapp link failed to start");
            self.set_whatsapp(WhatsAppStatus::Error {
                message: error.to_string(),
            })
            .await;
        }
    }

    async fn connect_telegram(&mut self, token: String) {
        self.telegram.stop().await;
        self.telegram.forget();
        self.telegram_code = telegram_link_code(crate::now().millis());
        self.set_telegram(TelegramStatus::Connecting).await;
        let options = ChannelStartOptions {
            phone_number: None,
            token: Some(token),
        };
        let handle = self.handle.clone();
        if let Err(error) = self.telegram.start(options, &handle).await {
            tracing::warn!(%error, "telegram bot failed to start");
            self.set_telegram(TelegramStatus::Error {
                message: error.to_string(),
            })
            .await;
        }
    }

    async fn send_test_message(&mut self, kind: ChannelKind, chat: Option<String>, text: &str) {
        let label = kind.label();
        if text.is_empty() {
            self.emit(ConnectionEvent::Notice("Type a message first.".into()))
                .await;
            return;
        }
        let Some(session) = self.link(kind).session.clone() else {
            self.emit(ConnectionEvent::Notice(format!(
                "{label} is not connected right now."
            )))
            .await;
            return;
        };
        let Some(chat) = chat else {
            self.emit(ConnectionEvent::Notice(format!(
                "{label} is still finishing the link. Try again in a moment."
            )))
            .await;
            return;
        };
        match session.send_text(&chat, text).await {
            Ok(id) => {
                self.link(kind).remember_sent(id);
                let notice = match kind {
                    ChannelKind::WhatsApp => {
                        "Sent. Open WhatsApp on your phone and look in the chat with yourself."
                    }
                    ChannelKind::Telegram => "Sent. Open the chat with your bot in Telegram.",
                };
                self.emit(ConnectionEvent::Notice(notice.into())).await;
            }
            Err(error) => {
                self.emit(ConnectionEvent::Notice(format!("Could not send: {error}")))
                    .await;
            }
        }
    }

    async fn whatsapp_event(&mut self, event: ChannelEvent) {
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
                self.set_whatsapp(WhatsAppStatus::AwaitingQrScan {
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
                self.set_whatsapp(status).await;
            }
            ChannelEvent::PairingCodeFailed { message } => {
                self.emit(ConnectionEvent::Notice(message)).await;
            }
            ChannelEvent::Scanned { device_label } => {
                self.set_whatsapp(WhatsAppStatus::VerifyingHandshake { device_label })
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
                self.set_whatsapp(WhatsAppStatus::Connected {
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
                    self.set_whatsapp(WhatsAppStatus::Connected {
                        device,
                        online: false,
                    })
                    .await;
                }
            }
            ChannelEvent::LoggedOut { reason } => {
                tracing::info!(%reason, "whatsapp link removed");
                self.whatsapp.stop().await;
                self.whatsapp.forget();
                self.replies
                    .retain(|_, reply| reply.kind != ChannelKind::WhatsApp);
                self.set_whatsapp(WhatsAppStatus::Disconnected).await;
                self.emit(ConnectionEvent::Notice(
                    "WhatsApp unlinked this computer. Click Connect WhatsApp to link again.".into(),
                ))
                .await;
                self.save().await;
            }
            ChannelEvent::Failed { message } => {
                tracing::warn!(%message, "whatsapp link failed");
                self.whatsapp.stop().await;
                if self.state.whatsapp.is_linked() {
                    self.emit(ConnectionEvent::Notice(message)).await;
                } else {
                    self.set_whatsapp(WhatsAppStatus::Error { message }).await;
                }
            }
            ChannelEvent::Message(message) => self.whatsapp_message(message).await,
        }
    }

    async fn whatsapp_message(&mut self, message: IncomingMessage) {
        if self.whatsapp.sent_ids.contains(&message.id) || !should_record(&message) {
            return;
        }
        let answer = should_answer(&message, &self.state.settings);
        let sender_name = if message.is_self_chat() {
            "You (WhatsApp)".to_owned()
        } else if message.sender_name.is_empty() {
            message.sender_jid.clone()
        } else {
            message.sender_name.clone()
        };
        self.record_and_answer(ChannelKind::WhatsApp, message, sender_name, answer)
            .await;
    }

    async fn telegram_event(&mut self, event: ChannelEvent) {
        match event {
            ChannelEvent::Connecting
            | ChannelEvent::QrCode { .. }
            | ChannelEvent::PairingCode { .. }
            | ChannelEvent::PairingCodeFailed { .. }
            | ChannelEvent::Scanned { .. } => {}
            ChannelEvent::Connected { device } => {
                let bot_username = device.phone_display.trim_start_matches('@').to_owned();
                let status = match &self.state.telegram {
                    TelegramStatus::Connected {
                        owner_chat_id,
                        owner_name,
                        linked_at,
                        ..
                    } => TelegramStatus::Connected {
                        bot_username,
                        owner_chat_id: owner_chat_id.clone(),
                        owner_name: owner_name.clone(),
                        linked_at: *linked_at,
                        online: true,
                    },
                    TelegramStatus::AwaitingOwner { code, .. } => TelegramStatus::AwaitingOwner {
                        bot_username,
                        code: code.clone(),
                    },
                    TelegramStatus::Disconnected
                    | TelegramStatus::Connecting
                    | TelegramStatus::Error { .. } => TelegramStatus::AwaitingOwner {
                        bot_username,
                        code: self.telegram_code.clone(),
                    },
                };
                self.set_telegram(status).await;
                self.save().await;
            }
            ChannelEvent::Offline { reason } => {
                tracing::info!(%reason, "telegram bot offline");
                if let TelegramStatus::Connected {
                    bot_username,
                    owner_chat_id,
                    owner_name,
                    linked_at,
                    ..
                } = &self.state.telegram
                {
                    let status = TelegramStatus::Connected {
                        bot_username: bot_username.clone(),
                        owner_chat_id: owner_chat_id.clone(),
                        owner_name: owner_name.clone(),
                        linked_at: *linked_at,
                        online: false,
                    };
                    self.set_telegram(status).await;
                }
            }
            ChannelEvent::LoggedOut { reason } => {
                tracing::info!(%reason, "telegram token no longer accepted");
                self.telegram.stop().await;
                self.telegram.forget();
                self.replies
                    .retain(|_, reply| reply.kind != ChannelKind::Telegram);
                self.set_telegram(TelegramStatus::Error {
                    message: "Telegram no longer accepts the bot token. Get a new one from @BotFather and connect again.".into(),
                })
                .await;
                self.save().await;
            }
            ChannelEvent::Failed { message } => {
                tracing::warn!(%message, "telegram bot failed");
                self.telegram.stop().await;
                self.set_telegram(TelegramStatus::Error { message }).await;
            }
            ChannelEvent::Message(message) => self.telegram_message(message).await,
        }
    }

    async fn telegram_message(&mut self, message: IncomingMessage) {
        if self.telegram.sent_ids.contains(&message.id) {
            return;
        }
        match telegram_action(
            &message,
            &self.state.telegram,
            &self.state.telegram_settings,
        ) {
            TelegramAction::Ignore => {}
            TelegramAction::AskForCode => {
                self.send_plain(
                    ChannelKind::Telegram,
                    &message.chat_jid,
                    TELEGRAM_ASK_FOR_CODE_REPLY,
                )
                .await;
            }
            TelegramAction::LinkOwner => {
                let bot_username = self
                    .state
                    .telegram
                    .bot_username()
                    .unwrap_or_default()
                    .to_owned();
                self.set_telegram(TelegramStatus::Connected {
                    bot_username,
                    owner_chat_id: message.chat_jid.clone(),
                    owner_name: message.sender_name.clone(),
                    linked_at: crate::now(),
                    online: true,
                })
                .await;
                self.emit(ConnectionEvent::Notice(
                    "Telegram is linked. Message your bot to talk to Sovra from your phone.".into(),
                ))
                .await;
                self.save().await;
                self.send_plain(
                    ChannelKind::Telegram,
                    &message.chat_jid,
                    TELEGRAM_LINKED_REPLY,
                )
                .await;
            }
            TelegramAction::Record { answer } => {
                let from_owner =
                    self.state.telegram.owner_chat_id() == Some(message.chat_jid.as_str());
                let sender_name = if from_owner {
                    "You (Telegram)".to_owned()
                } else if message.sender_name.is_empty() {
                    message.sender_jid.clone()
                } else {
                    message.sender_name.clone()
                };
                self.record_and_answer(ChannelKind::Telegram, message, sender_name, answer)
                    .await;
            }
        }
    }

    /// Records the message for the Connections page and, when wanted, asks the helpers for
    /// an answer on the service's own session.
    async fn record_and_answer(
        &mut self,
        kind: ChannelKind,
        message: IncomingMessage,
        sender_name: String,
        answer: bool,
    ) {
        let bridged = BridgedMessage {
            id: message.id.clone(),
            channel: kind,
            chat_jid: message.chat_jid.clone(),
            sender_name,
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
        let session_id = kind.session_id(&message.chat_jid);
        self.replies.insert(
            session_id.clone(),
            PendingReply {
                kind,
                chat: message.chat_jid,
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
                self.send_reply(&reply, text).await;
            }
            Event::AgentError { message, .. } => {
                tracing::warn!(%message, %session_id, "linked service answer failed");
                self.emit(ConnectionEvent::Notice(format!(
                    "Could not answer a message: {message}"
                )))
                .await;
                if let Some(reply) = self.replies.remove(&session_id) {
                    self.send_reply(&reply, ANSWER_FAILED_REPLY.to_owned())
                        .await;
                }
            }
            Event::AgUi { .. }
            | Event::AgentResponseStarted { .. }
            | Event::AgentThinkingChunk { .. }
            | Event::AgUiSurface { .. }
            | Event::ChatTitleGenerated { .. }
            | Event::FilePrepared(_)
            | Event::Auth(_)
            | Event::Memory(_)
            | Event::Models(_)
            | Event::FileIndex(_)
            | Event::Skills(_)
            | Event::Teams(_)
            | Event::Artifacts(_)
            | Event::Connections(_)
            | Event::Development(_)
            | Event::Google(_)
            | Event::Mcp(_)
            | Event::ChatHistoryLoaded { .. }
            | Event::Ready { .. }
            | Event::Pong
            | Event::RouteEvaluated { .. }
            | Event::Stopped => {}
        }
    }

    /// Sends a service message (link confirmation, instructions) without recording it.
    async fn send_plain(&mut self, kind: ChannelKind, chat: &str, text: &str) {
        let Some(session) = self.link(kind).session.clone() else {
            return;
        };
        match session.send_text(chat, text).await {
            Ok(id) => self.link(kind).remember_sent(id),
            Err(error) => tracing::warn!(%error, service = kind.label(), "message not sent"),
        }
    }

    async fn send_reply(&mut self, reply: &PendingReply, text: String) {
        let Some(session) = self.link(reply.kind).session.clone() else {
            return;
        };
        match session.send_text(&reply.chat, &text).await {
            Ok(id) => {
                self.link(reply.kind).remember_sent(id);
                let event = ConnectionEvent::ReplySent {
                    message_id: reply.message_id.clone(),
                    reply: text,
                };
                self.state.apply(&event);
                self.emit(event).await;
                self.save().await;
            }
            Err(error) => {
                tracing::warn!(%error, service = reply.kind.label(), "reply not sent");
                self.emit(ConnectionEvent::Notice(format!(
                    "Could not send a reply over {}: {error}",
                    reply.kind.label()
                )))
                .await;
            }
        }
    }

    async fn set_whatsapp(&mut self, status: WhatsAppStatus) {
        let event = ConnectionEvent::StatusChanged(status);
        self.state.apply(&event);
        self.emit(event).await;
    }

    async fn set_telegram(&mut self, status: TelegramStatus) {
        let event = ConnectionEvent::TelegramStatusChanged(status);
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
