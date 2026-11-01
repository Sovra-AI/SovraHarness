//! The Telegram bot link through the real `Client`, with a scripted channel adapter in
//! place of Telegram's servers: a pasted token connects the bot, the first person to send
//! the code becomes its owner, the owner's messages are answered on the phone through the
//! ordinary prompt path, strangers are ignored, and disconnecting forgets the token.

use super::*;
use sovra_core::connections::{
    ANSWER_FAILED_REPLY, ChannelError, ChannelEvent, ChannelSession, ChannelSink,
    ChannelStartOptions, ChatKind, ConnectionCommand, ConnectionEvent, IncomingMessage,
    LinkedDevice, MessagingChannel, PollOutcome, TELEGRAM_ASK_FOR_CODE_REPLY,
    TELEGRAM_LINKED_REPLY, TelegramStatus,
};

const TOKEN: &str = "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw5";
const OWNER_CHAT: &str = "777";

#[derive(Default)]
struct ScriptedBot {
    sink: Mutex<Option<ChannelSink>>,
    session: Arc<ScriptedSession>,
    tokens_seen: Mutex<Vec<Option<String>>>,
    forgotten: AtomicU64,
}

#[derive(Default)]
struct ScriptedSession {
    sent: Mutex<Vec<(String, String)>>,
    polls: AtomicU64,
    stopped: AtomicBool,
}

#[async_trait::async_trait]
impl MessagingChannel for ScriptedBot {
    async fn start(
        &self,
        options: ChannelStartOptions,
        events: ChannelSink,
    ) -> Result<Arc<dyn ChannelSession>, ChannelError> {
        self.tokens_seen.lock().unwrap().push(options.token.clone());
        if options.token.as_deref() != Some(TOKEN) {
            return Err(ChannelError::Failed(
                "Telegram does not accept this token.".into(),
            ));
        }
        events(ChannelEvent::Connected {
            device: LinkedDevice {
                phone_display: "@sovra_test_bot".into(),
                device_jid: "123456789".into(),
                companion_name: "Sovra".into(),
                linked_at: sovra_core::Timestamp(0),
            },
        });
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
        Ok(format!("{chat_jid}:sent{}", sent.len()))
    }

    async fn logout(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    async fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    fn own_chat_jid(&self) -> Option<String> {
        None
    }

    /// The runtime must keep polling bots; a scripted bot has nothing queued, so it asks
    /// for a pause each time instead of spinning.
    async fn poll(&self) -> PollOutcome {
        self.polls.fetch_add(1, Ordering::SeqCst);
        if self.stopped.load(Ordering::SeqCst) {
            PollOutcome::Finished
        } else {
            PollOutcome::Wait(Duration::from_millis(20))
        }
    }
}

impl ScriptedBot {
    fn message(&self, chat: &str, name: &str, text: &str) {
        let sink = self.sink.lock().unwrap().clone().expect("bot started");
        sink(ChannelEvent::Message(IncomingMessage {
            id: format!("{chat}:{}", text.len()),
            chat_jid: chat.into(),
            sender_jid: chat.into(),
            sender_name: name.into(),
            text: text.into(),
            from_me: false,
            chat: ChatKind::Direct,
            backlog: false,
        }));
    }
}

async fn telegram_status(
    client: &Client,
    matches: impl Fn(&TelegramStatus) -> bool,
) -> TelegramStatus {
    match event(client, |e| {
        matches!(e, Event::Connections(ConnectionEvent::TelegramStatusChanged(s)) if matches(s))
    })
    .await
    {
        Event::Connections(ConnectionEvent::TelegramStatusChanged(status)) => status,
        _ => unreachable!(),
    }
}

async fn wait_for_sent(bot: &ScriptedBot, count: usize) -> Vec<(String, String)> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let sent = bot.session.sent.lock().unwrap().clone();
            if sent.len() >= count {
                return sent;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("bot sent the expected messages")
}

#[tokio::test]
async fn telegram_bot_connects_links_its_owner_and_answers() {
    let bot = Arc::new(ScriptedBot::default());
    let client = sovra_runtime::start_with_messaging(
        "tg-test",
        AgentConfig::default(),
        None,
        None,
        None,
        None,
        None,
        sovra_runtime::MessagingChannels {
            whatsapp: None,
            telegram: Some(bot.clone()),
        },
    )
    .unwrap();
    event(&client, |e| matches!(e, Event::Ready { .. })).await;

    // A token that is not even shaped like one is refused before Telegram is asked.
    client
        .send(Command::Connections(ConnectionCommand::ConnectTelegram {
            token: "nonsense".into(),
        }))
        .unwrap();
    event(
        &client,
        |e| matches!(e, Event::Connections(ConnectionEvent::Notice(n)) if n.contains("BotFather")),
    )
    .await;
    assert!(bot.tokens_seen.lock().unwrap().is_empty());

    client
        .send(Command::Connections(ConnectionCommand::ConnectTelegram {
            token: format!("  bot{TOKEN} "),
        }))
        .unwrap();
    telegram_status(&client, |s| matches!(s, TelegramStatus::Connecting)).await;
    let waiting = telegram_status(&client, |s| {
        matches!(s, TelegramStatus::AwaitingOwner { .. })
    })
    .await;
    let TelegramStatus::AwaitingOwner { bot_username, code } = waiting else {
        unreachable!()
    };
    assert_eq!(bot_username, "sovra_test_bot");
    assert_eq!(code.len(), 6);
    assert_eq!(
        bot.tokens_seen.lock().unwrap().as_slice(),
        [Some(TOKEN.to_owned())],
        "the pasted token is cleaned and handed to the adapter once"
    );

    // Someone writes before the code: they are told how to link. The owner sends the code.
    bot.message("999", "Alex", "hello?");
    let sent = wait_for_sent(&bot, 1).await;
    assert_eq!(
        sent[0],
        ("999".to_owned(), TELEGRAM_ASK_FOR_CODE_REPLY.to_owned())
    );
    bot.message(OWNER_CHAT, "Sam", &code);
    let linked = telegram_status(&client, TelegramStatus::is_connected).await;
    let TelegramStatus::Connected {
        owner_chat_id,
        owner_name,
        ..
    } = linked
    else {
        unreachable!()
    };
    assert_eq!(owner_chat_id, OWNER_CHAT);
    assert_eq!(owner_name, "Sam");
    let sent = wait_for_sent(&bot, 2).await;
    assert_eq!(
        sent[1],
        (OWNER_CHAT.to_owned(), TELEGRAM_LINKED_REPLY.to_owned())
    );

    // The owner's message is recorded and answered on the phone; no model is configured in
    // this runtime, so the fallback goes back, and the chat window never sees the session.
    bot.message(OWNER_CHAT, "Sam", "What's on today?");
    let mut leaked = Vec::new();
    let reply = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let e = client.events().recv().await.unwrap();
            if e.session_id().is_some_and(|id| id.starts_with("telegram:")) {
                leaked.push(format!("{e:?}"));
            }
            if let Event::Connections(ConnectionEvent::ReplySent { message_id, reply }) = e
                && message_id.starts_with(OWNER_CHAT)
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
        "chat window saw Telegram events: {leaked:?}"
    );
    let sent = wait_for_sent(&bot, 3).await;
    assert_eq!(
        sent[2],
        (OWNER_CHAT.to_owned(), ANSWER_FAILED_REPLY.to_owned())
    );

    // A stranger is ignored with the safe default. Load is an actor barrier.
    bot.message("999", "Alex", "Are you a bot?");
    client
        .send(Command::Connections(ConnectionCommand::Load))
        .unwrap();
    event(&client, |e| {
        matches!(e, Event::Connections(ConnectionEvent::Loaded(_)))
    })
    .await;
    assert_eq!(
        bot.session.sent.lock().unwrap().len(),
        3,
        "the stranger got no answer"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while bot.session.polls.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the runtime keeps polling a bot session");

    client
        .send(Command::Connections(ConnectionCommand::DisconnectTelegram))
        .unwrap();
    telegram_status(&client, |s| matches!(s, TelegramStatus::Disconnected)).await;
    assert!(bot.session.stopped.load(Ordering::SeqCst));
    assert!(bot.forgotten.load(Ordering::SeqCst) >= 2);
    stop(client).await;
}
