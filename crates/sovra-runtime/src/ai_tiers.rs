//! Picking each message's AI by difficulty, and Laya's download and lifetime.
//!
//! The decision itself is pure (`sovra_core::ai_tiers::decide_tier`). This module does
//! the IO around it: download and verify Laya's weights, load them once off the async
//! threads, ask Laya with a deadline, and keep one ready harness per tier.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest as _, Sha256};
use sovra_core::ai_tiers::{
    DifficultyJudge, LAYA_FILES, LAYA_REPOSITORY, LAYA_REVISION, LayaJudgement, LayaLoader,
    LayaStatus, Sorter, ThinkingLevel, TierDecision, TierRouting, decide_tier, laya_download_bytes,
};
use sovra_core::providers::ModelTier;
use tokio::io::AsyncWriteExt as _;

use crate::models::ModelExecution;

/// Where Laya's verified files live and how to load them. From the composition root.
#[derive(Clone)]
pub struct LayaSetup {
    pub dir: PathBuf,
    pub loader: Arc<dyn LayaLoader>,
}

impl std::fmt::Debug for LayaSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayaSetup")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

/// A message waits at most this long for Laya; the quick sorter answers otherwise.
/// Laya takes about a quarter of a second on a laptop processor.
const CHAT_DEADLINE: Duration = Duration::from_millis(1_500);
/// The sample-question tester on My AIs may wait for a first load as well.
pub(crate) const TEST_DEADLINE: Duration = Duration::from_secs(20);
/// Download progress is reported in steps of this many bytes.
const PROGRESS_STEP: u64 = 4_000_000;
/// Written after every file passed its checksum, naming the verified revision.
const MARKER: &str = "verified";

/// Laya's files and, once loaded, the model itself. Shared by the actor and chat turns.
pub(crate) struct Laya {
    setup: Option<LayaSetup>,
    judge: Mutex<Option<Arc<dyn DifficultyJudge>>>,
}

impl Laya {
    pub(crate) fn new(setup: Option<LayaSetup>) -> Self {
        Self {
            setup,
            judge: Mutex::new(None),
        }
    }

    pub(crate) fn dir(&self) -> Option<&Path> {
        self.setup.as_ref().map(|setup| setup.dir.as_path())
    }

    fn loaded(&self) -> Option<Arc<dyn DifficultyJudge>> {
        self.judge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn forget(&self) {
        *self
            .judge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    pub(crate) fn is_loaded(&self) -> bool {
        self.loaded().is_some()
    }

    /// Every file present at its expected size, and the folder verified for this revision.
    pub(crate) fn installed(&self) -> bool {
        self.dir().is_some_and(installed_at)
    }

    pub(crate) fn status(&self) -> LayaStatus {
        if self.setup.is_none() {
            LayaStatus::Failed("Laya is not available in this build.".into())
        } else if self.is_loaded() {
            LayaStatus::Ready
        } else if self.installed() {
            LayaStatus::Installed
        } else {
            LayaStatus::NotInstalled
        }
    }

    /// Loads the model (about two seconds). Blocking: call through `spawn_blocking`.
    pub(crate) fn load_blocking(&self) -> Result<(), String> {
        if self.is_loaded() {
            return Ok(());
        }
        let setup = self
            .setup
            .as_ref()
            .ok_or_else(|| "Laya is not available in this build.".to_owned())?;
        if !installed_at(&setup.dir) {
            return Err("Laya is not downloaded yet.".into());
        }
        let judge = setup.loader.load(&setup.dir)?;
        *self
            .judge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(judge);
        Ok(())
    }

    /// Laya's judgement within `deadline`, or `None` when it is not loaded, fails or
    /// is too slow. A late answer finishes on the blocking pool and is dropped; each
    /// call is a single bounded forward pass, so nothing piles up.
    pub(crate) async fn judge(&self, prompt: &str, deadline: Duration) -> Option<LayaJudgement> {
        let judge = self.loaded()?;
        let prompt = prompt.to_owned();
        let work = tokio::task::spawn_blocking(move || judge.judge(&prompt));
        match tokio::time::timeout(deadline, work).await {
            Ok(Ok(Ok(judgement))) => Some(judgement),
            Ok(Ok(Err(error))) => {
                tracing::warn!(%error, "Laya could not rate this message");
                None
            }
            Ok(Err(_)) => None,
            Err(_) => {
                tracing::info!(?deadline, "Laya was too slow; using the quick sorter");
                None
            }
        }
    }
}

fn installed_at(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(MARKER)).is_ok_and(|revision| revision.trim() == LAYA_REVISION)
        && LAYA_FILES.iter().all(|file| {
            std::fs::metadata(dir.join(file.local)).is_ok_and(|meta| meta.len() == file.bytes)
        })
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

fn staging(dir: &Path) -> PathBuf {
    dir.with_extension("download")
}

/// Downloads every file into a staging folder, checking size and SHA-256 as bytes
/// arrive, then swaps the folder into place. Nothing half-written is ever loaded.
pub(crate) async fn install(
    dir: PathBuf,
    progress: async_channel::Sender<(u64, u64)>,
) -> Result<(), String> {
    let total = laya_download_bytes();
    let staging = staging(&dir);
    let _ = tokio::fs::remove_dir_all(&staging).await;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "Could not start the download.".to_owned())?;
    let mut received = 0;
    let mut reported = 0;
    for file in LAYA_FILES {
        let url = format!(
            "https://huggingface.co/{LAYA_REPOSITORY}/resolve/{LAYA_REVISION}/{}",
            file.remote
        );
        let target = staging.join(file.local);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|_| "Could not make Laya's folder.".to_owned())?;
        }
        let mut response = client
            .get(&url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| "Could not reach Hugging Face to download Laya. Check your internet connection and try again.".to_owned())?;
        let mut out = tokio::fs::File::create(&target)
            .await
            .map_err(|_| "Could not save Laya's files. Check free disk space.".to_owned())?;
        let mut hasher = Sha256::new();
        let mut written: u64 = 0;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "The Laya download was interrupted. Try again.".to_owned())?
        {
            written += chunk.len() as u64;
            if written > file.bytes {
                return Err("Laya's download was larger than expected, so it was stopped.".into());
            }
            hasher.update(&chunk);
            out.write_all(&chunk)
                .await
                .map_err(|_| "Could not save Laya's files. Check free disk space.".to_owned())?;
            received += chunk.len() as u64;
            // Network chunks are a few KB; report every few MB so the UI is not flooded.
            if received - reported >= PROGRESS_STEP || received == total {
                reported = received;
                let _ = progress.try_send((received, total));
            }
        }
        out.flush()
            .await
            .map_err(|_| "Could not save Laya's files. Check free disk space.".to_owned())?;
        if written != file.bytes || hex(&hasher.finalize()) != file.sha256 {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(
                "Laya's download did not match its checksum, so it was not used. Try again.".into(),
            );
        }
    }
    tokio::fs::write(staging.join(MARKER), LAYA_REVISION)
        .await
        .map_err(|_| "Could not save Laya's files. Check free disk space.".to_owned())?;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::rename(&staging, &dir)
        .await
        .map_err(|_| "Could not finish installing Laya.".to_owned())?;
    Ok(())
}

/// Deletes Laya's files and any partial download.
pub(crate) async fn uninstall(dir: &Path) -> Result<(), String> {
    let _ = tokio::fs::remove_dir_all(staging(dir)).await;
    match tokio::fs::remove_dir_all(dir).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Could not delete Laya's files.".into()),
    }
}

/// One tier's ready harness and how its replies are labelled.
#[derive(Debug, Clone)]
pub(crate) struct TierExecution {
    pub(crate) execution: ModelExecution,
    pub(crate) thinking: ThinkingLevel,
}

/// The saved routing plus a harness for each tier that has a usable model.
#[derive(Debug, Clone, Default)]
pub(crate) struct TierState {
    pub(crate) routing: TierRouting,
    pub(crate) tiers: BTreeMap<ModelTier, TierExecution>,
}

fn tier_name(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Fast => "Simple question",
        ModelTier::Standard => "Medium question",
        ModelTier::Deep => "Complex question",
    }
}

/// The label beside a routed reply's model name.
pub(crate) fn route_note(decision: &TierDecision, thinking: ThinkingLevel) -> String {
    format!(
        "{} · {} ({})",
        tier_name(decision.tier),
        thinking.label(),
        decision.reason
    )
}

/// Sorts `prompt` with the saved sorter.
pub(crate) async fn decide(
    routing: &TierRouting,
    laya: &Laya,
    prompt: &str,
    deadline: Duration,
) -> TierDecision {
    let judgement = match routing.sorter {
        Sorter::Laya => laya.judge(prompt, deadline).await,
        Sorter::Quick => None,
    };
    decide_tier(prompt, judgement)
}

/// The execution for this message when choosing by difficulty, or `None` to use the
/// single chat model as before. A tier without a ready harness falls back too.
pub(crate) async fn route(state: &TierState, laya: &Laya, prompt: &str) -> Option<ModelExecution> {
    if !state.routing.enabled {
        return None;
    }
    let decision = decide(&state.routing, laya, prompt, CHAT_DEADLINE).await;
    tracing::info!(tier = ?decision.tier, source = ?decision.source, reason = %decision.reason, "chose a tier");
    let tier = state.tiers.get(&decision.tier)?;
    Some(match tier.execution.clone() {
        ModelExecution::Selected { harness, timeout } => ModelExecution::Selected {
            harness: Box::new(harness.with_route_note(route_note(&decision, tier.thinking))),
            timeout,
        },
        other @ (ModelExecution::Legacy | ModelExecution::Unavailable(_)) => other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sovra_core::ai_tiers::DecisionSource;

    struct Fixed(LayaJudgement);
    impl DifficultyJudge for Fixed {
        fn judge(&self, _: &str) -> Result<LayaJudgement, String> {
            Ok(self.0)
        }
    }

    /// Blocks until its sender is dropped or 300 ms pass.
    struct Slow(Mutex<std::sync::mpsc::Receiver<()>>);
    impl DifficultyJudge for Slow {
        fn judge(&self, _: &str) -> Result<LayaJudgement, String> {
            let _ = self
                .0
                .lock()
                .expect("lock")
                .recv_timeout(Duration::from_millis(300));
            Ok(LayaJudgement {
                difficulty: 3.0,
                confidence: 1.0,
            })
        }
    }

    struct Loader(Arc<dyn DifficultyJudge>);
    impl LayaLoader for Loader {
        fn load(&self, _: &Path) -> Result<Arc<dyn DifficultyJudge>, String> {
            Ok(self.0.clone())
        }
    }

    fn laya_with(dir: &Path, judge: Arc<dyn DifficultyJudge>) -> Laya {
        Laya::new(Some(LayaSetup {
            dir: dir.to_owned(),
            loader: Arc::new(Loader(judge)),
        }))
    }

    fn fake_install(dir: &Path) {
        for file in LAYA_FILES {
            let path = dir.join(file.local);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
            std::fs::File::create(&path)
                .and_then(|f| f.set_len(file.bytes))
                .expect("sized file");
        }
        std::fs::write(dir.join(MARKER), LAYA_REVISION).expect("marker");
    }

    const UNSURE: &str = "What should I cook for six people tonight with chickpeas and spinach?";

    #[tokio::test]
    async fn laya_is_used_only_once_installed_and_loaded() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("laya");
        let laya = laya_with(
            &dir,
            Arc::new(Fixed(LayaJudgement {
                difficulty: 2.8,
                confidence: 0.7,
            })),
        );
        assert_eq!(laya.status(), LayaStatus::NotInstalled);
        assert!(laya.load_blocking().is_err());
        assert_eq!(laya.judge(UNSURE, CHAT_DEADLINE).await, None);

        fake_install(&dir);
        assert_eq!(laya.status(), LayaStatus::Installed);
        laya.load_blocking().expect("loads");
        assert_eq!(laya.status(), LayaStatus::Ready);
        let routing = TierRouting {
            sorter: Sorter::Laya,
            ..TierRouting::default()
        };
        let decision = decide(&routing, &laya, UNSURE, CHAT_DEADLINE).await;
        assert_eq!(
            (decision.tier, decision.source),
            (ModelTier::Deep, DecisionSource::Laya)
        );

        let quick = decide(&TierRouting::default(), &laya, UNSURE, CHAT_DEADLINE).await;
        assert_eq!(quick.source, DecisionSource::QuickSorter);

        laya.forget();
        uninstall(&dir).await.expect("uninstalls");
        assert_eq!(laya.status(), LayaStatus::NotInstalled);
    }

    #[tokio::test]
    async fn a_slow_laya_falls_back_to_the_quick_sorter() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (_hold, wait) = std::sync::mpsc::channel();
        let laya = laya_with(temp.path(), Arc::new(Slow(Mutex::new(wait))));
        fake_install(temp.path());
        laya.load_blocking().expect("loads");
        let routing = TierRouting {
            sorter: Sorter::Laya,
            ..TierRouting::default()
        };
        let decision = decide(&routing, &laya, UNSURE, Duration::from_millis(20)).await;
        assert_eq!(decision.source, DecisionSource::QuickSorter);
    }

    #[test]
    fn a_wrong_revision_or_size_is_not_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        fake_install(temp.path());
        assert!(installed_at(temp.path()));
        std::fs::write(temp.path().join(MARKER), "old").expect("marker");
        assert!(!installed_at(temp.path()));
        fake_install(temp.path());
        std::fs::write(temp.path().join(LAYA_FILES[0].local), "x").expect("truncate");
        assert!(!installed_at(temp.path()));
    }

    #[test]
    fn notes_say_tier_thinking_and_reason() {
        let note = route_note(
            &TierDecision {
                tier: ModelTier::Deep,
                source: DecisionSource::QuickSorter,
                reason: "it contains code".into(),
            },
            ThinkingLevel::High,
        );
        assert_eq!(note, "Complex question · Think hard (it contains code)");
        assert_eq!(hex(&[0, 15, 255]), "000fff");
    }
}
