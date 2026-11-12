//! Drives the personal messaging links (`WhatsApp`, Telegram) behind the core
//! `MessagingChannel` port: starts and resumes each link, turns adapter events into UI
//! events, persists the state, and answers incoming messages by running them through the
//! same prompt path as the chat window. One task owns everything, so there are no locks.

mod messaging;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use async_channel::{Receiver, Sender};
use sovra_core::connection_catalogue::{
    CatalogueKind, LinkPhase, LinkProbe, PROBE_BOUND, ProbeReport,
};
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
    Messaging {
        session_id: String,
        turn: u64,
        request: sovra_core::communications::MessagingRequest,
        reply: tokio::sync::oneshot::Sender<Result<serde_json::Value, ChannelError>>,
    },
    ProbeDone {
        generation: u64,
        report: ProbeReport,
    },
    /// A background contact pull finished. A stale generation is ignored.
    ContactsPulled {
        generation: u64,
        pulled: Result<sovra_core::connections::AddressBookSnapshot, ChannelError>,
    },
}

/// A saved messaging link, checked on its own during a catalogue refresh.
struct SavedLinkProbe {
    kind: CatalogueKind,
    channel: Arc<dyn MessagingChannel>,
}

#[async_trait::async_trait]
impl LinkProbe for SavedLinkProbe {
    fn kind(&self) -> CatalogueKind {
        self.kind
    }

    async fn probe(&self) -> Option<ProbeReport> {
        // The trait default, and any adapter that did not reach the service,
        // returns Ok(false). Reporting that as Linked would replace a live
        // "working on it" or "needs you" row until the next status event.
        let phase = match self.channel.check().await {
            Ok(false) => return None,
            Ok(true) => LinkPhase::Linked,
            Err(_) => LinkPhase::NeedsYou,
        };
        Some(ProbeReport {
            kind: self.kind,
            phase,
        })
    }
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
    messaging_available: bool,
    next_turn: Arc<std::sync::atomic::AtomicU64>,
    messaging_admission: Arc<tokio::sync::Semaphore>,
    owner_sessions: Arc<std::sync::RwLock<std::collections::BTreeSet<String>>>,
}

impl ConnectionsRuntime {
    pub(crate) fn start(
        channels: Channels,
        events: Sender<Event>,
        commands: Sender<Command>,
        handle: &tokio::runtime::Handle,
        probes: Vec<Arc<dyn LinkProbe>>,
    ) -> Self {
        let messaging_available = channels.whatsapp.is_some() || channels.telegram.is_some();
        let (inbox, rx) = async_channel::unbounded();
        let owner_sessions = Arc::new(std::sync::RwLock::new(std::collections::BTreeSet::new()));
        let actor = Actor {
            owner_sessions: owner_sessions.clone(),
            store: None,
            events,
            commands,
            handle: handle.clone(),
            inbox: inbox.downgrade(),
            state: ConnectionsState::default(),
            whatsapp: Link::new(ChannelKind::WhatsApp, channels.whatsapp, &inbox),
            telegram: Link::new(ChannelKind::Telegram, channels.telegram, &inbox),
            pending_code: None,
            telegram_code: String::new(),
            replies: BTreeMap::new(),
            messaging: messaging::MessagingState::default(),
            probes,
            refresh_generation: 0,
            refresh_tasks: Vec::new(),
            contact_generation: 0,
            contact_task: None,
        };
        drop(handle.spawn(actor.run(rx)));
        Self {
            inbox,
            messaging_available,
            next_turn: messaging::counter(),
            messaging_admission: messaging::admission(),
            owner_sessions,
        }
    }

    /// Runtime-authenticated own chats retain the owner's tools; other senders do not.
    pub(crate) fn private_access(&self, session_id: &str) -> bool {
        channel_of_session(session_id).is_none()
            || self
                .owner_sessions
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(session_id)
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
    messaging: messaging::MessagingState,
    owner_sessions: Arc<std::sync::RwLock<std::collections::BTreeSet<String>>>,
    // A strong self-sender would prevent shutdown from closing the actor.
    inbox: async_channel::WeakSender<Input>,
    probes: Vec<Arc<dyn LinkProbe>>,
    refresh_generation: u64,
    refresh_tasks: Vec<tokio::task::AbortHandle>,
    /// Bumped when a contact pull is replaced or cancelled, so a late result is dropped.
    contact_generation: u64,
    contact_task: Option<tokio::task::AbortHandle>,
}

impl Actor {
    async fn run(mut self, inbox: Receiver<Input>) {
        loop {
            let input = tokio::select! {
                input = inbox.recv() => {
                    let Ok(input) = input else {
                        self.stop_contact_pull();
                        break;
                    };
                    input
                },
                result = self.messaging.sends.join_next(), if !self.messaging.sends.is_empty() => {
                    if let Some(Ok((id, result))) = result { self.sent_message(&id, result).await; }
                    continue;
                }
            };
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
                Input::Messaging {
                    session_id,
                    turn,
                    request,
                    reply,
                } => {
                    if !reply.is_closed() {
                        let result = self.messaging_request(&session_id, turn, request).await;
                        let _ = reply.send(result);
                    }
                }
                Input::ContactsPulled { generation, pulled } => {
                    if generation == self.contact_generation {
                        match pulled {
                            Ok(snapshot) if snapshot.is_empty() => {
                                if self.state.people.people.is_empty() {
                                    self.state.people.note_empty_sync();
                                    self.publish_people().await;
                                }
                            }
                            Ok(snapshot) => {
                                self.fold_contacts(snapshot.contacts).await;
                                self.fold_groups(snapshot.groups).await;
                            }
                            Err(_) => {
                                self.state.people.note_pull_failed();
                                self.publish_people().await;
                            }
                        }
                    }
                }
                Input::ProbeDone { generation, report } => {
                    if generation == self.refresh_generation {
                        self.emit(ConnectionEvent::KindProbed(report)).await;
                    }
                }
            }
        }
        self.stop_refresh();
        self.whatsapp.stop().await;
        self.telegram.stop().await;
    }

    /// Drop every probe from the current refresh. A result that already landed stays.
    fn stop_refresh(&mut self) {
        for task in self.refresh_tasks.drain(..) {
            task.abort();
        }
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
    }

    /// Probe installed links together, at most [`PROBE_BOUND`] at once.
    ///
    /// Each probe is its own task, so a stalled link cannot delay another. Aborting
    /// the tasks drops their futures, which is what cancels an in-flight check.
    fn begin_refresh(&mut self) {
        self.stop_refresh();
        let generation = self.refresh_generation;
        let bound = Arc::new(tokio::sync::Semaphore::new(PROBE_BOUND));
        for probe in self.installed_probes() {
            let bound = Arc::clone(&bound);
            let inbox = self.inbox.clone();
            let task = self.handle.spawn(async move {
                let Ok(_permit) = bound.acquire_owned().await else {
                    return;
                };
                let Some(report) = probe.probe().await else {
                    return;
                };
                if let Some(inbox) = inbox.upgrade() {
                    let _ = inbox.try_send(Input::ProbeDone { generation, report });
                }
            });
            self.refresh_tasks.push(task.abort_handle());
        }
    }

    fn installed_probes(&self) -> Vec<Arc<dyn LinkProbe>> {
        let mut probes = self.probes.clone();
        if let Some(channel) = self.whatsapp.channel.clone()
            && channel.has_saved_link()
        {
            probes.push(Arc::new(SavedLinkProbe {
                kind: CatalogueKind::WhatsApp,
                channel,
            }));
        }
        if let Some(channel) = self.telegram.channel.clone()
            && channel.has_saved_link()
        {
            probes.push(Arc::new(SavedLinkProbe {
                kind: CatalogueKind::Telegram,
                channel,
            }));
        }
        probes
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
                for message in &mut self.state.outbound_messages {
                    use sovra_core::communications::MessageDelivery;
                    message.delivery = match &message.delivery {
                        MessageDelivery::Review => MessageDelivery::Expired,
                        MessageDelivery::Sending => MessageDelivery::Uncertain,
                        other => other.clone(),
                    };
                }
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
            ConnectionCommand::ReviewMessage {
                id,
                session_id,
                send,
            } => self.review_message(&id, &session_id, send).await,
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
            ConnectionCommand::PrepareMessage {
                session_id,
                channel,
                recipient,
                text,
            } => {
                if let Err(error) = self
                    .prepare_message(&session_id, 0, channel, &recipient, text)
                    .await
                {
                    self.emit(ConnectionEvent::Notice(error.to_string())).await;
                }
            }
            ConnectionCommand::SendScheduled {
                session_id,
                channel,
                recipient,
                text,
            } => {
                self.send_scheduled(&session_id, channel, &recipient, text)
                    .await;
            }
            ConnectionCommand::AttachHandle {
                person_id,
                reach,
                value,
            } => match self.state.people.attach(&person_id, reach, &value) {
                Ok(_) => self.publish_people().await,
                Err(message) => self.emit(ConnectionEvent::Notice(message)).await,
            },
            ConnectionCommand::LinkPeople { keep, merge } => {
                if self.state.people.link_people(&keep, &merge).is_some() {
                    self.publish_people().await;
                } else {
                    self.emit(ConnectionEvent::Notice(
                        "Choose two different saved people to join.".into(),
                    ))
                    .await;
                }
            }
            ConnectionCommand::DisconnectWhatsApp => {
                self.stop_contact_pull();
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
            ConnectionCommand::Refresh => self.begin_refresh(),
            ConnectionCommand::CancelRefresh => self.stop_refresh(),
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
                self.begin_contact_pull();
            }
            ChannelEvent::Offline { reason } => {
                self.stop_contact_pull();
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
                self.stop_contact_pull();
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
                self.stop_contact_pull();
                self.whatsapp.stop().await;
                if self.state.whatsapp.is_linked() {
                    self.emit(ConnectionEvent::Notice(message)).await;
                } else {
                    self.set_whatsapp(WhatsAppStatus::Error { message }).await;
                }
            }
            ChannelEvent::Message(message) => self.whatsapp_message(message).await,
            ChannelEvent::Contacts(contacts) => self.fold_contacts(contacts).await,
            ChannelEvent::Groups(groups) => self.fold_groups(groups).await,
        }
    }

    async fn whatsapp_message(&mut self, message: IncomingMessage) {
        let sending_echo = message.from_me
            && self.state.outbound_messages.iter().any(|outbound| {
                outbound.channel == ChannelKind::WhatsApp
                    && outbound.delivery == sovra_core::communications::MessageDelivery::Sending
                    && outbound.recipient == message.chat_jid
                    && outbound.text == message.text
            });
        if sending_echo || self.whatsapp.sent_ids.contains(&message.id) || !should_record(&message)
        {
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
        // Telegram has no address book, so a contact event must not invent chats.
        match event {
            ChannelEvent::Connecting
            | ChannelEvent::QrCode { .. }
            | ChannelEvent::PairingCode { .. }
            | ChannelEvent::PairingCodeFailed { .. }
            | ChannelEvent::Scanned { .. }
            | ChannelEvent::Contacts(_)
            | ChannelEvent::Groups(_) => {}
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
        let owner = sovra_core::connections::is_owner_message(kind, &message, &self.state);
        if owner {
            self.owner_sessions
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(session_id.clone());
        }
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
            | Event::RemoteControl(_)
            | Event::Development(_)
            | Event::Google(_)
            | Event::GitHub(_)
            | Event::Mcp(_)
            | Event::Terminal(_)
            | Event::Schedule(_)
            | Event::Group(_)
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
        if !status.is_linked() {
            self.owner_sessions
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|id| channel_of_session(id) != Some(ChannelKind::WhatsApp));
        }
        let event = ConnectionEvent::StatusChanged(status);
        self.state.apply(&event);
        self.emit(event).await;
    }

    async fn set_telegram(&mut self, status: TelegramStatus) {
        if !status.is_connected() {
            self.owner_sessions
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|id| channel_of_session(id) != Some(ChannelKind::Telegram));
        }
        let event = ConnectionEvent::TelegramStatusChanged(status);
        self.state.apply(&event);
        self.emit(event).await;
    }

    /// Replace any in-flight contact pull. The actor keeps serving other links.
    fn begin_contact_pull(&mut self) {
        self.stop_contact_pull();
        let Some(channel) = self.whatsapp.channel.clone() else {
            return;
        };
        let generation = self.contact_generation;
        let inbox = self.inbox.clone();
        let task = self.handle.spawn(async move {
            let pulled = channel.pull_contacts().await;
            if let Some(inbox) = inbox.upgrade() {
                let _ = inbox.try_send(Input::ContactsPulled { generation, pulled });
            }
        });
        self.contact_task = Some(task.abort_handle());
    }

    fn stop_contact_pull(&mut self) {
        if let Some(task) = self.contact_task.take() {
            task.abort();
        }
        self.contact_generation = self.contact_generation.wrapping_add(1);
    }

    async fn fold_contacts(&mut self, contacts: Vec<sovra_core::people::SyncedContact>) {
        if contacts.is_empty() {
            return;
        }
        let before = self.state.people.clone();
        for contact in &contacts {
            self.state.people.apply_synced(contact);
        }
        if self.state.people != before {
            self.publish_people().await;
        }
    }

    async fn fold_groups(&mut self, groups: Vec<sovra_core::connections::SyncedGroup>) {
        if groups.is_empty() {
            return;
        }
        if self.state.upsert_groups(&groups) {
            self.publish_groups().await;
        }
    }

    async fn publish_people(&mut self) {
        self.emit(ConnectionEvent::PeopleChanged(self.state.people.clone()))
            .await;
        self.save().await;
    }

    async fn publish_groups(&mut self) {
        self.emit(ConnectionEvent::GroupsChanged(self.state.groups.clone()))
            .await;
        self.save().await;
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
