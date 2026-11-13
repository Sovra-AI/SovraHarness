//! Turn-bound messaging access and native approval. The connections actor owns
//! drafts and send tasks, so neither model output nor a replay can approve a send.
//! Confirmed timed jobs transmit without a fire-time review.

use super::{Actor, ConnectionsRuntime, Input};
use serde_json::{Value, json};
use sovra_core::communications::{
    MAX_OUTBOUND_MESSAGES, MAX_SENDS_WITHOUT_REVIEW_PER_TURN, MessageDelivery, MessagingAccess,
    MessagingRequest, OutboundMessage, REVIEW_LIFETIME_MS, resolve_recipient, validate_message,
};
use sovra_core::connections::{
    ChannelError, ChannelKind, ChannelSession, ConnectionEvent, channel_of_session,
};
use sovra_core::{Command, ScheduleCommand};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{Semaphore, oneshot};
use tokio::task::JoinSet;

#[derive(Default)]
pub(super) struct MessagingState {
    bindings: BTreeMap<String, (u64, Arc<dyn ChannelSession>)>,
    pub sends: JoinSet<(String, Result<String, ChannelError>)>,
    /// Outbound id → schedule session id for confirmed timed sends.
    scheduled: BTreeMap<String, String>,
    /// Turn → sends that skipped review. Only recent turns are kept.
    unreviewed: BTreeMap<u64, usize>,
}

struct Caller {
    inbox: async_channel::WeakSender<Input>,
    session_id: String,
    turn: u64,
    admission: Arc<Semaphore>,
}

impl ConnectionsRuntime {
    pub(crate) fn messaging(&self, session_id: &str) -> Option<Arc<dyn MessagingAccess>> {
        if !self.messaging_available || channel_of_session(session_id).is_some() {
            return None;
        }
        Some(Arc::new(Caller {
            inbox: self.inbox.downgrade(),
            session_id: session_id.to_owned(),
            turn: self.next_turn.fetch_add(1, Ordering::Relaxed),
            admission: Arc::clone(&self.messaging_admission),
        }))
    }
}

#[async_trait::async_trait]
impl MessagingAccess for Caller {
    async fn call(&self, request: MessagingRequest) -> Result<Value, ChannelError> {
        let _permit = self
            .admission
            .try_acquire()
            .map_err(|_| unavailable("Messaging is busy. Wait for the current request."))?;
        let inbox = self
            .inbox
            .upgrade()
            .ok_or_else(|| unavailable("Messaging has stopped."))?;
        let (reply, receiver) = oneshot::channel();
        inbox
            .try_send(Input::Messaging {
                session_id: self.session_id.clone(),
                turn: self.turn,
                request,
                reply,
            })
            .map_err(|_| unavailable("Messaging has stopped."))?;
        tokio::time::timeout(Duration::from_secs(15), receiver)
            .await
            .map_err(|_| {
                unavailable(
                    "Messaging did not respond. Check the chat before requesting another draft.",
                )
            })?
            .map_err(|_| unavailable("Messaging has stopped."))?
    }
}

pub(super) fn counter() -> Arc<AtomicU64> {
    Arc::new(AtomicU64::new(1))
}
pub(super) fn admission() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(8))
}
fn unavailable(message: &str) -> ChannelError {
    ChannelError::Unavailable(message.into())
}

fn people_lookup(book: &sovra_core::people::PeopleBook, query: &str) -> Value {
    match book.lookup(query) {
        sovra_core::people::Lookup::Ask { count } => json!({
            "count": count,
            "ask": "Ask for a person's name. The full list is not sent."
        }),
        sovra_core::people::Lookup::None => json!({"matches": [], "ambiguous": false}),
        sovra_core::people::Lookup::One { name, channels } => json!({
            "matches": [{"name": name, "channels": channels}],
            "ambiguous": false
        }),
        sovra_core::people::Lookup::Many { names } => json!({"ambiguous": true, "names": names}),
    }
}

impl Actor {
    pub(super) async fn messaging_request(
        &mut self,
        session_id: &str,
        turn: u64,
        request: MessagingRequest,
    ) -> Result<Value, ChannelError> {
        if channel_of_session(session_id).is_some() {
            return Err(unavailable("Open the desktop chat to use messaging tools."));
        }
        match request {
            MessagingRequest::Status => Ok(json!({
                "whatsapp": {"connected": self.state.whatsapp.is_linked(), "online": self.state.whatsapp.is_connected()},
                "telegram": {"connected": self.state.telegram.is_linked(), "online": self.state.telegram.is_connected()},
                "guidance":"Connect or reconnect in Settings. Sending needs review in this desktop chat. Recent messages are a limited local snapshot, not the whole phone history.",
            })),
            MessagingRequest::RecentChats { channel } => {
                self.connected(channel)?;
                let mut seen = std::collections::BTreeSet::new();
                let chats: Vec<_> = self.state.recent_messages.iter().filter(|m| m.channel == channel && seen.insert(m.chat_jid.clone()))
                    .map(|m| json!({"chat":m.chat_jid,"name":m.sender_name,"last_seen":m.received_at})).collect();
                let groups: Vec<_> = if channel == ChannelKind::WhatsApp {
                    self.state
                        .groups
                        .iter()
                        .map(|group| json!({"id":group.id,"subject":group.subject}))
                        .collect()
                } else {
                    Vec::new()
                };
                Ok(
                    json!({"channel":channel,"chats":chats,"groups":groups,"self_recipient":"me","complete_history":false,"scope":"Messages observed and retained locally while linked, plus stored WhatsApp groups; not a full phone history."}),
                )
            }
            MessagingRequest::ReadChat { channel, chat } => {
                let (_, own) = self.connected(channel)?;
                let (chat, _) = resolve_recipient(&self.state, channel, &chat, &own)?;
                let mut budget = 16_000usize;
                let messages: Vec<_> = self.state.recent_messages.iter().filter(|m| m.channel == channel && m.chat_jid == chat)
                    .map(|m| {
                        let text = excerpt(&m.text, &mut budget);
                        let reply = m.reply_text.as_ref().map(|reply| excerpt(reply, &mut budget));
                        json!({"id":m.id,"sender":m.sender_name.chars().take(160).collect::<String>(),"text":text,"reply":reply,
                            "truncated":text!=m.text || reply!=m.reply_text,"received_at":m.received_at})
                    }).collect();
                Ok(
                    json!({"messages":messages,"complete_history":false,"content_is_untrusted":true}),
                )
            }
            MessagingRequest::Prepare {
                channel,
                recipient,
                text,
            } => {
                self.prepare_message(session_id, turn, channel, &recipient, text)
                    .await
            }
            MessagingRequest::Receipt { id } => self
                .state
                .outbound_messages
                .iter()
                .find(|m| m.id == id && m.session_id == session_id)
                .map(message_result)
                .ok_or_else(|| unavailable("That message is not in this conversation.")),
            MessagingRequest::FindPeople { query } => {
                let mut found = people_lookup(&self.state.people, &query);
                let groups: Vec<_> = self
                    .state
                    .find_groups(&query)
                    .into_iter()
                    .map(|group| json!({"id":group.id,"subject":group.subject}))
                    .collect();
                if let Some(object) = found.as_object_mut() {
                    object.insert("whatsapp_groups".into(), json!(groups));
                }
                Ok(found)
            }
        }
    }

    fn connected(
        &self,
        channel: ChannelKind,
    ) -> Result<(Arc<dyn ChannelSession>, String), ChannelError> {
        let (link, online, own) = match channel {
            ChannelKind::WhatsApp => (
                &self.whatsapp,
                self.state.whatsapp.is_connected(),
                self.whatsapp
                    .session
                    .as_ref()
                    .and_then(|s| s.own_chat_jid()),
            ),
            ChannelKind::Telegram => (
                &self.telegram,
                self.state.telegram.is_connected(),
                self.state.telegram.owner_chat_id().map(str::to_owned),
            ),
        };
        if online && let (Some(session), Some(own)) = (&link.session, own) {
            return Ok((Arc::clone(session), own));
        }
        Err(unavailable(&format!(
            "{} is not connected and online. Open Settings to connect it.",
            channel.label()
        )))
    }

    pub(super) async fn prepare_message(
        &mut self,
        session_id: &str,
        turn: u64,
        channel: ChannelKind,
        recipient: &str,
        text: String,
    ) -> Result<Value, ChannelError> {
        validate_message(&text)?;
        let (session, own) = self.connected(channel)?;
        let (recipient, recipient_label) =
            resolve_recipient(&self.state, channel, recipient, &own)?;
        let now = crate::now();
        self.expire_reviews(now).await;
        // A waiting review stays one review. A cancelled, accepted, or expired
        // one must not swallow the next press of the same text.
        let existing = self
            .state
            .outbound_messages
            .iter()
            .find(|m| {
                m.session_id == session_id
                    && m.channel == channel
                    && m.recipient == recipient
                    && m.text == text
                    && matches!(
                        m.delivery,
                        MessageDelivery::Review | MessageDelivery::Sending
                    )
                    && self
                        .messaging
                        .bindings
                        .get(&m.id)
                        .is_some_and(|(saved_turn, _)| *saved_turn == turn)
            })
            .cloned();
        if let Some(existing) = existing {
            self.publish_message(existing.clone()).await;
            return Ok(message_result(&existing));
        }
        if self
            .state
            .outbound_messages
            .iter()
            .filter(|m| {
                matches!(
                    m.delivery,
                    MessageDelivery::Review | MessageDelivery::Sending
                )
            })
            .count()
            >= 8
        {
            return Err(unavailable(
                "Review or cancel the waiting messages before preparing more.",
            ));
        }
        // Prune only terminal history. An outstanding approval must never disappear.
        if self.state.outbound_messages.len() >= MAX_OUTBOUND_MESSAGES
            && let Some(index) = self.state.outbound_messages.iter().position(|m| {
                !matches!(
                    m.delivery,
                    MessageDelivery::Review | MessageDelivery::Sending
                )
            })
        {
            let removed = self.state.outbound_messages.remove(index);
            self.messaging.bindings.remove(&removed.id);
        }
        let id = new_outbound_id()?;
        let without_review = self.may_skip_review(turn, channel, &recipient);
        let message = OutboundMessage {
            id: id.clone(),
            session_id: session_id.into(),
            channel,
            sender: match channel {
                ChannelKind::WhatsApp => self
                    .state
                    .whatsapp
                    .linked_device()
                    .map_or(own, |device| device.phone_display.clone()),
                ChannelKind::Telegram => format!(
                    "@{}",
                    self.state.telegram.bot_username().unwrap_or("your bot")
                ),
            },
            recipient,
            recipient_label,
            text,
            delivery: if without_review {
                MessageDelivery::Sending
            } else {
                MessageDelivery::Review
            },
            created_at: now,
            expires_at: sovra_core::Timestamp(now.0.saturating_add(REVIEW_LIFETIME_MS)),
        };
        self.messaging
            .bindings
            .insert(id, (turn, Arc::clone(&session)));
        if !self.publish_message(message.clone()).await {
            let mut failed = message;
            failed.delivery = MessageDelivery::NotSent;
            self.publish_message(failed).await;
            return Err(unavailable(
                "The message could not be saved for review. Nothing was sent.",
            ));
        }
        if without_review {
            *self.messaging.unreviewed.entry(turn).or_default() += 1;
            self.spawn_send(
                message.id.clone(),
                message.recipient.clone(),
                message.text.clone(),
                session,
            );
            let mut result = message_result(&message);
            if let Some(object) = result.as_object_mut() {
                object.insert(
                    "sent_without_review".into(),
                    json!("The person allowed this chat without asking, so it is sending now. Check message_receipt before saying it was sent."),
                );
            }
            return Ok(result);
        }
        Ok(message_result(&message))
    }

    /// A chat the person trusted skips review, up to a few sends per turn.
    /// Turn 0 is a UI-prepared draft, which always shows its review.
    fn may_skip_review(&mut self, turn: u64, channel: ChannelKind, recipient: &str) -> bool {
        if turn == 0 || !self.state.is_trusted(channel, recipient) {
            return false;
        }
        let floor = turn.saturating_sub(64);
        self.messaging.unreviewed.retain(|saved, _| *saved >= floor);
        self.messaging.unreviewed.get(&turn).copied().unwrap_or(0)
            < MAX_SENDS_WITHOUT_REVIEW_PER_TURN
    }

    pub(super) async fn trust_recipient(
        &mut self,
        channel: ChannelKind,
        recipient: &str,
        trusted: bool,
    ) {
        if !self.state.set_trusted(channel, recipient, trusted) {
            return;
        }
        let event = ConnectionEvent::TrustChanged(self.state.trusted_recipients.clone());
        self.emit(event).await;
        self.save().await;
    }

    pub(super) async fn review_message(&mut self, id: &str, session_id: &str, send: bool) {
        let now = crate::now();
        self.expire_reviews(now).await;
        let Some(mut message) = self
            .state
            .outbound_messages
            .iter()
            .find(|m| m.id == id && m.may_review(session_id, now))
            .cloned()
        else {
            return;
        };
        if !send {
            message.delivery = MessageDelivery::Cancelled;
            self.publish_message(message).await;
            return;
        }
        let Ok((session, _)) = self.connected(message.channel) else {
            message.delivery = MessageDelivery::Expired;
            self.publish_message(message).await;
            return;
        };
        if !self
            .messaging
            .bindings
            .get(id)
            .is_some_and(|(_, bound)| Arc::ptr_eq(&session, bound))
        {
            message.delivery = MessageDelivery::Expired;
            self.publish_message(message).await;
            return;
        }
        // Consume the review before starting IO. Duplicate clicks and stale commands
        // cannot enter the transport a second time, including after a timeout.
        message.delivery = MessageDelivery::Sending;
        if !self.publish_message(message.clone()).await {
            message.delivery = MessageDelivery::NotSent;
            self.publish_message(message).await;
            return;
        }
        self.spawn_send(message.id, message.recipient, message.text, session);
    }

    pub(super) async fn send_scheduled(
        &mut self,
        session_id: &str,
        channel: ChannelKind,
        recipient: &str,
        text: String,
    ) {
        if let Err(error) = self
            .start_scheduled_send(session_id, channel, recipient, text)
            .await
        {
            tracing::warn!(session_id, error = %error, "scheduled send failed");
            self.emit(ConnectionEvent::Notice(error.to_string())).await;
            self.finish_scheduled_run(session_id, false, Some(error.to_string()))
                .await;
        }
    }

    async fn start_scheduled_send(
        &mut self,
        session_id: &str,
        channel: ChannelKind,
        recipient: &str,
        text: String,
    ) -> Result<(), ChannelError> {
        validate_message(&text)?;
        let (session, own) = self.connected(channel)?;
        let (recipient, recipient_label) =
            resolve_recipient(&self.state, channel, recipient, &own)?;
        let now = crate::now();
        let id = new_outbound_id()?;
        let sender = match channel {
            ChannelKind::WhatsApp => self
                .state
                .whatsapp
                .linked_device()
                .map_or(own, |device| device.phone_display.clone()),
            ChannelKind::Telegram => format!(
                "@{}",
                self.state.telegram.bot_username().unwrap_or("your bot")
            ),
        };
        let message = OutboundMessage {
            id: id.clone(),
            session_id: session_id.into(),
            channel,
            sender,
            recipient: recipient.clone(),
            recipient_label,
            text: text.clone(),
            delivery: MessageDelivery::Sending,
            created_at: now,
            expires_at: sovra_core::Timestamp(now.0.saturating_add(REVIEW_LIFETIME_MS)),
        };
        self.messaging
            .bindings
            .insert(id.clone(), (0, session.clone()));
        self.messaging
            .scheduled
            .insert(id.clone(), session_id.to_owned());
        if !self.publish_message(message).await {
            self.messaging.bindings.remove(&id);
            self.messaging.scheduled.remove(&id);
            return Err(unavailable(
                "The timed message could not be saved. Nothing was sent.",
            ));
        }
        self.spawn_send(id, recipient, text, session);
        Ok(())
    }

    fn spawn_send(
        &mut self,
        id: String,
        recipient: String,
        text: String,
        session: Arc<dyn ChannelSession>,
    ) {
        self.messaging.sends.spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(30),
                session.send_text(&recipient, &text),
            )
            .await
            .unwrap_or_else(|_| Err(unavailable("Sending could not be confirmed.")));
            (id, result)
        });
    }

    async fn finish_scheduled_run(&self, session_id: &str, succeeded: bool, error: Option<String>) {
        let _ = self
            .commands
            .send(Command::Schedule(ScheduleCommand::FinishRun {
                session_id: session_id.to_owned(),
                succeeded,
                error,
            }))
            .await;
    }

    pub(super) async fn sent_message(&mut self, id: &str, result: Result<String, ChannelError>) {
        let Some(mut message) = self
            .state
            .outbound_messages
            .iter()
            .find(|m| m.id == id)
            .cloned()
        else {
            return;
        };
        let error = result.as_ref().err().map(ToString::to_string);
        message.delivery = match result {
            Ok(message_id) => {
                self.link(message.channel).remember_sent(message_id.clone());
                MessageDelivery::Accepted { message_id }
            }
            Err(_) => MessageDelivery::Uncertain,
        };
        let succeeded = matches!(message.delivery, MessageDelivery::Accepted { .. });
        self.publish_message(message).await;
        if let Some(session_id) = self.messaging.scheduled.remove(id) {
            self.finish_scheduled_run(&session_id, succeeded, error)
                .await;
        }
    }

    async fn publish_message(&mut self, message: OutboundMessage) -> bool {
        let event = ConnectionEvent::OutboundChanged(message);
        self.state.apply(&event);
        let persisted = match &self.store {
            Some(store) => store.save_connections(&self.state).await.is_ok(),
            None => true,
        };
        self.emit(event).await;
        persisted
    }

    async fn expire_reviews(&mut self, now: sovra_core::Timestamp) {
        let expired: Vec<_> = self
            .state
            .outbound_messages
            .iter()
            .filter(|m| m.delivery == MessageDelivery::Review && now.0 >= m.expires_at.0)
            .cloned()
            .collect();
        for mut message in expired {
            message.delivery = MessageDelivery::Expired;
            self.publish_message(message).await;
        }
    }
}

fn new_outbound_id() -> Result<String, ChannelError> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| unavailable("Could not prepare a message. Try again."))?;
    let mut id = String::with_capacity(random.len() * 2);
    for byte in random {
        use std::fmt::Write as _;
        let _ = write!(id, "{byte:02x}");
    }
    Ok(id)
}

fn message_result(message: &OutboundMessage) -> Value {
    json!({"id":message.id,"channel":message.channel,"recipient":message.recipient,"text":message.text,
        "status":message.delivery,"explanation":message.delivery.label(),"sent":matches!(message.delivery,MessageDelivery::Accepted {..})})
}

fn excerpt(text: &str, remaining: &mut usize) -> String {
    let result: String = text.chars().take((*remaining).min(4096)).collect();
    *remaining = remaining.saturating_sub(result.chars().count());
    result
}

#[cfg(test)]
mod tests {
    use super::super::Link;
    use super::*;
    use sovra_core::Timestamp;
    use sovra_core::connections::{
        BridgedMessage, ConnectionsState, LinkedDevice, TelegramStatus, WhatsAppStatus,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct Session {
        sent: Mutex<Vec<(String, String)>>,
        fail: bool,
    }
    #[async_trait::async_trait]
    impl ChannelSession for Session {
        async fn send_text(&self, chat: &str, text: &str) -> Result<String, ChannelError> {
            self.sent.lock().unwrap().push((chat.into(), text.into()));
            if self.fail {
                Err(unavailable("transport failed"))
            } else {
                Ok("receipt".into())
            }
        }
        async fn logout(&self) {}
        async fn stop(&self) {}
        fn own_chat_jid(&self) -> Option<String> {
            Some("15550000000@s.whatsapp.net".into())
        }
    }
    fn actor(session: Arc<Session>) -> Actor {
        let (inbox, _) = async_channel::unbounded();
        let (events, _) = async_channel::unbounded();
        let (commands, _) = async_channel::unbounded();
        let mut whatsapp = Link::new(ChannelKind::WhatsApp, None, &inbox);
        let mut telegram = Link::new(ChannelKind::Telegram, None, &inbox);
        whatsapp.session = Some(session.clone());
        telegram.session = Some(session);
        Actor {
            owner_sessions: Arc::new(std::sync::RwLock::new(std::collections::BTreeSet::new())),
            store: None,
            events,
            commands,
            handle: tokio::runtime::Handle::current(),
            state: ConnectionsState {
                whatsapp: WhatsAppStatus::Connected {
                    device: LinkedDevice {
                        phone_display: "+15550000000".into(),
                        device_jid: "15550000000:1@s.whatsapp.net".into(),
                        companion_name: "fixture".into(),
                        linked_at: Timestamp(0),
                    },
                    online: true,
                },
                telegram: TelegramStatus::Connected {
                    bot_username: "fixture_bot".into(),
                    owner_chat_id: "777".into(),
                    owner_name: "Fixture".into(),
                    linked_at: Timestamp(0),
                    online: true,
                },
                ..Default::default()
            },
            whatsapp,
            telegram,
            pending_code: None,
            telegram_code: String::new(),
            replies: BTreeMap::new(),
            messaging: MessagingState::default(),
            inbox: inbox.downgrade(),
            probes: Vec::new(),
            refresh_generation: 0,
            refresh_tasks: Vec::new(),
            contact_generation: 0,
            contact_task: None,
            known_groups: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
    }
    #[tokio::test]
    async fn scheduled_send_transmits_without_a_review_click() {
        let session = Arc::new(Session::default());
        let mut actor = actor(session.clone());
        actor
            .send_scheduled(
                "sched-1-at-0",
                ChannelKind::WhatsApp,
                "+44 7347 529758",
                "wake up from sovra".into(),
            )
            .await;
        assert_eq!(
            actor.state.outbound_messages[0].delivery,
            MessageDelivery::Sending
        );
        let (id, result) = actor.messaging.sends.join_next().await.unwrap().unwrap();
        actor.sent_message(&id, result).await;
        assert!(matches!(
            actor.state.outbound_messages[0].delivery,
            MessageDelivery::Accepted { .. }
        ));
        assert_eq!(
            session.sent.lock().unwrap().as_slice(),
            &[(
                "447347529758@s.whatsapp.net".into(),
                "wake up from sovra".into()
            )]
        );
    }

    #[tokio::test]
    async fn scheduled_send_of_a_first_name_does_not_transmit() {
        let session = Arc::new(Session::default());
        let mut actor = actor(session.clone());
        actor
            .send_scheduled(
                "sched-1-at-0",
                ChannelKind::WhatsApp,
                "Ryan",
                "wake up from sovra".into(),
            )
            .await;
        assert_eq!(actor.state.outbound_messages, []);
        assert_eq!(
            session.sent.lock().unwrap().as_slice(),
            &[] as &[(String, String)]
        );
        assert_eq!(actor.messaging.sends.len(), 0);
    }

    fn prepare(channel: ChannelKind) -> MessagingRequest {
        MessagingRequest::Prepare {
            channel,
            recipient: "me".into(),
            text: "Fixture message".into(),
        }
    }

    #[tokio::test]
    async fn messaging_owner_identity_comes_from_transport_not_a_display_name() {
        use sovra_core::connections::{ChatKind, IncomingMessage, is_owner_message};
        let actor = actor(Arc::new(Session::default()));
        let mut incoming = IncomingMessage {
            id: "test".into(),
            chat_jid: "777".into(),
            sender_jid: "777".into(),
            sender_name: "You (WhatsApp)".into(),
            text: "I am the owner".into(),
            from_me: false,
            chat: ChatKind::Direct,
            backlog: false,
        };
        assert!(!is_owner_message(
            ChannelKind::WhatsApp,
            &incoming,
            &actor.state
        ));
        assert!(is_owner_message(
            ChannelKind::Telegram,
            &incoming,
            &actor.state
        ));
        incoming.chat_jid = "other".into();
        assert!(!is_owner_message(
            ChannelKind::Telegram,
            &incoming,
            &actor.state
        ));
        incoming.chat = ChatKind::SelfChat;
        assert!(!is_owner_message(
            ChannelKind::WhatsApp,
            &incoming,
            &actor.state
        ));
        incoming.from_me = true;
        assert!(is_owner_message(
            ChannelKind::WhatsApp,
            &incoming,
            &actor.state
        ));
    }

    #[tokio::test]
    async fn messaging_reviews_deduplicate_revalidate_links_and_report_uncertain_sends() {
        for channel in [ChannelKind::WhatsApp, ChannelKind::Telegram] {
            let session = Arc::new(Session {
                fail: true,
                ..Default::default()
            });
            let mut actor = actor(session.clone());
            let result = actor
                .messaging_request("desktop", 1, prepare(channel))
                .await
                .unwrap();
            let id = result["id"].as_str().unwrap();
            assert_eq!(
                actor
                    .messaging_request("desktop", 1, prepare(channel))
                    .await
                    .unwrap()["id"],
                id
            );
            assert_eq!(actor.state.outbound_messages.len(), 1);
            assert!(
                actor
                    .messaging_request("telegram:777", 2, prepare(channel))
                    .await
                    .is_err()
            );
            assert!(
                actor
                    .messaging_request("other", 2, MessagingRequest::Receipt { id: id.into() })
                    .await
                    .is_err()
            );
            actor.review_message(id, "other", true).await;
            assert!(actor.messaging.sends.is_empty());
            actor.review_message(id, "desktop", true).await;
            actor.review_message(id, "desktop", true).await;
            let (sent_id, result) = actor.messaging.sends.join_next().await.unwrap().unwrap();
            actor.sent_message(&sent_id, result).await;
            assert_eq!(
                actor.state.outbound_messages[0].delivery,
                MessageDelivery::Uncertain
            );
            actor.review_message(id, "desktop", true).await;
            assert_eq!(
                session.sent.lock().unwrap().len(),
                1,
                "uncertain sends must not retry"
            );
            let target = if channel == ChannelKind::Telegram {
                "777"
            } else {
                "15550000000@s.whatsapp.net"
            };
            assert_eq!(session.sent.lock().unwrap()[0].0, target);
            // A new login must not reuse approval prepared for the old account.
            let fresh = actor
                .messaging_request("desktop", 3, prepare(channel))
                .await
                .unwrap();
            let fresh = fresh["id"].as_str().unwrap();
            actor.link(channel).session = Some(Arc::new(Session::default()));
            actor.review_message(fresh, "desktop", true).await;
            assert_eq!(
                actor.state.outbound_messages[1].delivery,
                MessageDelivery::Expired
            );
            assert!(actor.messaging.sends.is_empty());
        }
    }

    #[tokio::test]
    async fn a_second_press_prepares_a_new_review_after_the_first_is_finished() {
        use sovra_core::connections::ConnectionCommand;
        #[derive(Clone, Copy)]
        enum Finish {
            Cancel,
            Accept,
            Expire,
        }
        for finish in [Finish::Cancel, Finish::Accept, Finish::Expire] {
            let session = Arc::new(Session::default());
            let mut actor = actor(session.clone());
            let prepare = ConnectionCommand::PrepareMessage {
                session_id: "desktop".into(),
                channel: ChannelKind::WhatsApp,
                recipient: "me".into(),
                text: "The school gate opens at 3.".into(),
            };
            actor.command(prepare.clone()).await;
            assert_eq!(actor.state.outbound_messages.len(), 1);
            let first = actor.state.outbound_messages[0].id.clone();
            assert_eq!(
                actor.state.outbound_messages[0].delivery,
                MessageDelivery::Review
            );
            match finish {
                Finish::Cancel => actor.review_message(&first, "desktop", false).await,
                Finish::Accept => {
                    actor.review_message(&first, "desktop", true).await;
                    let (sent_id, result) =
                        actor.messaging.sends.join_next().await.unwrap().unwrap();
                    actor.sent_message(&sent_id, result).await;
                }
                Finish::Expire => {
                    actor.link(ChannelKind::WhatsApp).session = Some(Arc::new(Session::default()));
                    actor.review_message(&first, "desktop", true).await;
                }
            }
            assert_ne!(
                actor.state.outbound_messages[0].delivery,
                MessageDelivery::Review
            );
            let sent_before = session.sent.lock().unwrap().len();
            actor.command(prepare.clone()).await;
            assert_eq!(actor.state.outbound_messages.len(), 2);
            let second = actor.state.outbound_messages[1].id.clone();
            assert_ne!(second, first);
            assert_eq!(
                actor.state.outbound_messages[1].delivery,
                MessageDelivery::Review
            );
            assert_eq!(
                actor.state.outbound_messages[1].text,
                "The school gate opens at 3."
            );
            actor.command(prepare).await;
            assert_eq!(actor.state.outbound_messages.len(), 2);
            assert_eq!(actor.state.outbound_messages[1].id, second);
            assert_eq!(session.sent.lock().unwrap().len(), sent_before);
        }
    }

    #[tokio::test]
    async fn messaging_recent_reads_are_channel_scoped_and_disconnection_revokes_access() {
        let mut actor = actor(Arc::new(Session::default()));
        for channel in [ChannelKind::WhatsApp, ChannelKind::Telegram] {
            actor.state.recent_messages.push(BridgedMessage {
                id: channel.label().into(),
                channel,
                chat_jid: "777".into(),
                sender_name: "Sam".into(),
                sender_jid: "777".into(),
                text: format!("{} fixture", channel.label()),
                reply_text: None,
                received_at: Timestamp(5),
            });
        }
        let read = actor
            .messaging_request(
                "desktop",
                1,
                MessagingRequest::ReadChat {
                    channel: ChannelKind::Telegram,
                    chat: "777".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(read["messages"].as_array().unwrap().len(), 1);
        assert_eq!(read["messages"][0]["text"], "Telegram fixture");
        assert_eq!(read["complete_history"], false);
        actor.state.telegram = TelegramStatus::Disconnected;
        assert!(
            actor
                .messaging_request("desktop", 1, prepare(ChannelKind::Telegram))
                .await
                .is_err()
        );
        assert!(
            actor
                .messaging_request(
                    "desktop",
                    1,
                    MessagingRequest::RecentChats {
                        channel: ChannelKind::Telegram
                    }
                )
                .await
                .is_err()
        );
    }

    fn to_group(text: &str) -> MessagingRequest {
        MessagingRequest::Prepare {
            channel: ChannelKind::WhatsApp,
            recipient: "school gate".into(),
            text: text.into(),
        }
    }

    #[tokio::test]
    async fn a_group_found_by_part_of_its_name_gets_a_review_then_trust_sends_without_one() {
        let session = Arc::new(Session::default());
        let mut actor = actor(session.clone());
        actor
            .state
            .upsert_groups(&[sovra_core::connections::SyncedGroup {
                id: "120363111@g.us".into(),
                subject: "School Gate".into(),
            }]);
        let found = actor
            .messaging_request(
                "desktop",
                1,
                MessagingRequest::FindPeople {
                    query: "school".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(found["whatsapp_groups"][0]["id"], "120363111@g.us");

        let first = actor
            .messaging_request("desktop", 1, to_group("First"))
            .await
            .unwrap();
        assert_eq!(first["status"], json!("Review"));
        assert!(first.get("sent_without_review").is_none());
        let id = first["id"].as_str().unwrap().to_owned();
        actor
            .trust_recipient(ChannelKind::WhatsApp, "120363111@g.us", true)
            .await;
        assert!(
            actor.state.trusted_recipients.is_empty(),
            "a waiting review is not a send, so it cannot be trusted yet"
        );
        actor.review_message(&id, "desktop", true).await;
        let (sent_id, result) = actor.messaging.sends.join_next().await.unwrap().unwrap();
        actor.sent_message(&sent_id, result).await;
        actor
            .trust_recipient(ChannelKind::WhatsApp, "120363111@g.us", true)
            .await;
        assert!(
            actor
                .state
                .is_trusted(ChannelKind::WhatsApp, "120363111@g.us")
        );

        for (index, text) in ["Two", "Three", "Four"].into_iter().enumerate() {
            let result = actor
                .messaging_request("desktop", 2, to_group(text))
                .await
                .unwrap();
            assert_eq!(result["status"], json!("Sending"), "{text}");
            assert!(result.get("sent_without_review").is_some());
            let (sent_id, result) = actor.messaging.sends.join_next().await.unwrap().unwrap();
            actor.sent_message(&sent_id, result).await;
            assert_eq!(session.sent.lock().unwrap().len(), index + 2);
        }
        let capped = actor
            .messaging_request("desktop", 2, to_group("Five"))
            .await
            .unwrap();
        assert_eq!(
            capped["status"],
            json!("Review"),
            "a burst in one turn falls back to a review"
        );
        assert_eq!(actor.messaging.sends.len(), 0);

        actor
            .trust_recipient(ChannelKind::WhatsApp, "120363111@g.us", false)
            .await;
        let asked = actor
            .messaging_request("desktop", 3, to_group("Six"))
            .await
            .unwrap();
        assert_eq!(asked["status"], json!("Review"));
        assert_eq!(session.sent.lock().unwrap().len(), 4);
    }
}
