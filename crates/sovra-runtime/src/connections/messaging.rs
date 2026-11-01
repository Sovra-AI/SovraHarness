//! Turn-bound messaging access and native approval. The connections actor owns
//! drafts and send tasks, so neither model output nor a replay can approve a send.

use super::{Actor, ConnectionsRuntime, Input};
use serde_json::{Value, json};
use sovra_core::communications::{
    MAX_OUTBOUND_MESSAGES, MessageDelivery, MessagingAccess, MessagingRequest, OutboundMessage,
    REVIEW_LIFETIME_MS, resolve_recipient, validate_message,
};
use sovra_core::connections::{
    ChannelError, ChannelKind, ChannelSession, ConnectionEvent, channel_of_session,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{Semaphore, oneshot};
use tokio::task::JoinSet;

#[derive(Default)]
pub(super) struct MessagingState {
    bindings: BTreeMap<String, (u64, Arc<dyn ChannelSession>)>,
    pub sends: JoinSet<(String, Result<String, ChannelError>)>,
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
                Ok(
                    json!({"channel":channel,"chats":chats,"self_recipient":"me","complete_history":false,"scope":"Messages observed and retained locally while linked; not a contact book."}),
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

    async fn prepare_message(
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
        if let Some(existing) = self.state.outbound_messages.iter().find(|m| {
            m.session_id == session_id
                && m.channel == channel
                && m.recipient == recipient
                && m.text == text
                && self
                    .messaging
                    .bindings
                    .get(&m.id)
                    .is_some_and(|(saved_turn, _)| *saved_turn == turn)
        }) {
            return Ok(message_result(existing));
        }
        let now = crate::now();
        self.expire_reviews(now).await;
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
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| unavailable("Could not prepare a message. Try again."))?;
        let mut id = String::with_capacity(random.len() * 2);
        for byte in random {
            use std::fmt::Write as _;
            let _ = write!(id, "{byte:02x}");
        }
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
            delivery: MessageDelivery::Review,
            created_at: now,
            expires_at: sovra_core::Timestamp(now.0.saturating_add(REVIEW_LIFETIME_MS)),
        };
        self.messaging.bindings.insert(id, (turn, session));
        if !self.publish_message(message.clone()).await {
            let mut failed = message;
            failed.delivery = MessageDelivery::NotSent;
            self.publish_message(failed).await;
            return Err(unavailable(
                "The message could not be saved for review. Nothing was sent.",
            ));
        }
        Ok(message_result(&message))
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
        self.messaging.sends.spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(30),
                session.send_text(&message.recipient, &message.text),
            )
            .await
            .unwrap_or_else(|_| Err(unavailable("Sending could not be confirmed.")));
            (message.id, result)
        });
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
        message.delivery = match result {
            Ok(message_id) => {
                self.link(message.channel).remember_sent(message_id.clone());
                MessageDelivery::Accepted { message_id }
            }
            Err(_) => MessageDelivery::Uncertain,
        };
        self.publish_message(message).await;
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
        }
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
}
