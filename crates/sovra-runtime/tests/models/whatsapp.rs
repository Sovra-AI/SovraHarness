//! The `WhatsApp` link through the real `Client`, with a scripted channel adapter in place
//! of `WhatsApp`'s servers: pairing events reach the UI as status changes, a message in the
//! "Message yourself" chat is answered on the phone, the answer never reaches the chat
//! window, and unlinking logs out and forgets the saved link.

use super::*;
use sovra_core::connections::{
    ANSWER_FAILED_REPLY, ChannelError, ChannelEvent, ChannelSession, ChannelSink,
    ChannelStartOptions, ChatKind, ConnectionCommand, ConnectionEvent, IncomingMessage,
    LinkedDevice, MessagingChannel, WhatsAppStatus,
};

const OWN_CHAT: &str = "447700900123@s.whatsapp.net";

#[derive(Default)]
struct ScriptedChannel {
    sink: Mutex<Option<ChannelSink>>,
    session: Arc<ScriptedSession>,
    starts: AtomicU64,
    forgotten: AtomicU64,
}

#[derive(Default)]
struct ScriptedSession {
    sent: Mutex<Vec<(String, String)>>,
    logged_out: AtomicBool,
    stopped: AtomicBool,
}

#[async_trait::async_trait]
impl MessagingChannel for ScriptedChannel {
    async fn start(
        &self,
        _options: ChannelStartOptions,
        events: ChannelSink,
    ) -> Result<Arc<dyn ChannelSession>, ChannelError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        *self.sink.lock().unwrap() = Some(events);
        Ok(self.session.clone())
    }

    fn has_saved_link(&self) -> bool {
        false
    }

    fn forget(&self) -> Result<(), ChannelError> {
        self.forgotten.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait::async_trait]
impl ChannelSession for ScriptedSession {
    async fn send_text(&self, chat_jid: &str, text: &str) -> Result<String, ChannelError> {
        let mut sent = self.sent.lock().unwrap();
        sent.push((chat_jid.to_owned(), text.to_owned()));
        Ok(format!("SENT{}", sent.len()))
    }

    async fn logout(&self) {
        self.logged_out.store(true, Ordering::SeqCst);
    }

    async fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    fn own_chat_jid(&self) -> Option<String> {
        Some(OWN_CHAT.to_owned())
    }
}

impl ScriptedChannel {
    fn emit(&self, event: ChannelEvent) {
        let sink = self.sink.lock().unwrap().clone().expect("link started");
        sink(event);
    }
}

async fn status(client: &Client, matches: impl Fn(&WhatsAppStatus) -> bool) -> WhatsAppStatus {
    match event(
        client,
        |e| matches!(e, Event::Connections(ConnectionEvent::StatusChanged(s)) if matches(s)),
    )
    .await
    {
        Event::Connections(ConnectionEvent::StatusChanged(status)) => status,
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn whatsapp_link_pairs_answers_on_the_phone_and_unlinks() {
    let channel = Arc::new(ScriptedChannel::default());
    let client = sovra_runtime::start_with_channels(
        "wa-test",
        AgentConfig::default(),
        None,
        None,
        None,
        None,
        None,
        Some(channel.clone()),
    )
    .unwrap();
    event(&client, |e| matches!(e, Event::Ready { .. })).await;

    client
        .send(Command::Connections(
            ConnectionCommand::StartWhatsAppPairing,
        ))
        .unwrap();
    status(&client, |s| {
        matches!(s, WhatsAppStatus::Connecting { pairing: true })
    })
    .await;
    // Starting a fresh link opens the adapter once and clears stale keys first.
    tokio::time::timeout(Duration::from_secs(5), async {
        while channel.sink.lock().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(channel.starts.load(Ordering::SeqCst), 1);
    assert_eq!(channel.forgotten.load(Ordering::SeqCst), 1);

    channel.emit(ChannelEvent::QrCode {
        payload: "2@ref,noise,identity,adv,7".into(),
        matrix: vec![vec![true, false], vec![false, true]],
        expires_in_secs: 60,
    });
    let awaiting = status(&client, |s| {
        matches!(s, WhatsAppStatus::AwaitingQrScan { .. })
    })
    .await;
    let WhatsAppStatus::AwaitingQrScan { ticket } = awaiting else {
        unreachable!()
    };
    assert_eq!(ticket.qr_payload, "2@ref,noise,identity,adv,7");
    assert_eq!(ticket.qr_matrix.len(), 2);

    channel.emit(ChannelEvent::PairingCode {
        code: "ABCD1234".into(),
        expires_in_secs: 180,
    });
    let with_code = status(
        &client,
        |s| matches!(s, WhatsAppStatus::AwaitingQrScan { ticket } if ticket.pairing_code.is_some()),
    )
    .await;
    let WhatsAppStatus::AwaitingQrScan { ticket } = with_code else {
        unreachable!()
    };
    assert_eq!(ticket.pairing_code.as_deref(), Some("ABCD1234"));
    assert_eq!(
        ticket.qr_matrix.len(),
        2,
        "the code joins the QR, it does not replace it"
    );

    channel.emit(ChannelEvent::Scanned {
        device_label: "Android".into(),
    });
    status(&client, |s| {
        matches!(s, WhatsAppStatus::VerifyingHandshake { .. })
    })
    .await;

    channel.emit(ChannelEvent::Connected {
        device: LinkedDevice {
            phone_display: "+447700900123".into(),
            device_jid: "447700900123:7@s.whatsapp.net".into(),
            companion_name: "Sovra".into(),
            linked_at: sovra_core::Timestamp(0),
        },
    });
    let connected = status(&client, WhatsAppStatus::is_connected).await;
    let WhatsAppStatus::Connected { device, .. } = connected else {
        unreachable!()
    };
    assert!(
        device.linked_at.millis() > 0,
        "linked_at is stamped by the runtime"
    );

    // A message in the "Message yourself" chat is recorded and answered on the phone.
    channel.emit(ChannelEvent::Message(IncomingMessage {
        id: "MSG1".into(),
        chat_jid: OWN_CHAT.into(),
        sender_jid: OWN_CHAT.into(),
        sender_name: "Sam".into(),
        text: "What's on today?".into(),
        from_me: true,
        chat: ChatKind::SelfChat,
        backlog: false,
    }));
    let bridged = event(
        &client,
        |e| matches!(e, Event::Connections(ConnectionEvent::MessageBridged(m)) if m.id == "MSG1"),
    )
    .await;
    let Event::Connections(ConnectionEvent::MessageBridged(message)) = bridged else {
        unreachable!()
    };
    assert_eq!(message.sender_name, "You (WhatsApp)");
    // No model is configured in this runtime, so the phone gets the plain fallback and
    // the chat window never sees the WhatsApp session's events.
    let mut leaked = Vec::new();
    let reply = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let e = client.events().recv().await.unwrap();
            if e.session_id().is_some_and(|id| id.starts_with("whatsapp:")) {
                leaked.push(format!("{e:?}"));
            }
            if let Event::Connections(ConnectionEvent::ReplySent { message_id, reply }) = e
                && message_id == "MSG1"
            {
                return reply;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(reply, ANSWER_FAILED_REPLY);
    assert!(
        leaked.is_empty(),
        "chat window saw WhatsApp events: {leaked:?}"
    );
    assert_eq!(
        channel.session.sent.lock().unwrap().as_slice(),
        [(OWN_CHAT.to_owned(), ANSWER_FAILED_REPLY.to_owned())]
    );

    // Someone else's message is recorded but, with the safe default, not answered.
    channel.emit(ChannelEvent::Message(IncomingMessage {
        id: "MSG2".into(),
        chat_jid: "447700900999@s.whatsapp.net".into(),
        sender_jid: "447700900999@s.whatsapp.net".into(),
        sender_name: "Alex".into(),
        text: "Hi!".into(),
        from_me: false,
        chat: ChatKind::Direct,
        backlog: false,
    }));
    event(
        &client,
        |e| matches!(e, Event::Connections(ConnectionEvent::MessageBridged(m)) if m.id == "MSG2"),
    )
    .await;
    client
        .send(Command::Connections(
            ConnectionCommand::SendTestWhatsAppMessage {
                text: "ping".into(),
            },
        ))
        .unwrap();
    event(
        &client,
        |e| matches!(e, Event::Connections(ConnectionEvent::Notice(n)) if n.starts_with("Sent.")),
    )
    .await;
    assert_eq!(channel.session.sent.lock().unwrap().len(), 2);
    assert_eq!(
        channel.session.sent.lock().unwrap()[1],
        (OWN_CHAT.to_owned(), "ping".to_owned())
    );

    client
        .send(Command::Connections(ConnectionCommand::DisconnectWhatsApp))
        .unwrap();
    status(&client, |s| matches!(s, WhatsAppStatus::Disconnected)).await;
    assert!(channel.session.logged_out.load(Ordering::SeqCst));
    assert_eq!(channel.forgotten.load(Ordering::SeqCst), 2);
    stop(client).await;
}
