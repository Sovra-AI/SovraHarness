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
    /// When set, the next `send_text` records the message and then waits.
    hold: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
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
        let count = {
            let mut sent = self.sent.lock().unwrap();
            sent.push((chat_jid.to_owned(), text.to_owned()));
            sent.len()
        };
        let hold = self.hold.lock().unwrap().take();
        if let Some(hold) = hold {
            let _ = hold.await;
        }
        Ok(format!("SENT{count}"))
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

fn messaging_reply(request: &Request, phase: u64) -> Reply {
    let call = |name: &str, arguments: Value| {
        json!({"role":"assistant", "tool_calls":[{
            "index":0,"id":format!("call-{phase}"),"type":"function","function":{"name":name,"arguments":arguments.to_string()}
        }]})
    };
    let (message, finish) = match phase % 5 {
        0 => {
            assert!(
                request.body["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| t["function"]["name"] == "CommunicationsAgent")
            );
            (
                call(
                    "CommunicationsAgent",
                    json!({"request":"Prepare a WhatsApp note to me: Bring the shopping list."}),
                ),
                "tool_calls",
            )
        }
        1 => (call("messaging_status", json!({})), "tool_calls"),
        2 => (
            call(
                "prepare_message",
                json!({"channel":"whats_app","recipient":"me","text":"Bring the shopping list."}),
            ),
            "tool_calls",
        ),
        _ => (
            json!({"role":"assistant","content":"Please review the message and press Send in chat."}),
            "stop",
        ),
    };
    if request.body["stream"] == true {
        Reply {
            status: 200,
            content_type: "text/event-stream",
            tail: None,
            body: format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"messaging-model","choices":[{"index":0,"delta":message,"finish_reason":null}]}),
                json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"messaging-model","choices":[{"index":0,"delta":{},"finish_reason":finish}]})
            ),
        }
    } else {
        Reply::json(
            &json!({"id":"c","object":"chat.completion","created":1,"model":"messaging-model","choices":[{"index":0,"message":message,"finish_reason":finish}]}),
        )
    }
}

#[tokio::test]
async fn main_chat_delegates_to_messaging_but_only_native_review_can_send_once() {
    use sovra_core::communications::MessageDelivery;
    let phase = AtomicU64::new(0);
    let server = Server::new(move |request| {
        if request.path == "/v1/models" {
            return Reply::json(
                &json!({"data":[{"id":"messaging-model","supported_parameters":["tools"],"output_modalities":["text"]}]}),
            );
        }
        messaging_reply(request, phase.fetch_add(1, Ordering::SeqCst))
    });
    let mut connection = ModelConnection::new("messages", ProviderKind::OpenAiCompatible);
    connection.base_url = format!("{}/v1", server.url());
    let fixture = Fixture::new(connection, None);
    let channel = Arc::new(ScriptedChannel::default());
    let client = sovra_runtime::start_with_channels(
        "messaging-test",
        AgentConfig::default(),
        None,
        Some(ModelsConfig {
            repository: fixture.repo.clone(),
            providers: vec![Arc::new(CompatibleProvider)],
        }),
        None,
        None,
        None,
        Some(channel.clone()),
    )
    .unwrap();
    catalog(&client).await;
    let key = ModelKey {
        connection_id: "messages".into(),
        model_id: "messaging-model".into(),
    };
    send(&client, ModelCommand::Activate { key: key.clone() });
    event(&client,|e| matches!(e,Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref()==Some(&key))).await;
    client
        .send(Command::Connections(
            ConnectionCommand::StartWhatsAppPairing,
        ))
        .unwrap();
    status(&client, |s| matches!(s, WhatsAppStatus::Connecting { .. })).await;
    // Load executes after start has installed the channel's event sink.
    client
        .send(Command::Connections(ConnectionCommand::Load))
        .unwrap();
    event(&client, |e| {
        matches!(e, Event::Connections(ConnectionEvent::Loaded(_)))
    })
    .await;
    channel.emit(ChannelEvent::Connected {
        device: LinkedDevice {
            phone_display: "+447700900123".into(),
            device_jid: OWN_CHAT.into(),
            companion_name: "fixture".into(),
            linked_at: sovra_core::Timestamp(0),
        },
    });
    status(&client, WhatsAppStatus::is_connected).await;

    for step in 0..3 {
        let session_id = format!("desktop-{step}");
        client
            .send(Command::RunPrompt {
                context: None,
                session_id: session_id.clone(),
                prompt: "Send me a WhatsApp note: Bring the shopping list.".into(),
            })
            .unwrap();
        let prepared = event(&client, |e| {
            matches!(
                e,
                Event::Connections(ConnectionEvent::OutboundChanged(_)) | Event::AgentError { .. }
            )
        })
        .await;
        let Event::Connections(ConnectionEvent::OutboundChanged(draft)) = prepared else {
            panic!("{prepared:?}")
        };
        assert_eq!(draft.recipient, OWN_CHAT);
        assert_eq!(draft.delivery, MessageDelivery::Review);
        event(
            &client,
            |e| matches!(e,Event::AgentCompleted{session_id:id} if id==&session_id),
        )
        .await;
        assert_eq!(
            channel.session.sent.lock().unwrap().len(),
            usize::from(step > 0),
            "model calls cannot send"
        );
        let approve = |session: &str, send| {
            Command::Connections(ConnectionCommand::ReviewMessage {
                id: draft.id.clone(),
                session_id: session.into(),
                send,
            })
        };
        client.send(approve("another-session", true)).unwrap();
        client
            .send(Command::Connections(ConnectionCommand::Load))
            .unwrap();
        event(&client, |e| {
            matches!(e, Event::Connections(ConnectionEvent::Loaded(_)))
        })
        .await;
        assert_eq!(
            channel.session.sent.lock().unwrap().len(),
            usize::from(step > 0)
        );
        if step == 2 {
            client
                .send(Command::Connections(ConnectionCommand::DisconnectWhatsApp))
                .unwrap();
            status(&client, |s| matches!(s, WhatsAppStatus::Disconnected)).await;
        }
        client.send(approve(&session_id, step != 1)).unwrap();
        client.send(approve(&session_id, true)).unwrap();
        let receipt = event(&client,|e| matches!(e,Event::Connections(ConnectionEvent::OutboundChanged(m)) if m.id==draft.id && !matches!(m.delivery,MessageDelivery::Review|MessageDelivery::Sending))).await;
        let Event::Connections(ConnectionEvent::OutboundChanged(receipt)) = receipt else {
            unreachable!()
        };
        match step {
            0 => assert!(matches!(receipt.delivery, MessageDelivery::Accepted { .. })),
            1 => assert_eq!(receipt.delivery, MessageDelivery::Cancelled),
            _ => assert_eq!(receipt.delivery, MessageDelivery::Expired),
        }
        assert_eq!(
            channel.session.sent.lock().unwrap().as_slice(),
            &[(OWN_CHAT.into(), "Bring the shopping list.".into())]
        );
    }
    stop(client).await;
}

#[tokio::test]
async fn confirmed_timed_whatsapp_sends_on_tick_without_review() {
    use sovra_core::communications::MessageDelivery;
    use sovra_core::connections::ChannelKind;
    use sovra_core::{ScheduleCommand, ScheduleEvent, ScheduledSend};

    let channel = Arc::new(ScriptedChannel::default());
    let client = sovra_runtime::start_with_channels(
        "scheduled-send-test",
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
    status(&client, |s| matches!(s, WhatsAppStatus::Connecting { .. })).await;
    client
        .send(Command::Connections(ConnectionCommand::Load))
        .unwrap();
    event(&client, |e| {
        matches!(e, Event::Connections(ConnectionEvent::Loaded(_)))
    })
    .await;
    channel.emit(ChannelEvent::Connected {
        device: LinkedDevice {
            phone_display: "+447700900123".into(),
            device_jid: OWN_CHAT.into(),
            companion_name: "fixture".into(),
            linked_at: sovra_core::Timestamp(0),
        },
    });
    status(&client, WhatsAppStatus::is_connected).await;

    client
        .send(Command::Schedule(ScheduleCommand::Create {
            name: "Wake Ryan".into(),
            cadence: "in 1 minute".into(),
            prompt: "Send Ryan a WhatsApp: wake up from sovra".into(),
            send: Some(ScheduledSend {
                channel: ChannelKind::WhatsApp,
                recipient: "+44 7347 529758".into(),
                text: "wake up from sovra".into(),
            }),
        }))
        .unwrap();
    let created = event(&client, |e| {
        matches!(
            e,
            Event::Schedule(ScheduleEvent::Snapshot { schedules, .. }) if schedules.len() == 1
        )
    })
    .await;
    let Event::Schedule(ScheduleEvent::Snapshot { schedules, .. }) = created else {
        panic!("{created:?}")
    };
    let send = schedules[0].send.as_ref().expect("stored job is a send");
    assert_eq!(send.channel, ChannelKind::WhatsApp);
    assert_eq!(send.recipient, "+44 7347 529758");
    assert_eq!(send.text, "wake up from sovra");
    let due = schedules[0].next_run_at;

    client
        .send(Command::Schedule(ScheduleCommand::Tick { now: due }))
        .unwrap();
    let outbound = event(&client, |e| {
        matches!(
            e,
            Event::Connections(ConnectionEvent::OutboundChanged(m))
                if m.text == "wake up from sovra"
                    && matches!(m.delivery, MessageDelivery::Accepted { .. })
        )
    })
    .await;
    let Event::Connections(ConnectionEvent::OutboundChanged(message)) = outbound else {
        unreachable!()
    };
    assert_ne!(message.delivery, MessageDelivery::Review);
    assert_eq!(message.recipient, "447347529758@s.whatsapp.net");
    assert_eq!(
        channel.session.sent.lock().unwrap().as_slice(),
        &[(
            "447347529758@s.whatsapp.net".into(),
            "wake up from sovra".into()
        )]
    );
    stop(client).await;
}

const GENTLEMEN_ID: &str = "120363419000000001@g.us";
const GENTLEMEN_TWIN_ID: &str = "120363419000000002@g.us";
const STUB_FINDING: &str = "STUB-FINDING-UAE-AI-FUND";

fn stub_search() -> sovra_adk::SearchClient {
    sovra_adk::SearchClient::from_fn(|_query| async {
        Ok(
            r#"{"results":[{"title":"UAE AI fund","url":"https://example.com/uae-ai","content":"STUB-FINDING-UAE-AI-FUND"}]}"#
                .to_owned(),
        )
    })
}

fn research_client(channel: Arc<ScriptedChannel>) -> Client {
    sovra_runtime::start_with_services(
        "research-schedule-test",
        AgentConfig::default(),
        sovra_runtime::RuntimeServices {
            messaging: sovra_runtime::MessagingChannels {
                whatsapp: Some(channel),
                telegram: None,
            },
            schedule_search: Some(stub_search()),
            ..sovra_runtime::RuntimeServices::default()
        },
    )
    .unwrap()
}

fn gentlemen(id: &str) -> sovra_core::connections::SyncedGroup {
    sovra_core::connections::SyncedGroup {
        id: id.to_owned(),
        subject: "The Gentlemen".into(),
    }
}

fn research_create(recipient: &str) -> sovra_core::ScheduleCommand {
    use sovra_core::connections::ChannelKind;
    sovra_core::ScheduleCommand::CreateResearch {
        name: "UAE AI investments".into(),
        cadence: "every morning at 615 am".into(),
        prompt: "Search the web for investments in AI in the UAE and send a short summary to The Gentlemen.".into(),
        request: sovra_core::ResearchRequest {
            channel: ChannelKind::WhatsApp,
            recipient: recipient.into(),
            query: "investments in AI in the UAE".into(),
        },
    }
}

async fn open_whatsapp(client: &Client, channel: &ScriptedChannel, connect: bool) {
    event(client, |e| matches!(e, Event::Ready { .. })).await;
    client
        .send(Command::Connections(
            ConnectionCommand::StartWhatsAppPairing,
        ))
        .unwrap();
    status(client, |s| matches!(s, WhatsAppStatus::Connecting { .. })).await;
    client
        .send(Command::Connections(ConnectionCommand::Load))
        .unwrap();
    event(client, |e| {
        matches!(e, Event::Connections(ConnectionEvent::Loaded(_)))
    })
    .await;
    if connect {
        channel.emit(ChannelEvent::Connected {
            device: LinkedDevice {
                phone_display: "+447700900123".into(),
                device_jid: OWN_CHAT.into(),
                companion_name: "fixture".into(),
                linked_at: sovra_core::Timestamp(0),
            },
        });
        status(client, WhatsAppStatus::is_connected).await;
    }
}

async fn store_groups(
    client: &Client,
    channel: &ScriptedChannel,
    groups: Vec<sovra_core::connections::SyncedGroup>,
) {
    let ids: Vec<String> = groups.iter().map(|group| group.id.clone()).collect();
    channel.emit(ChannelEvent::Groups(groups));
    event(client, |e| {
        matches!(
            e,
            Event::Connections(ConnectionEvent::GroupsChanged(found))
                if ids.iter().all(|id| found.iter().any(|group| group.id == *id))
        )
    })
    .await;
}

#[tokio::test]
async fn confirmed_search_sends_one_summary_to_the_gentlemen_group() {
    use sovra_core::communications::MessageDelivery;
    use sovra_core::{RunOutcome, ScheduleCadence, ScheduleEvent, ScheduleStatus, next_run_after};

    let channel = Arc::new(ScriptedChannel::default());
    let (release, hold) = tokio::sync::oneshot::channel();
    *channel.session.hold.lock().unwrap() = Some(hold);
    let client = research_client(channel.clone());
    open_whatsapp(&client, &channel, true).await;
    store_groups(&client, &channel, vec![gentlemen(GENTLEMEN_ID)]).await;

    client
        .send(Command::Schedule(research_create("The Gentlemen")))
        .unwrap();
    let created = event(&client, |e| {
        matches!(
            e,
            Event::Schedule(ScheduleEvent::Snapshot { schedules, .. }) if schedules.len() == 1
        )
    })
    .await;
    let Event::Schedule(ScheduleEvent::Snapshot { schedules, .. }) = created else {
        panic!("{created:?}")
    };
    let item = &schedules[0];
    assert_eq!(item.status, ScheduleStatus::Active);
    assert_eq!(
        item.cadence,
        ScheduleCadence::Daily {
            hour: 6,
            minute: 15
        }
    );
    assert!(item.send.is_none());
    let research = item.research.as_ref().expect("saved research job");
    assert_eq!(research.recipient, GENTLEMEN_ID);
    assert_eq!(research.query, "investments in AI in the UAE");
    assert_eq!(channel.session.sent.lock().unwrap().as_slice(), &[]);
    let due = item.next_run_at;
    let due_again = next_run_after(item.cadence, item.tz_offset_minutes, due);

    client
        .send(Command::Schedule(sovra_core::ScheduleCommand::Tick {
            now: due,
        }))
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let sent = channel.session.sent.lock().unwrap().clone();
            if !sent.is_empty() {
                return sent;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the due run sends once");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].0, GENTLEMEN_ID);
    assert!(first[0].1.contains(STUB_FINDING));

    client
        .send(Command::Schedule(sovra_core::ScheduleCommand::Tick {
            now: due_again,
        }))
        .unwrap();
    event(&client, |e| {
        matches!(
            e,
            Event::Schedule(ScheduleEvent::Snapshot { runs, .. })
                if runs.iter().any(|run| run.outcome == Some(RunOutcome::Skipped))
        )
    })
    .await;
    assert_eq!(channel.session.sent.lock().unwrap().len(), 1);

    release.send(()).unwrap();
    let outbound = event(&client, |e| {
        matches!(
            e,
            Event::Connections(ConnectionEvent::OutboundChanged(message))
                if message.recipient == GENTLEMEN_ID
                    && message.text.contains(STUB_FINDING)
                    && matches!(message.delivery, MessageDelivery::Accepted { .. })
        )
    })
    .await;
    let Event::Connections(ConnectionEvent::OutboundChanged(message)) = outbound else {
        unreachable!()
    };
    assert_ne!(message.delivery, MessageDelivery::Review);
    assert_eq!(channel.session.sent.lock().unwrap().len(), 1);
    stop(client).await;

    let ambiguous = Arc::new(ScriptedChannel::default());
    let ambiguous_client = research_client(ambiguous.clone());
    open_whatsapp(&ambiguous_client, &ambiguous, true).await;
    store_groups(
        &ambiguous_client,
        &ambiguous,
        vec![gentlemen(GENTLEMEN_ID), gentlemen(GENTLEMEN_TWIN_ID)],
    )
    .await;
    ambiguous_client
        .send(Command::Schedule(research_create("The Gentlemen")))
        .unwrap();
    event(&ambiguous_client, |e| {
        matches!(
            e,
            Event::Schedule(ScheduleEvent::Error(message)) if message.contains("More than one")
        )
    })
    .await;
    let refused = event(&ambiguous_client, |e| {
        matches!(e, Event::Schedule(ScheduleEvent::Snapshot { schedules, .. }) if schedules.is_empty())
    })
    .await;
    assert!(matches!(
        refused,
        Event::Schedule(ScheduleEvent::Snapshot { schedules, .. }) if schedules.is_empty()
    ));
    assert_eq!(ambiguous.session.sent.lock().unwrap().as_slice(), &[]);
    stop(ambiguous_client).await;

    let offline = Arc::new(ScriptedChannel::default());
    let offline_client = research_client(offline.clone());
    open_whatsapp(&offline_client, &offline, false).await;
    store_groups(&offline_client, &offline, vec![gentlemen(GENTLEMEN_ID)]).await;
    offline_client
        .send(Command::Schedule(research_create("The Gentlemen")))
        .unwrap();
    let saved = event(&offline_client, |e| {
        matches!(
            e,
            Event::Schedule(ScheduleEvent::Snapshot { schedules, .. }) if schedules.len() == 1
        )
    })
    .await;
    let Event::Schedule(ScheduleEvent::Snapshot { schedules, .. }) = saved else {
        panic!("{saved:?}")
    };
    let due = schedules[0].next_run_at;
    assert_eq!(offline.session.sent.lock().unwrap().as_slice(), &[]);
    offline_client
        .send(Command::Schedule(sovra_core::ScheduleCommand::Tick {
            now: due,
        }))
        .unwrap();
    event(&offline_client, |e| {
        matches!(
            e,
            Event::Schedule(ScheduleEvent::Snapshot { runs, .. })
                if runs.iter().any(|run| run.outcome == Some(RunOutcome::Failed))
        )
    })
    .await;
    assert_eq!(offline.session.sent.lock().unwrap().as_slice(), &[]);
    stop(offline_client).await;
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
    assert_eq!(channel.session.sent.lock().unwrap().len(), 1);

    client
        .send(Command::Connections(ConnectionCommand::DisconnectWhatsApp))
        .unwrap();
    status(&client, |s| matches!(s, WhatsAppStatus::Disconnected)).await;
    assert!(channel.session.logged_out.load(Ordering::SeqCst));
    assert_eq!(channel.forgotten.load(Ordering::SeqCst), 2);
    stop(client).await;
}
