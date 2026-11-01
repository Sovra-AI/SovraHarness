//! Held reads prove bounded concurrent recall and cancellation through the real client.

use super::*;

const WORDS: [&str; 8] = [
    "jasmine",
    "tea",
    "breakfast",
    "preference",
    "garden",
    "weekend",
    "lunch",
    "coffee",
];

pub(super) struct ReadProbe {
    started: async_channel::Sender<String>,
    finished: async_channel::Sender<String>,
    gates: BTreeMap<String, async_channel::Receiver<()>>,
    active: AtomicU64,
    peak: AtomicU64,
}

pub(super) struct PendingRead<'a> {
    probe: &'a ReadProbe,
    term: String,
}

impl Drop for PendingRead<'_> {
    fn drop(&mut self) {
        self.probe.active.fetch_sub(1, Ordering::SeqCst);
        let _ = self.probe.finished.try_send(self.term.clone());
    }
}

impl ReadProbe {
    pub(super) async fn enter(&self, term: &str) -> PendingRead<'_> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        let pending = PendingRead {
            probe: self,
            term: term.into(),
        };
        self.started.send(term.into()).await.unwrap();
        self.gates[term].recv().await.unwrap();
        pending
    }
}

struct HeldReads {
    probe: Arc<ReadProbe>,
    started: async_channel::Receiver<String>,
    finished: async_channel::Receiver<String>,
    releases: BTreeMap<String, async_channel::Sender<()>>,
    enqueue_release: async_channel::Sender<()>,
    store: Arc<Store>,
}

impl HeldReads {
    fn new() -> Self {
        let (started, starts) = async_channel::bounded(16);
        let (finished, finishes) = async_channel::bounded(16);
        let mut gates = BTreeMap::new();
        let mut releases = BTreeMap::new();
        for word in WORDS {
            let (release, gate) = async_channel::bounded(1);
            gates.insert(word.into(), gate);
            releases.insert(word.into(), release);
        }
        let probe = Arc::new(ReadProbe {
            started,
            finished,
            gates,
            active: AtomicU64::new(0),
            peak: AtomicU64::new(0),
        });
        // Hold capture admission so only the foreground recall owns these reads.
        let (enqueue_release, enqueue_gate) = async_channel::bounded(1);
        let store = Arc::new(Store {
            keyword_probe: Some(probe.clone()),
            enqueue_gate: Some(enqueue_gate),
            ..Default::default()
        });
        saved_tea(&store);
        Self {
            probe,
            started: starts,
            finished: finishes,
            releases,
            enqueue_release,
            store,
        }
    }

    async fn first_batch(&self) {
        for expected in &WORDS[..4] {
            let term = tokio::time::timeout(Duration::from_secs(1), self.started.recv())
                .await
                .expect("four reads must begin without releasing any earlier read")
                .unwrap();
            assert_eq!(&term, expected);
        }
        assert_eq!(self.probe.active.load(Ordering::SeqCst), 4);
        assert!(self.started.try_recv().is_err());
    }

    fn release(&self, word: &str) {
        self.releases[word].try_send(()).unwrap();
    }

    fn prompt(client: &Client) {
        client
            .send(Command::RunPrompt {
                context: Some(sovra_core::ConversationContext::default()),
                session_id: "parallel-recall".into(),
                prompt: WORDS.join(" "),
            })
            .unwrap();
    }

    async fn stop(self, client: Client) {
        // Disable before releasing capture; it must not start another extraction.
        send(
            &client,
            ModelCommand::SetAgentSettings {
                settings: AgentSettings {
                    memory_enabled: false,
                    ..Default::default()
                },
            },
        );
        self.enqueue_release.send(()).await.unwrap();
        stop(client).await;
    }
}

#[tokio::test]
async fn recall_checks_four_keywords_at_once_and_delivers_saved_evidence() {
    let server = recall_server();
    let held = HeldReads::new();
    let (_fixture, client) = start(&server, held.store.clone(), true).await;
    HeldReads::prompt(&client);
    held.first_batch().await;
    client.send(Command::Ping).unwrap();
    event(&client, |event| matches!(event, Event::Pong)).await;
    // Complete the batch out of order; scheduling must remain bounded.
    for word in WORDS[..4].iter().rev() {
        held.release(word);
    }
    for expected in &WORDS[4..] {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), held.started.recv())
                .await
                .unwrap()
                .unwrap(),
            *expected
        );
    }
    assert_eq!(held.probe.active.load(Ordering::SeqCst), 4);
    assert_eq!(held.probe.peak.load(Ordering::SeqCst), 4);
    for word in &WORDS[4..] {
        held.release(word);
    }
    event(&client, |event| {
        matches!(event, Event::AgentCompleted { session_id } if session_id == "parallel-recall")
    })
    .await;
    assert_eq!(held.probe.active.load(Ordering::SeqCst), 0);
    assert_eq!(recalled_requests(&server), 1);
    let terms = held
        .store
        .recalls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|query| query.text.clone())
        .collect::<Vec<_>>();
    assert_eq!(terms, WORDS);
    held.stop(client).await;
}

#[tokio::test]
async fn pausing_drops_all_four_pending_keyword_reads_without_starting_the_rest() {
    let server = recall_server();
    let held = HeldReads::new();
    let (_fixture, client) = start(&server, held.store.clone(), true).await;
    HeldReads::prompt(&client);
    held.first_batch().await;
    send(
        &client,
        ModelCommand::SetAgentSettings {
            settings: AgentSettings {
                memory_enabled: false,
                ..Default::default()
            },
        },
    );
    for _ in 0..4 {
        tokio::time::timeout(Duration::from_secs(1), held.finished.recv())
            .await
            .expect("pause must drop every pending read without releasing a gate")
            .unwrap();
    }
    assert_eq!(held.probe.active.load(Ordering::SeqCst), 0);
    assert_eq!(held.probe.peak.load(Ordering::SeqCst), 4);
    assert!(held.started.try_recv().is_err());
    event(&client, |event| {
        matches!(event, Event::AgentCompleted { session_id } if session_id == "parallel-recall")
    })
    .await;
    assert_eq!(recalled_requests(&server), 0);
    held.stop(client).await;
}
