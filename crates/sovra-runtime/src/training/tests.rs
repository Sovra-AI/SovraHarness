use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::*;
use sovra_core::{
    AgentConfig, Command,
    group::ContentHash,
    training::{
        Architecture, BaseModel, MetricPoint, ModelFacts, ModelHeader, ModelOrigin, SAMPLE_TEXT,
        SourceKind, TrainingBase, TrainingEngineKind, TrainingError, TrainingPlan,
        TrainingPlanSummary, TrainingPreset, TrainingProgress, TrainingResult, TrainingSettings,
        TrainingSource, TrainingTask, encode_model, inspect_model, model_id,
    },
};

use crate::private_storage::{read_private_bounded, write_private};

const DEADLINE: Duration = Duration::from_secs(20);

/// A stand-in learner: each round takes `round` and counts one. Its checkpoint
/// is small JSON so a test can read what reached the disk.
struct Engine {
    round: Duration,
    /// The round whose first attempt fails, to test a broken step. Zero: none.
    fail_at: u32,
    failed: Arc<AtomicU32>,
    created: Arc<AtomicU64>,
}

impl Engine {
    fn new(round_ms: u64) -> Self {
        Self {
            round: Duration::from_millis(round_ms),
            fail_at: 0,
            failed: Arc::new(AtomicU32::new(0)),
            created: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    rounds: u32,
    plan: TrainingPlanSummary,
    base_rounds: u32,
}

struct Session {
    saved: Saved,
    round: Duration,
    fail_at: u32,
    failed: Arc<AtomicU32>,
}

fn wait(duration: Duration) {
    let started = std::time::Instant::now();
    while started.elapsed() < duration {
        std::thread::park_timeout(duration.saturating_sub(started.elapsed()));
    }
}

fn hardware() -> HardwareInfo {
    HardwareInfo {
        os: "Test OS".into(),
        arch: "test64".into(),
        cpu: "Test processor".into(),
        physical_cores: 4,
        logical_cores: 8,
        memory_total_bytes: 16 << 30,
        memory_available_bytes: 8 << 30,
        engine: TrainingEngineKind::Cpu,
        engine_name: "Built-in processor engine".into(),
        gpus: Vec::new(),
    }
}

impl TrainingEngine for Engine {
    fn hardware(&self) -> HardwareInfo {
        hardware()
    }

    fn create(
        &self,
        plan: &TrainingPlan,
        text: &str,
        base: Option<&[u8]>,
        _: ContentHash,
    ) -> Result<Box<dyn TrainingSession>, TrainingError> {
        let (architecture, base_rounds) = match (plan.base, base) {
            (TrainingBase::Fresh(model), None) => (model.architecture(), 0),
            (TrainingBase::Continue(id), Some(file)) => {
                let facts = inspect_model(file)?;
                if facts.id != id {
                    return Err(TrainingError::InvalidModel);
                }
                (facts.header.architecture, facts.header.rounds)
            }
            (TrainingBase::Fresh(_), Some(_)) | (TrainingBase::Continue(_), None) => {
                return Err(TrainingError::InvalidModel);
            }
        };
        self.created.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Session {
            saved: Saved {
                rounds: 0,
                plan: plan.summary(architecture, text.len()),
                base_rounds,
            },
            round: self.round,
            fail_at: self.fail_at,
            failed: self.failed.clone(),
        }))
    }

    fn restore(&self, bytes: &[u8]) -> Result<Box<dyn TrainingSession>, TrainingError> {
        let saved = serde_json::from_slice(bytes).map_err(|_| TrainingError::InvalidCheckpoint)?;
        Ok(Box::new(Session {
            saved,
            round: self.round,
            fail_at: self.fail_at,
            failed: self.failed.clone(),
        }))
    }

    fn work(&self, _: TrainingTask, _: DeviceId) -> Result<TrainingResult, TrainingError> {
        Err(TrainingError::InvalidWork)
    }

    fn inspect(&self, model: &[u8]) -> Result<ModelFacts, TrainingError> {
        inspect_model(model)
    }

    fn generate(
        &self,
        model: &[u8],
        prompt: &str,
        bytes: u16,
        _: u32,
        seed: u64,
    ) -> Result<String, TrainingError> {
        inspect_model(model)?;
        Ok(format!("{prompt}… {bytes} more, seed {seed}"))
    }
}

impl TrainingSession for Session {
    fn advance(&mut self, _: u64) -> Result<(), TrainingError> {
        wait(self.round);
        if self.fail_at == self.saved.rounds + 1 && self.failed.fetch_add(1, Ordering::SeqCst) == 0
        {
            return Err(TrainingError::Step);
        }
        self.saved.rounds += 1;
        Ok(())
    }

    fn progress(&self) -> TrainingProgress {
        TrainingProgress {
            rounds: self.saved.rounds,
            total_rounds: self.saved.plan.settings.rounds,
            tokens: u64::from(self.saved.rounds) * 32,
            loss_milli: Some(5000_u32.saturating_sub(self.saved.rounds)),
        }
    }

    fn checkpoint(&self) -> Result<Vec<u8>, TrainingError> {
        serde_json::to_vec(&self.saved).map_err(|_| TrainingError::Step)
    }

    fn assign(&mut self, _: DeviceId, _: u64) -> Result<TrainingTask, TrainingError> {
        Err(TrainingError::InvalidWork)
    }

    fn merge(&mut self, _: TrainingResult, _: DeviceId, _: u64) -> Result<(), TrainingError> {
        Err(TrainingError::InvalidWork)
    }

    fn abandon(&mut self, _: DeviceId) {}

    fn plan(&self) -> TrainingPlanSummary {
        self.saved.plan.clone()
    }

    fn metrics(&self) -> Option<MetricPoint> {
        let round = self.saved.rounds;
        (round > 0).then(|| MetricPoint {
            round,
            loss_milli: 5000_u32.saturating_sub(round),
            check_loss_milli: round
                .is_multiple_of(5)
                .then(|| 5100_u32.saturating_sub(round)),
            lr_micro: 100,
            tokens_per_second: 2000,
        })
    }

    fn sample(&self) -> Option<(u32, String)> {
        let round = self.saved.rounds / 5 * 5;
        (round > 0).then(|| (round, format!("written at round {round}")))
    }

    fn export(&self) -> Result<Vec<u8>, TrainingError> {
        let architecture = self.saved.plan.architecture;
        #[expect(clippy::cast_precision_loss, reason = "a test value")]
        let fill = self.saved.rounds as f32 / 1000.0;
        encode_model(
            &ModelHeader {
                base: if architecture.is_table() {
                    BaseModel::Starter
                } else {
                    BaseModel::Small
                },
                architecture,
                rounds: self.saved.base_rounds + self.saved.rounds,
                tokens: u64::from(self.saved.rounds) * 32,
                loss_milli: self.progress().loss_milli,
                check_loss_milli: None,
            },
            &vec![fill; architecture.parameters()],
        )
    }
}

fn client_with(root: &std::path::Path, engine: Engine) -> crate::Client {
    crate::start_with_services(
        "test",
        AgentConfig::default(),
        crate::RuntimeServices {
            training: Some(TrainingConfig {
                engine: Arc::new(engine),
                directory: root.into(),
            }),
            ..crate::RuntimeServices::default()
        },
    )
    .unwrap()
}

fn client(root: &std::path::Path) -> crate::Client {
    client_with(root, Engine::new(2))
}

/// A Starter plan on the sample with `rounds` rounds.
fn plan(rounds: u32) -> TrainingPlan {
    let mut settings = TrainingSettings::recommended(
        TrainingPreset::Quick,
        Architecture::STARTER,
        SAMPLE_TEXT.len(),
        &hardware(),
    );
    settings.rounds = rounds;
    settings.stop_when_stalled = 0;
    TrainingPlan {
        name: "Garden model".into(),
        base: TrainingBase::Fresh(BaseModel::Starter),
        source: TrainingSource::Sample,
        preset: None,
        settings,
        with_group: false,
    }
}

fn send(client: &crate::Client, command: TrainingCommand) {
    client.send(Command::Training(command)).unwrap();
}

fn start(client: &crate::Client, plan: TrainingPlan) {
    send(client, TrainingCommand::Start(Box::new(plan)));
}

async fn state(
    client: &crate::Client,
    predicate: impl Fn(&TrainingState) -> bool,
) -> TrainingState {
    tokio::time::timeout(DEADLINE, async {
        loop {
            if let Event::Training(TrainingEvent(state)) = client.events().recv().await.unwrap()
                && predicate(&state)
            {
                return *state;
            }
        }
    })
    .await
    .expect("training state deadline")
}

async fn stop(client: crate::Client) {
    tokio::task::spawn_blocking(move || client.shutdown_blocking().unwrap())
        .await
        .unwrap();
}

fn saved_rounds(state: &TrainingState) -> u32 {
    let bytes = std::fs::read(state.saved_at.as_ref().unwrap()).unwrap();
    serde_json::from_slice::<Saved>(&bytes).unwrap().rounds
}

#[tokio::test]
async fn pause_restart_and_resume_use_saved_progress_and_keep_the_chart() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let first = client(root.as_path());
    let ready = state(&first, |s| s.phase == TrainingPhase::Ready).await;
    assert!(ready.plan.is_none() && ready.models.is_empty());
    start(&first, plan(50_000));
    let running = state(&first, |s| s.progress.rounds > 10).await;
    assert_eq!(running.phase, TrainingPhase::Running);
    assert_eq!(running.plan.as_ref().unwrap().name, "Garden model");
    assert_eq!(
        running.plan.as_ref().unwrap().source.kind,
        SourceKind::Sample
    );
    send(&first, TrainingCommand::Pause);
    let paused = state(&first, |s| s.phase == TrainingPhase::Paused).await;
    // Paused means saved: the file holds exactly the progress that is shown.
    assert_eq!(saved_rounds(&paused), paused.progress.rounds);
    assert_ne!(paused.metrics.points.len(), 0);
    assert_eq!(
        paused.metrics.latest().unwrap().round,
        paused.progress.rounds
    );
    assert!(paused.metrics.sample.is_some());
    assert!(paused.metrics.elapsed_ms > 0);
    first.send(Command::Ping).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while first.events().recv().await.unwrap() != Event::Pong {}
    })
    .await
    .unwrap();
    stop(first).await;

    let second = client(root.as_path());
    let restored = state(&second, |s| s.phase == TrainingPhase::Paused).await;
    assert_eq!(restored.progress, paused.progress);
    assert_eq!(restored.plan, paused.plan);
    assert_eq!(restored.metrics.points, paused.metrics.points);
    assert_eq!(restored.metrics.sample, paused.metrics.sample);
    assert_eq!(restored.sharing, None);
    send(&second, TrainingCommand::Resume);
    state(&second, |s| s.progress.rounds > paused.progress.rounds).await;
    send(&second, TrainingCommand::Pause);
    let again = state(&second, |s| s.phase == TrainingPhase::Paused).await;
    assert_eq!(again.models.len(), 0);

    // Starting again keeps the earlier run's file and puts what it had
    // learned into the library.
    start(&second, plan(50_000));
    let new = state(&second, |s| {
        s.phase == TrainingPhase::Running && s.saved_at != paused.saved_at
    })
    .await;
    assert!(std::path::Path::new(paused.saved_at.as_ref().unwrap()).exists());
    assert_eq!(new.models.len(), 1);
    assert_eq!(new.models[0].card.rounds, again.progress.rounds);
    assert!(new.progress.rounds < again.progress.rounds);
    stop(second).await;
}

#[tokio::test]
async fn quitting_mid_run_saves_the_rounds_done_so_far() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let first = client(root.as_path());
    state(&first, |s| s.phase == TrainingPhase::Ready).await;
    start(&first, plan(50_000));
    let running = state(&first, |s| s.progress.rounds > 20).await;
    stop(first).await;
    let second = client(root.as_path());
    let restored = state(&second, |s| s.phase == TrainingPhase::Paused).await;
    assert!(restored.progress.rounds >= running.progress.rounds);
    assert_eq!(saved_rounds(&restored), restored.progress.rounds);
    stop(second).await;
}

#[tokio::test]
async fn invalid_input_and_corrupt_checkpoint_are_visible_and_preserved() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let first = client(root.as_path());
    state(&first, |s| s.phase == TrainingPhase::Ready).await;
    let mut short = plan(100);
    short.source = TrainingSource::Text("short".into());
    start(&first, short);
    let error = state(&first, |s| s.error.is_some()).await;
    assert_eq!(error.phase, TrainingPhase::Ready);
    assert!(!root.as_path().join("latest.json").exists());
    let mut wild = plan(100);
    wild.settings.rounds = 3;
    start(&first, wild);
    let error = state(&first, |s| {
        s.error.as_ref().is_some_and(|e| e.contains("Rounds"))
    })
    .await;
    assert_eq!(error.phase, TrainingPhase::Ready);
    stop(first).await;

    write_private(root.as_path(), "latest.json", &"../../outside")
        .await
        .unwrap();
    let next = client(root.as_path());
    let error = state(&next, |s| s.error.is_some()).await;
    assert!(error.error.as_ref().unwrap().contains("filename"));
    assert_eq!(error.phase, TrainingPhase::Ready);
    assert!(root.as_path().join("latest.json").exists());
    // Opening the page does not hide the problem, and a new run still starts.
    send(&next, TrainingCommand::Load);
    let loaded = state(&next, |s| s.hardware.is_some()).await;
    assert_eq!(loaded.error, error.error);
    start(&next, plan(50_000));
    let running = state(&next, |s| s.phase == TrainingPhase::Running).await;
    assert_eq!(running.error, None);
    stop(next).await;
}

#[test]
fn storage_lease_excludes_another_process_and_filenames_cannot_escape_root() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let first = storage::acquire_lock(root.as_path()).unwrap();
    assert!(storage::acquire_lock(root.as_path()).is_err());
    drop(first);
    assert!(storage::acquire_lock(root.as_path()).is_ok());
    let id = "a".repeat(64);
    assert!(storage::valid_filename(&storage::run_filename(&id)));
    // Runs saved by the first training release keep their name.
    assert!(storage::valid_filename(&format!("run-{id}.json")));
    assert!(!storage::valid_filename("../latest.json"));
    assert!(!storage::valid_filename(&format!(
        "run-{}.train",
        "A".repeat(64)
    )));
    assert!(!storage::valid_filename(&format!("run-{id}/../x.train")));
    assert_eq!(
        storage::notes_filename(&storage::run_filename(&id)).unwrap(),
        format!("run-{id}.notes.json")
    );
    assert!(storage::notes_filename("latest.json").is_none());
}

async fn finished_run(root: &std::path::Path, rounds: u32) -> String {
    let filename = storage::run_filename(&"a".repeat(64));
    let saved = Saved {
        rounds,
        plan: plan(rounds).summary(Architecture::STARTER, SAMPLE_TEXT.len()),
        base_rounds: 0,
    };
    write_private(root, &filename, &saved).await.unwrap();
    write_private(root, "latest.json", &filename).await.unwrap();
    filename
}

#[tokio::test]
async fn invalid_new_run_keeps_the_completed_run_complete() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let filename = finished_run(&root, 20).await;
    let before = read_private_bounded(&root, &filename, 4096).await.unwrap();
    let client = client(&root);
    let finished = state(&client, |s| s.phase == TrainingPhase::Complete).await;
    let mut short = plan(100);
    short.source = TrainingSource::Text("short".into());
    start(&client, short);
    let rejected = state(&client, |s| s.error.is_some()).await;
    assert_eq!(rejected.phase, TrainingPhase::Complete);
    assert_eq!(rejected.progress, finished.progress);
    assert_eq!(rejected.saved_at, finished.saved_at);
    assert_eq!(
        read_private_bounded(&root, &filename, 4096).await.unwrap(),
        before
    );
    // A finished run whose model never reached the library is added when the
    // page is opened.
    assert_eq!(rejected.models.len(), 0);
    send(&client, TrainingCommand::Load);
    let listed = state(&client, |s| !s.models.is_empty()).await;
    assert_eq!(listed.models[0].card.rounds, 20);
    assert_eq!(listed.phase, TrainingPhase::Complete);
    stop(client).await;
}

#[tokio::test]
async fn a_finished_run_lands_in_the_library_and_can_be_tried_and_continued() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let first = client(root.as_path());
    let ready = state(&first, |s| s.hardware.is_some()).await;
    assert_eq!(ready.hardware.as_ref().unwrap().cpu, "Test processor");
    start(&first, plan(60));
    let done = state(&first, |s| s.phase == TrainingPhase::Complete).await;
    assert_eq!(done.progress.rounds, 60);
    assert_eq!(done.error, None);
    assert_eq!(done.metrics.latest().unwrap().round, 60);
    assert_eq!(done.metrics.best_check_round, 60);
    assert_eq!(done.metrics.sample.as_deref(), Some("written at round 60"));
    assert_eq!(saved_rounds(&done), 60);
    let model = done.models.first().expect("the model is in the library");
    assert_eq!(model.card.name, "Garden model");
    assert_eq!(model.card.rounds, 60);
    assert_eq!(model.card.origin, ModelOrigin::TrainedHere);
    assert_eq!(model.card.group, None);
    assert_eq!(model.card.trained_by, "This computer");
    let file = std::fs::read(&model.saved_at).unwrap();
    assert_eq!(model_id(&file), model.card.id);
    assert_eq!(
        std::path::Path::new(&model.saved_at).extension(),
        Some(std::ffi::OsStr::new("model"))
    );
    assert!(model.saved_at.contains("models"));
    let id = model.card.id;
    stop(first).await;

    // After a restart the library and the finished run are both still there.
    let second = client(root.as_path());
    let restored = state(&second, |s| s.phase == TrainingPhase::Complete).await;
    assert_eq!(restored.models, done.models);
    send(
        &second,
        TrainingCommand::Try {
            model: id,
            prompt: "The garden".into(),
        },
    );
    let trying = state(&second, |s| s.trial.is_some()).await;
    assert_eq!(trying.trial.as_ref().unwrap().prompt, "The garden");
    let tried = state(&second, |s| {
        s.trial.as_ref().is_some_and(|trial| !trial.running)
    })
    .await;
    let output = tried
        .trial
        .unwrap()
        .output
        .expect("the model wrote something");
    assert!(output.starts_with("The garden…"), "{output}");
    send(
        &second,
        TrainingCommand::Try {
            model: ContentHash::of(b"no such model"),
            prompt: String::new(),
        },
    );
    let missing = state(&second, |s| s.error.is_some()).await;
    assert!(missing.error.unwrap().contains("no longer in Your models"));

    // Continue from the library model: the run keeps that model's shape and
    // the result is a second model; the first is untouched.
    let mut more = plan(40);
    more.name = "Garden model, more".into();
    more.base = TrainingBase::Continue(id);
    start(&second, more);
    let continued = state(&second, |s| {
        s.phase == TrainingPhase::Complete && s.models.len() == 2
    })
    .await;
    assert_eq!(
        continued.plan.as_ref().unwrap().base,
        TrainingBase::Continue(id)
    );
    assert_eq!(continued.models[0].card.name, "Garden model, more");
    assert_eq!(continued.models[0].card.rounds, 100);
    assert_eq!(continued.models[1], done.models[0]);
    assert_eq!(std::fs::read(&done.models[0].saved_at).unwrap(), file);

    // Sharing needs a group; nothing changes without one.
    send(&second, TrainingCommand::Share(id));
    let refused = state(&second, |s| s.error.is_some()).await;
    assert!(refused.error.unwrap().contains("group"));
    assert_eq!(refused.models, continued.models);
    stop(second).await;
}

#[tokio::test]
async fn stop_finishes_now_and_keeps_the_model_as_it_is() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let first = client(root.as_path());
    state(&first, |s| s.phase == TrainingPhase::Ready).await;
    start(&first, plan(50_000));
    state(&first, |s| s.progress.rounds > 10).await;
    send(&first, TrainingCommand::Stop);
    let stopped = state(&first, |s| s.phase == TrainingPhase::Complete).await;
    assert!(stopped.progress.rounds > 10 && stopped.progress.rounds < 50_000);
    assert_eq!(stopped.error, None);
    assert_eq!(saved_rounds(&stopped), stopped.progress.rounds);
    assert_eq!(stopped.models.len(), 1);
    assert_eq!(stopped.models[0].card.rounds, stopped.progress.rounds);
    // Stopping again, pausing or resuming a finished run does nothing.
    for command in [
        TrainingCommand::Stop,
        TrainingCommand::Pause,
        TrainingCommand::Resume,
    ] {
        send(&first, command);
        let same = state(&first, |_| true).await;
        assert_eq!(same.phase, TrainingPhase::Complete);
        assert_eq!(same.progress, stopped.progress);
        assert_eq!(same.models, stopped.models);
    }
    stop(first).await;
    // A stopped run stays finished after a restart; it is not offered to resume.
    let second = client(root.as_path());
    let restored = state(&second, |s| s.phase != TrainingPhase::Loading).await;
    assert_eq!(restored.phase, TrainingPhase::Complete);
    assert_eq!(restored.progress, stopped.progress);
    assert_eq!(restored.models, stopped.models);
    stop(second).await;
}

#[tokio::test]
async fn pause_and_stop_wait_for_the_round_in_progress_and_no_longer() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    // Rounds far longer than a slice: every round is its own slice.
    let client = client_with(root.as_path(), Engine::new(400));
    state(&client, |s| s.phase == TrainingPhase::Ready).await;
    start(&client, plan(50_000));
    let running = state(&client, |s| s.phase == TrainingPhase::Running).await;
    send(&client, TrainingCommand::Pause);
    let pausing = state(&client, |s| s.phase != TrainingPhase::Running).await;
    assert_eq!(pausing.phase, TrainingPhase::Pausing);
    let paused = state(&client, |s| s.phase == TrainingPhase::Paused).await;
    assert!(paused.progress.rounds <= running.progress.rounds + 2);
    assert_eq!(saved_rounds(&paused), paused.progress.rounds);

    send(&client, TrainingCommand::Resume);
    state(&client, |s| s.progress.rounds > paused.progress.rounds).await;
    send(&client, TrainingCommand::Stop);
    let stopped = state(&client, |s| s.phase == TrainingPhase::Complete).await;
    assert!(stopped.progress.rounds <= paused.progress.rounds + 3);
    assert_eq!(stopped.models.len(), 1);
    stop(client).await;
}

#[tokio::test]
async fn a_text_file_is_read_bounded_and_refused_in_plain_words() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let notes = fixture.path().join("notes.txt");
    let text = "\u{feff}".to_owned() + &"Tomatoes like sun and steady water. ".repeat(40);
    std::fs::write(&notes, &text).unwrap();
    let binary = fixture.path().join("photo.md");
    std::fs::write(&binary, [0xff_u8; 200]).unwrap();
    let huge = fixture.path().join("huge.txt");
    std::fs::write(&huge, vec![b'a'; sovra_core::training::MAX_TEXT_BYTES + 10]).unwrap();
    let tiny = fixture.path().join("tiny.txt");
    std::fs::write(&tiny, "too small").unwrap();

    let client = client(root.as_path());
    state(&client, |s| s.hardware.is_some()).await;
    for (path, expected) in [
        (
            fixture.path().join("missing.txt"),
            "could not open missing.txt",
        ),
        (binary, "photo.md is not plain text"),
        (huge, "huge.txt is larger than 4 MB"),
        (tiny, "tiny.txt needs between 32 bytes"),
        (fixture.path().to_owned(), ".txt or .md"),
        (fixture.path().join("program.exe"), ".txt or .md"),
    ] {
        let mut from_file = plan(50_000);
        from_file.source = TrainingSource::File(path);
        start(&client, from_file);
        let refused = state(&client, |s| s.error.is_some()).await;
        assert!(
            refused.error.as_ref().unwrap().contains(expected),
            "{:?}",
            refused.error
        );
        assert_eq!(refused.phase, TrainingPhase::Ready);
        assert!(refused.plan.is_none());
    }
    assert!(!root.join("latest.json").exists());

    let mut from_file = plan(50_000);
    from_file.source = TrainingSource::File(notes);
    start(&client, from_file);
    let running = state(&client, |s| s.phase == TrainingPhase::Running).await;
    let source = &running.plan.as_ref().unwrap().source;
    assert_eq!(source.kind, SourceKind::File);
    assert_eq!(source.file_name.as_deref(), Some("notes.txt"));
    // The byte-order mark is not part of the text.
    assert_eq!(source.bytes, text.len() as u64 - 3);
    stop(client).await;
}

#[tokio::test]
async fn progress_is_published_a_few_times_a_second_however_fast_rounds_are() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let client = client_with(root.as_path(), Engine::new(1));
    state(&client, |s| s.phase == TrainingPhase::Ready).await;
    start(&client, plan(100_000));
    let first = state(&client, |s| s.progress.rounds > 0).await;
    let started = std::time::Instant::now();
    let mut events = 0_u64;
    let mut last = first.clone();
    while last.progress.rounds < first.progress.rounds + 300 {
        assert!(started.elapsed() < DEADLINE, "training is continuous");
        if let Ok(Ok(Event::Training(TrainingEvent(state)))) =
            tokio::time::timeout(Duration::from_millis(100), client.events().recv()).await
        {
            events += 1;
            last = *state;
        }
    }
    // Hundreds of rounds, a handful of events: one per quarter second at most.
    let allowed = u64::try_from(started.elapsed().as_millis()).unwrap() / PUBLISH_EVERY_MS + 2;
    assert!(
        events <= allowed,
        "{events} events in {:?} for {} rounds",
        started.elapsed(),
        last.progress.rounds - first.progress.rounds
    );
    // The chart keeps a bounded history however many rounds there are.
    assert!(last.metrics.points.len() <= sovra_core::training::MAX_METRIC_POINTS);
    assert!(last.metrics.eta_ms.is_some());
    stop(client).await;
}

#[tokio::test]
async fn a_failed_round_keeps_the_saved_run_and_resume_carries_on() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let mut engine = Engine::new(2);
    engine.fail_at = 30;
    let client = client_with(root.as_path(), engine);
    state(&client, |s| s.phase == TrainingPhase::Ready).await;
    start(&client, plan(50_000));
    let failed = state(&client, |s| s.error.is_some()).await;
    assert_eq!(failed.phase, TrainingPhase::Paused);
    assert!(failed.error.as_ref().unwrap().contains("last saved step"));
    // What is on disk is still a run that opens.
    assert!(saved_rounds(&failed) < 30);
    send(&client, TrainingCommand::Resume);
    let resumed = state(&client, |s| s.progress.rounds > 40).await;
    assert_eq!(resumed.phase, TrainingPhase::Running);
    assert_eq!(resumed.error, None);
    stop(client).await;
}

#[tokio::test]
async fn training_waits_while_a_chat_turn_is_active() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let client = client(root.as_path());
    state(&client, |s| s.phase == TrainingPhase::Ready).await;
    start(&client, plan(100_000));
    state(&client, |s| s.progress.rounds > 5).await;
    let chat = crate::model_priority::interactive();
    // The slice that was out hands the run back after its round.
    tokio::time::sleep(Duration::from_millis(600)).await;
    while client.events().try_recv().is_ok() {}
    send(&client, TrainingCommand::Load);
    let held = state(&client, |_| true).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    send(&client, TrainingCommand::Load);
    let still = state(&client, |_| true).await;
    assert_eq!(still.phase, TrainingPhase::Running);
    assert_eq!(still.progress.rounds, held.progress.rounds);
    drop(chat);
    state(&client, |s| s.progress.rounds > held.progress.rounds).await;
    stop(client).await;
}

#[tokio::test]
async fn a_second_start_while_training_changes_nothing_and_says_why() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let engine = Engine::new(2);
    let created = engine.created.clone();
    let client = client_with(root.as_path(), engine);
    state(&client, |s| s.phase == TrainingPhase::Ready).await;
    start(&client, plan(50_000));
    let running = state(&client, |s| s.phase == TrainingPhase::Running).await;
    start(&client, plan(50_000));
    let told = state(&client, |s| s.notice.is_some()).await;
    assert!(told.notice.unwrap().contains("already running"));
    assert_eq!(told.saved_at, running.saved_at);
    assert_eq!(created.load(Ordering::SeqCst), 1);
    // A group run needs a group; the running run is not disturbed.
    send(&client, TrainingCommand::Pause);
    state(&client, |s| s.phase == TrainingPhase::Paused).await;
    let mut shared = plan(50_000);
    shared.with_group = true;
    start(&client, shared);
    let refused = state(&client, |s| s.error.is_some()).await;
    assert!(refused.error.unwrap().contains("join a group"));
    assert_eq!(refused.phase, TrainingPhase::Paused);
    assert_eq!(refused.saved_at, running.saved_at);
    assert_eq!(created.load(Ordering::SeqCst), 1);
    stop(client).await;
}

#[test]
fn the_library_lists_only_whole_models_and_never_trusts_a_card_alone() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    assert_eq!(storage::scan_library(&root).unwrap().len(), 0);
    let directory = storage::models_directory(&root);
    crate::private_storage::private_directory(&directory).unwrap();
    let write = |name: &str, bytes: &[u8]| {
        let path = directory.join(name);
        std::fs::write(&path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    };
    let model = |rounds: u32, created: u64| {
        let file = encode_model(
            &ModelHeader {
                base: BaseModel::Starter,
                architecture: Architecture::STARTER,
                rounds,
                tokens: 0,
                loss_milli: None,
                check_loss_milli: None,
            },
            &vec![0.0; Architecture::STARTER.parameters()],
        )
        .unwrap();
        let card = sovra_core::training::ModelCard::from_facts(
            &inspect_model(&file).unwrap(),
            format!("Model {rounds}"),
            "This computer".into(),
            ModelOrigin::TrainedHere,
            None,
            created,
        );
        (file, card)
    };
    let entry = |card: &sovra_core::training::ModelCard| {
        serde_json::to_vec(&sovra_core::training::LibraryModel {
            card: card.clone(),
            saved_at: "/somewhere/else".into(),
            delivery: Vec::new(),
        })
        .unwrap()
    };
    let (old_file, old) = model(1, 100);
    let (new_file, new) = model(2, 200);
    let (_, lonely) = model(3, 300);
    let (short_file, short) = model(4, 400);
    write(&storage::model_filename(old.id), &old_file);
    write(&storage::card_filename(old.id), &entry(&old));
    write(&storage::model_filename(new.id), &new_file);
    write(&storage::card_filename(new.id), &entry(&new));
    // A card with no model, a model cut short, a card under another model's
    // name, and files that are not ours.
    write(&storage::card_filename(lonely.id), &entry(&lonely));
    write(&storage::model_filename(short.id), &short_file[..100]);
    write(&storage::card_filename(short.id), &entry(&short));
    write(
        &storage::card_filename(ContentHash::of(b"other")),
        &entry(&old),
    );
    write("notes.json", b"{}");
    write(&format!("{}.json", "z".repeat(64)), b"{}");

    let listed = storage::scan_library(&root).unwrap();
    assert_eq!(
        listed.iter().map(|m| m.card.id).collect::<Vec<_>>(),
        vec![new.id, old.id],
        "newest first"
    );
    assert_eq!(
        listed[0].saved_at,
        directory
            .join(storage::model_filename(new.id))
            .display()
            .to_string()
    );
    assert_eq!(storage::read_model(&root, old.id).unwrap(), old_file);
    // A file that is not the model its name says is refused.
    write(&storage::model_filename(old.id), &new_file);
    assert!(storage::read_model(&root, old.id).is_err());
    assert!(storage::read_model(&root, lonely.id).is_err());
}
