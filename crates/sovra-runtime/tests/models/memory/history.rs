//! Held storage writes expose ordering and shutdown races through the real Client.

use super::*;

/// Runtime startup shares the machine with the rest of the suite.
const EVENT_WAIT: Duration = Duration::from_secs(15);

async fn next(client: &Client, matches: impl Fn(&Event) -> bool) -> Event {
    tokio::time::timeout(EVENT_WAIT, async {
        loop {
            let event = client.events().recv().await.unwrap();
            if matches(&event) {
                return event;
            }
        }
    })
    .await
    .unwrap()
}

fn save(client: &Client, title: &str) {
    let mut thread = sovra_core::ChatThread::new("restored", Timestamp(2_000_000));
    thread.title = title.into();
    client
        .send(Command::SaveChatThread {
            thread: Box::new(thread),
        })
        .unwrap();
}

async fn fixture() -> (
    Client,
    Arc<Store>,
    async_channel::Sender<()>,
    async_channel::Receiver<String>,
) {
    let (gate, waiting) = async_channel::bounded(1);
    let (started, observed) = async_channel::unbounded();
    let store = Arc::new(Store {
        chat_gate: Some(waiting),
        chat_started: Some(started),
        ..Default::default()
    });
    let memory = store.clone();
    let client = sovra_runtime::start_with_config(
        "history-test",
        AgentConfig::default(),
        Some(Box::new(move || {
            Box::pin(async move { Ok(memory as Arc<dyn MemoryStore>) })
        })),
    )
    .unwrap();
    next(&client, |event| {
        matches!(event, Event::ChatHistoryLoaded { .. })
    })
    .await;
    save(&client, "held");
    assert_eq!(
        tokio::time::timeout(EVENT_WAIT, observed.recv())
            .await
            .unwrap()
            .unwrap(),
        "held"
    );
    (client, store, gate, observed)
}

#[tokio::test]
async fn history_writes_reads_and_deletes_follow_command_order_without_blocking_ping() {
    let (client, store, gate, observed) = fixture().await;
    save(&client, "latest");
    client.send(Command::LoadChatHistory).unwrap();
    client
        .send(Command::DeleteChatThread {
            session_id: "restored".into(),
        })
        .unwrap();
    client.send(Command::LoadChatHistory).unwrap();
    client.send(Command::Ping).unwrap();
    next(&client, |event| matches!(event, Event::Pong)).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), observed.recv())
            .await
            .is_err(),
        "a newer write started while the older snapshot was held"
    );
    gate.send(()).await.unwrap();
    let Event::ChatHistoryLoaded { threads } = next(&client, |event| {
        matches!(event, Event::ChatHistoryLoaded { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].title, "latest");
    let Event::ChatHistoryLoaded { threads } = next(&client, |event| {
        matches!(event, Event::ChatHistoryLoaded { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(threads, Vec::<sovra_core::ChatThread>::new());
    client.send(Command::Shutdown).unwrap();
    next(&client, |event| matches!(event, Event::Stopped)).await;
    let chats = store.chats.lock().unwrap();
    assert_eq!(chats["restored"].title, "latest");
    assert!(chats["restored"].deleted_at.is_some());
}

#[tokio::test]
async fn shutdown_drains_accepted_chat_snapshots_before_reporting_stopped() {
    let (client, store, gate, _) = fixture().await;
    save(&client, "latest");
    client.send(Command::Shutdown).unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            next(&client, |event| matches!(event, Event::Stopped))
        )
        .await
        .is_err(),
        "shutdown abandoned accepted chat writes"
    );
    gate.send(()).await.unwrap();
    next(&client, |event| matches!(event, Event::Stopped)).await;
    assert_eq!(store.chats.lock().unwrap()["restored"].title, "latest");
}
