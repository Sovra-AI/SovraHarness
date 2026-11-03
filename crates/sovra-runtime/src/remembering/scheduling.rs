//! Virtual time distinguishes readiness from the lease-recovery polling interval.

use super::*;
use sovra_core::{
    Episode, EpisodeId, FactId, MemoryCandidate, MemoryError, MemoryExtractedChunk, MemoryJob,
    MemoryJobState, MemoryPolicy, NewFact, RecalledFact, Timestamp,
};
use std::collections::VecDeque;
use std::sync::Mutex as SyncMutex;
use tokio::time::Instant;

struct Store {
    pending: SyncMutex<VecDeque<MemoryClaim>>,
    claimed: Sender<Instant>,
    release: Receiver<()>,
    attempts: AtomicU64,
}

#[async_trait::async_trait]
impl MemoryStore for Store {
    async fn health(&self) -> Result<(), MemoryError> {
        Ok(())
    }
    async fn memory_policy(&self) -> Result<MemoryPolicy, MemoryError> {
        Ok(MemoryPolicy {
            enabled: true,
            generation: 0,
        })
    }
    async fn set_memory_enabled(&self, _: bool) -> Result<MemoryPolicy, MemoryError> {
        self.memory_policy().await
    }
    async fn claim_memory(&self, _: Timestamp) -> Result<Option<MemoryClaim>, MemoryError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        let claim = self.pending.lock().unwrap().pop_front();
        if claim.is_some() {
            self.claimed.send(Instant::now()).await.unwrap();
        }
        Ok(claim)
    }
    async fn memory_preceding_sources(&self, _: &EpisodeId) -> Result<Vec<Episode>, MemoryError> {
        // Hold before ordered consolidation so all independent slots can overlap.
        self.release.recv().await.unwrap();
        Ok(Vec::new())
    }
    async fn complete_memory(
        &self,
        _: &MemoryClaim,
        _: Vec<MemoryCandidate>,
        _: &str,
        _: Timestamp,
    ) -> Result<Vec<FactId>, MemoryError> {
        Ok(Vec::new())
    }
    async fn fail_memory(&self, _: &MemoryClaim, _: &str, _: Timestamp) -> Result<(), MemoryError> {
        Ok(())
    }
    async fn memory_job(&self, id: &EpisodeId) -> Result<MemoryJob, MemoryError> {
        Ok(MemoryJob {
            episode_id: id.clone(),
            state: MemoryJobState::Processing,
            attempts: 1,
            error: None,
            fact_ids: vec![],
            candidates: vec![],
            extraction_model: None,
        })
    }
    async fn recall(&self, _: RecallQuery) -> Result<Vec<RecalledFact>, MemoryError> {
        Ok(Vec::new())
    }
    async fn enqueue_memory(&self, input: MemoryInput) -> Result<EpisodeId, MemoryError> {
        let mut claim = claims(1).pop_front().unwrap();
        claim.episode.id = EpisodeId(input.key);
        claim.episode.session_id = input.episode.session_id;
        claim.episode.content = input.episode.content;
        let id = claim.episode.id.clone();
        self.pending.lock().unwrap().push_back(claim);
        Ok(id)
    }
    async fn save_memory_extraction(
        &self,
        _: &MemoryClaim,
        _: &MemoryExtractedChunk,
        _: Timestamp,
    ) -> Result<(), MemoryError> {
        unreachable!()
    }
    async fn record_episode(&self, _: NewEpisode) -> Result<EpisodeId, MemoryError> {
        unreachable!()
    }
    async fn episode(&self, _: &EpisodeId) -> Result<Episode, MemoryError> {
        unreachable!()
    }
    async fn recent_episodes(&self, _: &str, _: usize) -> Result<Vec<Episode>, MemoryError> {
        unreachable!()
    }
    async fn save_fact(&self, _: NewFact) -> Result<FactId, MemoryError> {
        unreachable!()
    }
    async fn invalidate_fact(&self, _: &FactId, _: Timestamp) -> Result<(), MemoryError> {
        unreachable!()
    }
}

fn claims(count: usize) -> VecDeque<MemoryClaim> {
    (0..count)
        .map(|index| MemoryClaim {
            episode: Episode {
                id: EpisodeId(format!("episode:{index}")),
                session_id: format!("chat-{index}"),
                kind: EpisodeKind::UserMessage,
                content: "Fixture wording.".into(),
                occurred_at: Timestamp(1),
            },
            token: format!("lease-{index}"),
            generation: 0,
            attempt: 1,
            lease_until: Timestamp(i64::MAX),
            revision: 0,
            cursor: 0,
            extracted: Some(MemoryExtractedChunk {
                candidates: vec![],
                model: "fixture".into(),
            }),
        })
        .collect()
}

struct Fixture {
    inputs: Sender<CaptureInput>,
    claimed: Receiver<Instant>,
    events: Receiver<Event>,
    store: Arc<Store>,
    release: Sender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    fn start() -> Self {
        Self::with_pending(6)
    }
    fn with_pending(count: usize) -> Self {
        let (inputs, receiver) = async_channel::bounded(64);
        let (claimed, claims_started) = async_channel::unbounded();
        let (release, held) = async_channel::unbounded();
        let (events, observed) = async_channel::unbounded();
        let store = Arc::new(Store {
            pending: SyncMutex::new(claims(count)),
            attempts: AtomicU64::new(0),
            claimed,
            release: held,
        });
        let control = MemoryControl::new();
        // Persisted empty extraction means the configured harness makes no network call.
        let harness = AdkHarness::from_gemini_config(&sovra_core::AgentConfig::with_api_key(Some(
            "fixture-unused".into(),
        )))
        .unwrap();
        control.publish(true, Some(harness));
        let worker = Worker {
            store: store.clone(),
            events,
            config: control.subscribe(),
            control,
            inputs: receiver,
            jobs: JoinSet::new(),
            active: std::collections::HashMap::new(),
            slots: 4,
            permits: Arc::new(Semaphore::new(4)),
            pause_epoch: 0,
        };
        Self {
            inputs,
            claimed: claims_started,
            events: observed,
            store,
            release,
            task: tokio::runtime::Handle::current().spawn(worker.run()),
        }
    }
    async fn claim(&self) -> Instant {
        tokio::time::timeout(Duration::from_secs(2), self.claimed.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn stop(self) {
        self.inputs.close();
        self.task.await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn ready_work_fills_four_slots_without_waiting_for_recovery_ticks() {
    let started = Instant::now();
    let fixture = Fixture::start();
    for _ in 0..4 {
        assert_eq!(
            fixture.claim().await,
            started,
            "free extraction slots must not wait for a polling tick"
        );
    }
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert!(
        fixture.claimed.try_recv().is_err(),
        "the fifth job must wait for a slot"
    );
    fixture.release.send(()).await.unwrap();
    assert_eq!(
        fixture.claim().await,
        started,
        "completion must immediately make a slot available"
    );
    fixture.stop().await;
}

#[tokio::test(start_paused = true)]
async fn background_dispatch_wakes_when_the_interactive_turn_finishes() {
    let interactive = crate::model_priority::interactive();
    let fixture = Fixture::start();
    while !matches!(
        fixture.events.recv().await.unwrap(),
        Event::Memory(MemoryEvent::Status(MemoryStatus::Ready))
    ) {}
    tokio::time::advance(Duration::from_millis(100)).await;
    assert!(fixture.claimed.try_recv().is_err());
    let finished = Instant::now();
    drop(interactive);
    assert_eq!(
        fixture.claim().await,
        finished,
        "interactive completion must wake queued work without a timer delay"
    );
    fixture.stop().await;
}

#[tokio::test(start_paused = true)]
async fn new_evidence_wakes_an_empty_queue_without_busy_polling() {
    let fixture = Fixture::with_pending(0);
    while !matches!(
        fixture.events.recv().await.unwrap(),
        Event::Memory(MemoryEvent::Status(MemoryStatus::Ready))
    ) {}
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(fixture.store.attempts.load(Ordering::SeqCst), 1);
    let captured = Instant::now();
    fixture
        .inputs
        .send(CaptureInput {
            epoch: 0,
            message: None,
            saved: None,
            input: MemoryInput {
                key: "new-source".into(),
                episode: NewEpisode {
                    session_id: "new-chat".into(),
                    kind: EpisodeKind::UserMessage,
                    content: "Fixture wording.".into(),
                    occurred_at: Timestamp(1),
                },
            },
        })
        .await
        .unwrap();
    assert_eq!(fixture.claim().await, captured);
    fixture.release.send(()).await.unwrap();
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    let idle_attempts = fixture.store.attempts.load(Ordering::SeqCst);
    assert!(
        idle_attempts <= 4,
        "an empty queue must not continuously re-claim"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert!(
        fixture.store.attempts.load(Ordering::SeqCst) <= idle_attempts + 2,
        "missed recovery ticks must be skipped, not replayed as a burst"
    );
    fixture.stop().await;
}
