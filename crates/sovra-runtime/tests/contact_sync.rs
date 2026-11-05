//! Synced `WhatsApp` contacts stay addressable before anyone has written.
//!
//! A stalled contact pull must not stop another link from reporting its state.
//! Preparing a unique saved person creates a review and does not send.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sovra_core::communications::MessageDelivery;
use sovra_core::connections::{
    AddressBookSnapshot, ChannelError, ChannelEvent, ChannelSession, ChannelSink,
    ChannelStartOptions, ConnectionCommand, ConnectionEvent, LinkedDevice, MessagingChannel,
    SyncedGroup, TelegramStatus, WhatsAppStatus,
};
use sovra_core::people::SyncedContact;
use sovra_core::{AgentConfig, Command, Event, Timestamp};
use sovra_runtime::{MessagingChannels, RuntimeServices};

const OWN_CHAT: &str = "447700900123@s.whatsapp.net";
const TOKEN: &str = "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw5";
const NOTE: &str = "The school gate opens at 3.";

struct ScriptedWhatsApp {
    sink: Mutex<Option<ChannelSink>>,
    session: Arc<ScriptedSession>,
    entered: AtomicBool,
    finished: AtomicBool,
    release: async_channel::Receiver<()>,
}

struct ScriptedSession {
    sent: Mutex<Vec<(String, String)>>,
}

struct ScriptedTelegram {
    sink: Mutex<Option<ChannelSink>>,
}

struct QuietSession;

#[async_trait::async_trait]
impl MessagingChannel for ScriptedWhatsApp {
    async fn start(
        &self,
        _options: ChannelStartOptions,
        events: ChannelSink,
    ) -> Result<Arc<dyn ChannelSession>, ChannelError> {
        *self.sink.lock().expect("sink") = Some(events);
        Ok(self.session.clone())
    }

    fn has_saved_link(&self) -> bool {
        false
    }

    fn forget(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn pull_contacts(&self) -> Result<AddressBookSnapshot, ChannelError> {
        self.entered.store(true, Ordering::SeqCst);
        let _ = self.release.recv().await;
        self.finished.store(true, Ordering::SeqCst);
        Ok(AddressBookSnapshot {
            contacts: vec![SyncedContact {
                name: "Sam Rivera".into(),
                phone: "+447700900111".into(),
                removed: false,
            }],
            groups: vec![SyncedGroup {
                id: "120363111@g.us".into(),
                subject: "School Gate".into(),
            }],
        })
    }
}

#[async_trait::async_trait]
impl ChannelSession for ScriptedSession {
    async fn send_text(&self, chat_jid: &str, text: &str) -> Result<String, ChannelError> {
        self.sent
            .lock()
            .expect("sent")
            .push((chat_jid.to_owned(), text.to_owned()));
        Ok("sent".into())
    }

    async fn logout(&self) {}

    async fn stop(&self) {}

    fn own_chat_jid(&self) -> Option<String> {
        Some(OWN_CHAT.to_owned())
    }
}

#[async_trait::async_trait]
impl MessagingChannel for ScriptedTelegram {
    async fn start(
        &self,
        _options: ChannelStartOptions,
        events: ChannelSink,
    ) -> Result<Arc<dyn ChannelSession>, ChannelError> {
        *self.sink.lock().expect("sink") = Some(events);
        Ok(Arc::new(QuietSession))
    }

    fn has_saved_link(&self) -> bool {
        false
    }

    fn forget(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl ChannelSession for QuietSession {
    async fn send_text(&self, _chat_jid: &str, _text: &str) -> Result<String, ChannelError> {
        Ok("sent".into())
    }

    async fn logout(&self) {}

    async fn stop(&self) {}

    fn own_chat_jid(&self) -> Option<String> {
        None
    }
}

impl ScriptedWhatsApp {
    fn emit(&self, event: ChannelEvent) {
        let sink = self
            .sink
            .lock()
            .expect("sink")
            .clone()
            .expect("link started");
        sink(event);
    }
}

impl ScriptedTelegram {
    fn emit(&self, event: ChannelEvent) {
        let sink = self
            .sink
            .lock()
            .expect("sink")
            .clone()
            .expect("bot started");
        sink(event);
    }
}

struct Seen {
    kind_probed: bool,
}

async fn wait(
    client: &sovra_runtime::Client,
    seen: &mut Seen,
    pred: impl Fn(&Event) -> bool,
) -> Event {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let event = client.events().recv().await.expect("event");
            if matches!(event, Event::Connections(ConnectionEvent::KindProbed(_))) {
                seen.kind_probed = true;
            }
            if pred(&event) {
                return event;
            }
        }
    })
    .await
    .expect("expected event")
}

#[tokio::test]
async fn synced_contact_can_be_prepared_before_they_write_and_a_stalled_pull_does_not_block_telegram()
 {
    let (release_tx, release) = async_channel::bounded(1);
    let whatsapp = Arc::new(ScriptedWhatsApp {
        sink: Mutex::new(None),
        session: Arc::new(ScriptedSession {
            sent: Mutex::new(Vec::new()),
        }),
        entered: AtomicBool::new(false),
        finished: AtomicBool::new(false),
        release,
    });
    let telegram = Arc::new(ScriptedTelegram {
        sink: Mutex::new(None),
    });
    let client = sovra_runtime::start_with_services(
        "contact-sync",
        AgentConfig::default(),
        RuntimeServices {
            messaging: MessagingChannels {
                whatsapp: Some(whatsapp.clone()),
                telegram: Some(telegram.clone()),
            },
            ..RuntimeServices::default()
        },
    )
    .expect("runtime");
    let mut seen = Seen { kind_probed: false };
    wait(&client, &mut seen, |event| {
        matches!(event, Event::Ready { .. })
    })
    .await;

    client
        .send(Command::Connections(
            ConnectionCommand::StartWhatsAppPairing,
        ))
        .expect("pair");
    wait(&client, &mut seen, |event| {
        matches!(
            event,
            Event::Connections(ConnectionEvent::StatusChanged(
                WhatsAppStatus::Connecting { .. }
            ))
        )
    })
    .await;
    client
        .send(Command::Connections(ConnectionCommand::Load))
        .expect("load");
    wait(&client, &mut seen, |event| {
        matches!(event, Event::Connections(ConnectionEvent::Loaded(_)))
    })
    .await;

    whatsapp.emit(ChannelEvent::Connected {
        device: LinkedDevice {
            phone_display: "+447700900123".into(),
            device_jid: OWN_CHAT.into(),
            companion_name: "fixture".into(),
            linked_at: Timestamp(0),
        },
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !whatsapp.entered.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("contact pull started");
    assert!(
        !whatsapp.finished.load(Ordering::SeqCst),
        "the pull is still waiting"
    );

    client
        .send(Command::Connections(ConnectionCommand::ConnectTelegram {
            token: TOKEN.into(),
        }))
        .expect("telegram");
    wait(&client, &mut seen, |event| {
        matches!(
            event,
            Event::Connections(ConnectionEvent::TelegramStatusChanged(
                TelegramStatus::Connecting
            ))
        )
    })
    .await;
    client
        .send(Command::Connections(ConnectionCommand::Load))
        .expect("load");
    wait(&client, &mut seen, |event| {
        matches!(event, Event::Connections(ConnectionEvent::Loaded(_)))
    })
    .await;
    telegram.emit(ChannelEvent::Connected {
        device: LinkedDevice {
            phone_display: "@sovra_test_bot".into(),
            device_jid: "123456789".into(),
            companion_name: "Sovra".into(),
            linked_at: Timestamp(0),
        },
    });
    wait(&client, &mut seen, |event| {
        matches!(
            event,
            Event::Connections(ConnectionEvent::TelegramStatusChanged(
                TelegramStatus::AwaitingOwner { .. }
            ))
        )
    })
    .await;
    assert!(
        !whatsapp.finished.load(Ordering::SeqCst),
        "telegram reported while the contact pull was still waiting"
    );

    release_tx.send(()).await.expect("release");
    let people = wait(&client, &mut seen, |event| {
        matches!(
            event,
            Event::Connections(ConnectionEvent::PeopleChanged(book))
                if book.people.iter().any(|person| person.title() == "Sam Rivera")
        )
    })
    .await;
    let Event::Connections(ConnectionEvent::PeopleChanged(book)) = people else {
        unreachable!()
    };
    assert!(
        book.people
            .iter()
            .any(|person| person.title() == "Sam Rivera")
    );
    assert!(whatsapp.finished.load(Ordering::SeqCst));
    wait(&client, &mut seen, |event| {
        matches!(
            event,
            Event::Connections(ConnectionEvent::GroupsChanged(groups))
                if groups.iter().any(|group| group.subject == "School Gate")
        )
    })
    .await;

    client
        .send(Command::Connections(ConnectionCommand::Load))
        .expect("load");
    let loaded = wait(&client, &mut seen, |event| {
        matches!(event, Event::Connections(ConnectionEvent::Loaded(_)))
    })
    .await;
    let Event::Connections(ConnectionEvent::Loaded(state)) = loaded else {
        unreachable!()
    };
    assert_eq!(state.recent_messages.len(), 0);
    assert!(
        state
            .people
            .people
            .iter()
            .any(|person| person.title() == "Sam Rivera")
    );
    assert!(
        state
            .groups
            .iter()
            .any(|group| group.id == "120363111@g.us" && group.subject == "School Gate")
    );

    client
        .send(Command::Connections(ConnectionCommand::PrepareMessage {
            session_id: "desktop".into(),
            channel: sovra_core::connections::ChannelKind::WhatsApp,
            recipient: "Sam Rivera".into(),
            text: NOTE.into(),
        }))
        .expect("prepare");
    let prepared = wait(&client, &mut seen, |event| {
        matches!(
            event,
            Event::Connections(ConnectionEvent::OutboundChanged(_))
        )
    })
    .await;
    let Event::Connections(ConnectionEvent::OutboundChanged(draft)) = prepared else {
        unreachable!()
    };
    assert_eq!(draft.recipient, "447700900111@s.whatsapp.net");
    assert_eq!(draft.recipient_label, "Sam Rivera");
    assert_eq!(draft.delivery, MessageDelivery::Review);
    assert_eq!(draft.text, NOTE);
    assert_eq!(whatsapp.session.sent.lock().expect("sent").len(), 0);

    client
        .send(Command::Connections(ConnectionCommand::PrepareMessage {
            session_id: "desktop".into(),
            channel: sovra_core::connections::ChannelKind::WhatsApp,
            recipient: "School Gate".into(),
            text: NOTE.into(),
        }))
        .expect("prepare group");
    let group_prepared = wait(&client, &mut seen, |event| {
        matches!(
            event,
            Event::Connections(ConnectionEvent::OutboundChanged(draft))
                if draft.recipient == "120363111@g.us"
        )
    })
    .await;
    let Event::Connections(ConnectionEvent::OutboundChanged(group_draft)) = group_prepared else {
        unreachable!()
    };
    assert_eq!(group_draft.recipient, "120363111@g.us");
    assert_eq!(group_draft.recipient_label, "School Gate");
    assert_eq!(group_draft.delivery, MessageDelivery::Review);
    assert_eq!(whatsapp.session.sent.lock().expect("sent").len(), 0);

    whatsapp.emit(ChannelEvent::Contacts(vec![SyncedContact {
        name: "Sam Okonkwo".into(),
        phone: "+447700900222".into(),
        removed: false,
    }]));
    wait(&client, &mut seen, |event| {
        matches!(
            event,
            Event::Connections(ConnectionEvent::PeopleChanged(book))
                if book.people.iter().any(|person| person.title() == "Sam Okonkwo")
        )
    })
    .await;
    client
        .send(Command::Connections(ConnectionCommand::PrepareMessage {
            session_id: "desktop".into(),
            channel: sovra_core::connections::ChannelKind::WhatsApp,
            recipient: "Sam".into(),
            text: NOTE.into(),
        }))
        .expect("prepare");
    let notice = wait(&client, &mut seen, |event| {
        matches!(event, Event::Connections(ConnectionEvent::Notice(_)))
    })
    .await;
    let Event::Connections(ConnectionEvent::Notice(message)) = notice else {
        unreachable!()
    };
    assert!(
        message.contains("More than one"),
        "ambiguous name must ask, got {message}"
    );

    client
        .send(Command::Connections(ConnectionCommand::Load))
        .expect("load");
    let loaded = wait(&client, &mut seen, |event| {
        matches!(event, Event::Connections(ConnectionEvent::Loaded(_)))
    })
    .await;
    let Event::Connections(ConnectionEvent::Loaded(state)) = loaded else {
        unreachable!()
    };
    assert_eq!(state.outbound_messages.len(), 2);
    assert_eq!(
        state.outbound_messages[0].recipient,
        "447700900111@s.whatsapp.net"
    );
    assert_eq!(state.outbound_messages[0].delivery, MessageDelivery::Review);
    assert_eq!(state.outbound_messages[1].recipient, "120363111@g.us");
    assert_eq!(state.outbound_messages[1].delivery, MessageDelivery::Review);
    assert!(matches!(
        state.whatsapp,
        WhatsAppStatus::Connected { online: true, .. }
    ));
    assert!(matches!(
        state.telegram,
        TelegramStatus::AwaitingOwner { .. }
    ));
    assert_eq!(state.recent_messages.len(), 0);
    assert_eq!(whatsapp.session.sent.lock().expect("sent").len(), 0);
    assert!(!seen.kind_probed, "reading contacts must not probe a link");

    client.shutdown_blocking().expect("shutdown");
}
