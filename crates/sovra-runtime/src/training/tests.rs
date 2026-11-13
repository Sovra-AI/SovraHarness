use super::*;
use sovra_core::{
    AgentConfig, Command, Event,
    training::{TrainingError, TrainingProgress, TrainingSource},
};

struct Engine;
struct Session(u32);
impl TrainingEngine for Engine {
    fn create(
        &self,
        _: &str,
        _: sovra_core::group::ContentHash,
    ) -> Result<Box<dyn TrainingSession>, TrainingError> {
        Ok(Box::new(Session(0)))
    }
    fn restore(&self, bytes: &[u8]) -> Result<Box<dyn TrainingSession>, TrainingError> {
        let value = serde_json::from_slice(bytes).map_err(|_| TrainingError::InvalidCheckpoint)?;
        Ok(Box::new(Session(value)))
    }
    fn work(
        &self,
        _: sovra_core::training::TrainingTask,
        _: sovra_core::group::DeviceId,
    ) -> Result<sovra_core::training::TrainingResult, TrainingError> {
        Err(TrainingError::InvalidWork)
    }
}
impl TrainingSession for Session {
    fn advance(&mut self, _: u64) -> Result<(), TrainingError> {
        self.0 += 1;
        Ok(())
    }
    fn progress(&self) -> TrainingProgress {
        TrainingProgress {
            rounds: self.0,
            total_rounds: 20,
            tokens: u64::from(self.0) * 32,
            loss_milli: Some(1000),
        }
    }
    fn checkpoint(&self) -> Result<Vec<u8>, TrainingError> {
        serde_json::to_vec(&self.0).map_err(|_| TrainingError::Step)
    }
    fn assign(
        &mut self,
        _: sovra_core::group::DeviceId,
        _: u64,
    ) -> Result<sovra_core::training::TrainingTask, TrainingError> {
        Err(TrainingError::InvalidWork)
    }
    fn merge(
        &mut self,
        _: sovra_core::training::TrainingResult,
        _: sovra_core::group::DeviceId,
        _: u64,
    ) -> Result<(), TrainingError> {
        Err(TrainingError::InvalidWork)
    }
    fn abandon(&mut self, _: sovra_core::group::DeviceId) {}
}

fn client(root: &std::path::Path) -> crate::Client {
    crate::start_with_services(
        "test",
        AgentConfig::default(),
        crate::RuntimeServices {
            training: Some(TrainingConfig {
                engine: Arc::new(Engine),
                directory: root.into(),
            }),
            ..crate::RuntimeServices::default()
        },
    )
    .unwrap()
}
async fn state(
    client: &crate::Client,
    predicate: impl Fn(&TrainingState) -> bool,
) -> TrainingState {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Event::Training(TrainingEvent(state)) = client.events().recv().await.unwrap()
                && predicate(&state)
            {
                return state;
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

#[tokio::test]
async fn client_pause_restart_and_resume_use_acknowledged_durable_progress() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let first = client(root.as_path());
    state(&first, |s| s.phase == TrainingPhase::Ready).await;
    first
        .send(Command::Training(TrainingCommand::Start(
            TrainingSource::Sample,
        )))
        .unwrap();
    state(&first, |s| s.progress.rounds > 0).await;
    first
        .send(Command::Training(TrainingCommand::Pause))
        .unwrap();
    let paused = state(&first, |s| s.phase == TrainingPhase::Paused).await;
    let bytes = std::fs::read(paused.saved_at.as_ref().unwrap()).unwrap();
    assert_eq!(
        serde_json::from_slice::<u32>(&bytes).unwrap(),
        paused.progress.rounds
    );
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
    second
        .send(Command::Training(TrainingCommand::Resume))
        .unwrap();
    state(&second, |s| s.progress.rounds > paused.progress.rounds).await;
    second
        .send(Command::Training(TrainingCommand::Pause))
        .unwrap();
    state(&second, |s| s.phase == TrainingPhase::Paused).await;
    second
        .send(Command::Training(TrainingCommand::Start(
            TrainingSource::Sample,
        )))
        .unwrap();
    let new = state(&second, |s| {
        s.phase == TrainingPhase::Running && s.progress.rounds == 0
    })
    .await;
    assert_ne!(new.saved_at, paused.saved_at);
    assert!(std::path::Path::new(paused.saved_at.as_ref().unwrap()).exists());
    stop(second).await;
}

#[tokio::test]
async fn invalid_input_and_corrupt_checkpoint_are_visible_and_preserved() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let first = client(root.as_path());
    state(&first, |s| s.phase == TrainingPhase::Ready).await;
    first
        .send(Command::Training(TrainingCommand::Start(
            TrainingSource::Text("short".into()),
        )))
        .unwrap();
    let error = state(&first, |s| s.error.is_some()).await;
    assert_eq!(error.phase, TrainingPhase::Ready);
    assert!(!root.as_path().join("latest.json").exists());
    stop(first).await;
    write_private(root.as_path(), "latest.json", &"../../outside")
        .await
        .unwrap();
    let next = client(root.as_path());
    let error = state(&next, |s| s.error.is_some()).await;
    assert!(error.error.unwrap().contains("filename"));
    assert!(root.as_path().join("latest.json").exists());
    stop(next).await;
}

#[test]
fn storage_lease_excludes_another_process_and_filenames_cannot_escape_root() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let first = acquire_lock(root.as_path()).unwrap();
    assert!(acquire_lock(root.as_path()).is_err());
    drop(first);
    assert!(acquire_lock(root.as_path()).is_ok());
    assert!(valid_filename(&format!("run-{}.json", "a".repeat(64))));
    assert!(!valid_filename("../latest.json"));
}

#[tokio::test]
async fn invalid_new_run_keeps_the_completed_run_complete() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("training");
    let filename = format!("run-{}.json", "a".repeat(64));
    write_private(&root, &filename, &20_u32).await.unwrap();
    write_private(&root, "latest.json", &filename)
        .await
        .unwrap();
    let client = client(&root);
    let finished = state(&client, |s| s.phase == TrainingPhase::Complete).await;
    client
        .send(Command::Training(TrainingCommand::Start(
            TrainingSource::Text("short".into()),
        )))
        .unwrap();
    let rejected = state(&client, |s| s.error.is_some()).await;
    assert_eq!(rejected.phase, TrainingPhase::Complete);
    assert_eq!(rejected.progress, finished.progress);
    assert_eq!(rejected.saved_at, finished.saved_at);
    assert_eq!(read_private(&root, &filename).await.unwrap(), b"20");
    stop(client).await;
}
