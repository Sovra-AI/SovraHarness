//! Desktop commands driving two real runtimes, the Burn engine, real folders and
//! the real network: a group trains together and both computers end up with the
//! same finished model.

use std::path::Path;

use sovra_core::group::DeviceId;
use sovra_core::training::{
    BaseModel, HardwareInfo, HelpingPhase, LibraryModel, ModelId, ModelOrigin, SAMPLE_TEXT,
    TrainingBase, TrainingCommand, TrainingEngine as _, TrainingEvent, TrainingPhase, TrainingPlan,
    TrainingPreset, TrainingSettings, TrainingSource, TrainingState, model_id,
};
use sovra_train::BurnTrainingEngine;

use super::*;

/// Real learning in an unoptimised test build, on a machine that may be running
/// every other test at the same time.
const LEARN: Duration = Duration::from_secs(240);

/// A loss (in thousandths) only a model that has learned something reaches. One
/// that knows nothing, with every byte value as likely as any other, is at
/// `ln 256` = 5545.
const LEARNED_MILLI: u32 = 3600;

impl Computer {
    fn training(&self, command: TrainingCommand) {
        self.client
            .send(Command::Training(command))
            .expect("training command accepted");
    }

    async fn training_state(&self, predicate: impl Fn(&TrainingState) -> bool) -> TrainingState {
        self.training_within(WAIT, predicate)
            .await
            .expect("training state deadline")
    }

    async fn training_within(
        &self,
        limit: Duration,
        predicate: impl Fn(&TrainingState) -> bool,
    ) -> Option<TrainingState> {
        tokio::time::timeout(limit, async {
            loop {
                if let Event::Training(TrainingEvent(state)) =
                    self.events.recv().await.expect("runtime events")
                    && predicate(&state)
                {
                    return *state;
                }
            }
        })
        .await
        .ok()
    }

    /// Opens the Training page again and again until it shows what is expected.
    /// Opening the page is what makes a computer look for its group's models.
    async fn training_page(
        &self,
        what: &str,
        predicate: impl Fn(&TrainingState) -> bool,
    ) -> TrainingState {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            self.training(TrainingCommand::Load);
            if let Some(state) = self
                .training_within(Duration::from_secs(1), &predicate)
                .await
            {
                return state;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
        }
    }

    /// Turns this computer's "Help the group train" on or off.
    async fn permit(&self, on: bool) {
        self.send(GroupCommand::SetTrainingHelp(on));
        self.wait("help the group train saved", |event| {
            matches!(event, GroupEvent::TrainingHelp(saved) if saved == on).then_some(())
        })
        .await;
    }

    /// Types a beginning into Try it and returns what the model wrote after it.
    async fn try_model(&self, model: ModelId, prompt: &str) -> String {
        self.training(TrainingCommand::Try {
            model,
            prompt: prompt.into(),
        });
        let state = self
            .training_state(|state| {
                state
                    .trial
                    .as_ref()
                    .is_some_and(|trial| trial.model == model && !trial.running)
            })
            .await;
        assert_eq!(state.error, None);
        state
            .trial
            .and_then(|trial| trial.output)
            .expect("the model wrote something")
    }

    async fn shut_down(self) -> TempDir {
        let Self {
            client,
            events: _,
            dir,
        } = self;
        tokio::task::spawn_blocking(move || client.shutdown_blocking().expect("runtime stops"))
            .await
            .expect("stopped");
        dir
    }
}

async fn join(host: &Computer, helper: &Computer) {
    host.send(GroupCommand::Create {
        name: "Training group".into(),
        your_name: "Host".into(),
        computer_name: "Host computer".into(),
    });
    host.roster("created training group", |state| {
        state.name == "Training group"
    })
    .await;
    host.send(GroupCommand::InvitePerson {
        name: "Helper".into(),
        role: Role::Member,
    });
    let link = host
        .wait("training invite", |event| match event {
            GroupEvent::InviteReady { link, .. } => Some(link),
            _ => None,
        })
        .await;
    helper.send(GroupCommand::Join {
        link,
        computer_name: "Helper computer".into(),
    });
    helper
        .roster("helper joined training group", |state| {
            state.people.len() == 2
        })
        .await;
    host.roster("host sees helper", |state| state.people.len() == 2)
        .await;
}

/// The Quick try settings for `base` on the built-in sample, with `rounds`
/// rounds and nothing that could end the run early.
fn plan(name: &str, base: BaseModel, rounds: u32, with_group: bool) -> TrainingPlan {
    let mut settings = TrainingSettings::recommended(
        TrainingPreset::Quick,
        base.architecture(),
        SAMPLE_TEXT.len(),
        &HardwareInfo::default(),
    );
    settings.rounds = rounds;
    settings.warmup_rounds = settings.warmup_rounds.min(rounds);
    settings.stop_when_stalled = 0;
    TrainingPlan {
        name: name.into(),
        base: TrainingBase::Fresh(base),
        source: TrainingSource::Sample,
        preset: None,
        settings,
        with_group,
    }
}

/// A Starter run long enough that it is still going when the test ends.
fn long_run(with_group: bool) -> TrainingPlan {
    plan("Long run", BaseModel::Starter, 100_000, with_group)
}

/// The round the saved run on disk has reached, read the way the app reads it.
fn saved_round(path: &str) -> u32 {
    let saved = std::fs::read(path).expect("saved run");
    BurnTrainingEngine::processor()
        .restore(&saved)
        .expect("saved run opens")
        .progress()
        .rounds
}

fn helping(
    state: &TrainingState,
    test: impl Fn(&sovra_core::training::TrainingHelp) -> bool,
) -> bool {
    state.helping.as_ref().is_some_and(test)
}

/// The model file a library entry points at, checked against its name.
fn model_file(model: &LibraryModel) -> Vec<u8> {
    let file = std::fs::read(Path::new(&model.saved_at)).expect("model file");
    assert_eq!(model_id(&file), model.card.id, "the file is the model");
    assert_eq!(file.len() as u64, model.card.bytes);
    file
}

fn device(key: u8) -> DeviceId {
    sovra_mesh::Mesh::device_for(&[key; 32])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_group_trains_a_small_model_together_and_both_computers_get_it() {
    let host = Computer::configured("training-together-host", 97, &[], true, true);
    let helper = Computer::configured("training-together-helper", 98, &[], true, true);
    join(&host, &helper).await;
    // The helper's only part in this: it turned "Help the group train" on.
    helper.permit(true).await;

    host.training(TrainingCommand::Start(Box::new(plan(
        "Garden writer",
        BaseModel::Small,
        160,
        true,
    ))));
    let started = host
        .training_state(|state| state.phase == TrainingPhase::Running && state.sharing.is_some())
        .await;
    assert_eq!(started.error, None);

    // Nobody picked a computer: the helper finds the run and joins by itself.
    let joined = helper
        .training_within(LEARN, |state| {
            helping(state, |help| help.accepted_rounds >= 1)
        })
        .await
        .expect("the helper joins the run by itself");
    let help = joined.helping.expect("helping");
    assert_eq!(help.peer.device, device(97));
    let run = help.run.expect("automatic help knows which run it joined");
    assert_eq!(run.name, "Garden writer");
    assert_eq!(run.base, BaseModel::Small);
    assert_eq!(
        joined.phase,
        TrainingPhase::Ready,
        "it trains nothing of its own"
    );

    let done = host
        .training_within(LEARN, |state| {
            state.phase == TrainingPhase::Complete && !state.models.is_empty()
        })
        .await
        .expect("the run completes");
    assert_eq!(done.error, None);
    assert_eq!(done.progress.rounds, 160);
    assert!(done.progress.is_complete());
    assert!(done.helped_rounds >= 1, "the helper's rounds were used");
    assert_eq!(done.sharing, None, "a finished run hands out no more text");
    assert!(
        done.metrics.points.len() >= 2,
        "the page had a line to draw: {:?}",
        done.metrics.points
    );
    assert!(done.metrics.sample.is_some(), "it showed what it writes");
    let trained = done.models[0].clone();
    assert_eq!(trained.card.name, "Garden writer");
    assert_eq!(trained.card.base, BaseModel::Small);
    assert_eq!(trained.card.origin, ModelOrigin::TrainedHere);
    assert_eq!(trained.card.rounds, 160);
    let loss = trained.card.loss_milli.expect("a measured loss");
    assert!(
        loss < LEARNED_MILLI,
        "the model learned from the sample: loss {loss}"
    );
    let id = trained.card.id;

    // The same model, by the same name, is now in the helper's library too.
    let arrived = helper
        .training_page("the model to arrive", |state| state.model(id).is_some())
        .await;
    let received = arrived.model(id).expect("just seen").clone();
    assert_eq!(received.card.id, id);
    assert_eq!(received.card.name, "Garden writer");
    assert_eq!(
        received.card.origin,
        ModelOrigin::Received { from: device(97) }
    );
    assert_eq!(received.card.trained_by, "Host · Host computer");
    assert_eq!(received.card.group, trained.card.group);
    assert!(trained.card.group.is_some());
    let (here, there) = (model_file(&trained), model_file(&received));
    assert_eq!(here, there, "both computers hold the same file");
    assert_ne!(trained.saved_at, received.saved_at);
    assert!(
        received
            .saved_at
            .starts_with(&*helper.dir.0.to_string_lossy()),
        "the helper keeps its own copy: {}",
        received.saved_at
    );

    // The host's page counts the delivery.
    let counted = host
        .training_page("the delivery to be counted", |state| {
            state
                .model(id)
                .is_some_and(|model| model.delivered() == (1, 1))
        })
        .await;
    assert_eq!(
        counted.model(id).expect("listed").delivery[0].device,
        device(98)
    );

    // Try it works on both computers, and the two files write the same text.
    for computer in [&host, &helper] {
        let written = computer.try_model(id, "The garden").await;
        assert!(!written.trim().is_empty(), "Try it wrote: {written:?}");
    }
    let write = |file: &[u8]| {
        BurnTrainingEngine::processor()
            .generate(file, "The garden", 80, 800, 7)
            .expect("the model writes")
    };
    assert_eq!(write(&here), write(&there));
    assert_ne!(write(&here).trim(), "");
    assert_eq!(
        BurnTrainingEngine::processor()
            .inspect(&there)
            .expect("a sound model")
            .id,
        id
    );

    let _host = host.shut_down().await;
    let _helper = helper.shut_down().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restarting_a_shared_run_does_not_resume_sharing_or_accept_work() {
    let host = Computer::configured("training-restart-host", 95, &[], true, true);
    let helper = Computer::configured("training-restart-helper", 96, &[], true, true);
    join(&host, &helper).await;
    helper.permit(true).await;
    host.training(TrainingCommand::Start(Box::new(long_run(true))));
    let shared = host
        .training_state(|state| state.sharing.is_some() && state.helped_rounds >= 1)
        .await;
    let saved_at = shared.saved_at.expect("a saved run");

    // From here on the helper only helps when its person says so.
    helper.permit(false).await;
    helper
        .training_state(|state| !state.busy() && state.helping.is_some())
        .await;
    let kept = host.shut_down().await;
    let durable_round = saved_round(&saved_at);
    assert!(durable_round >= 1);

    let reopened = Computer::configured("training-restart-host", 95, &[], true, false);
    reopened
        .roster("restored group", |state| state.people.len() == 2)
        .await;
    let restored = reopened
        .training_page("the saved run", |state| {
            state.phase == TrainingPhase::Paused
        })
        .await;
    assert!(
        restored.sharing.is_none(),
        "sharing starts off after restart"
    );
    assert_eq!(restored.progress.rounds, durable_round);

    // Asking by hand gets nothing: a restart is not a new "train with my group".
    helper.training(TrainingCommand::Help(device(95)));
    let refused = helper
        .training_state(|state| helping(state, |help| help.run.is_none() && help.notice.is_some()))
        .await;
    assert_eq!(
        refused.helping.expect("helping").accepted_rounds,
        0,
        "restart must not grant fresh training work from a paused run"
    );
    // Nor does the helper's standing permission, turned back on.
    helper.training(TrainingCommand::StopHelping);
    helper.training_state(|state| !state.busy()).await;
    helper.permit(true).await;
    assert!(
        helper
            .training_within(Duration::from_secs(8), |state| {
                helping(state, |help| help.phase.active())
            })
            .await
            .is_none(),
        "a restored run must not be found as a shared run"
    );
    let after = reopened
        .training_page("the saved run again", |state| {
            state.phase == TrainingPhase::Paused
        })
        .await;
    assert!(after.sharing.is_none());
    assert_eq!(after.helped_rounds, 0);
    assert_eq!(saved_round(&saved_at), durable_round);

    let _reopened = reopened.shut_down().await;
    let _helper = helper.shut_down().await;
    drop(kept);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leaving_the_group_stops_helping_without_a_separate_stop_command() {
    let host = Computer::configured("training-leave-host", 93, &[], true, true);
    let helper = Computer::configured("training-leave-helper", 94, &[], true, true);
    join(&host, &helper).await;
    helper.permit(true).await;
    host.training(TrainingCommand::Start(Box::new(long_run(true))));
    helper
        .training_state(|state| helping(state, |help| help.accepted_rounds >= 1))
        .await;
    // Consume the host's pre-acceptance snapshots before comparing counters.
    // The helper's acknowledgement does not drain the host's event queue.
    host.training_state(|state| state.helped_rounds >= 1).await;

    helper.send(GroupCommand::Leave);
    helper
        .wait("helper left", |event| {
            matches!(event, GroupEvent::LeftGroup).then_some(())
        })
        .await;
    let stopped = helper
        .training_page("helping to stop", |state| {
            helping(state, |help| !help.phase.active())
        })
        .await;
    assert!(!stopped.busy());
    let help = stopped.helping.expect("helping");
    assert_eq!(help.phase, HelpingPhase::Stopped);
    assert!(help.notice.is_some(), "the page says why it stopped");

    // Membership propagation is asynchronous; compare after both computers
    // have observed the departure, not after an arbitrary network delay.
    host.roster("host observed helper leaving", |state| {
        state.people.len() == 1
    })
    .await;

    // The host carries on alone, and nothing more from that computer is used.
    let before = host
        .training_page("the host's run", |state| {
            state.phase == TrainingPhase::Running
        })
        .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    // The host has been publishing states all along. Without emptying the
    // queue, an old one from before the helper's last round was counted would
    // answer for the present.
    while host.events.try_recv().is_ok() {}
    let later = host
        .training_page("more rounds", |state| {
            state.progress.rounds > before.progress.rounds
        })
        .await;
    let settled = later.helped_rounds;
    assert!(settled >= 1, "the round given before leaving still counts");
    tokio::time::sleep(Duration::from_secs(3)).await;
    while host.events.try_recv().is_ok() {}
    let last = host
        .training_page("the host's run", |state| {
            state.progress.rounds > later.progress.rounds
        })
        .await;
    assert_eq!(last.helped_rounds, settled);

    let _host = host.shut_down().await;
    let _helper = helper.shut_down().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sharing_and_helping_require_consent_save_before_ack_and_stop_on_request() {
    let host = Computer::configured("training-host", 91, &[], true, true);
    let helper = Computer::configured("training-helper", 92, &[], true, true);
    join(&host, &helper).await;
    let host_id = device(91);
    helper.training(TrainingCommand::Start(Box::new(long_run(false))));
    helper
        .training_state(|state| state.progress.rounds >= 1)
        .await;
    helper.training(TrainingCommand::Pause);
    let own = helper
        .training_state(|state| state.phase == TrainingPhase::Paused)
        .await;
    let own_bytes = std::fs::read(own.saved_at.as_ref().unwrap()).unwrap();

    // Merely joining the group, or asking to help, starts no work on the host.
    helper.training(TrainingCommand::Help(host_id));
    let waiting = helper
        .training_state(|state| helping(state, |help| help.notice.is_some()))
        .await;
    assert_eq!(waiting.helping.as_ref().unwrap().accepted_rounds, 0);
    let idle = host
        .training_page("an idle host", |state| state.phase == TrainingPhase::Ready)
        .await;
    assert!(idle.sharing.is_none());

    // A general group permission cannot silently expose a local-only run.
    host.permit(true).await;
    host.training(TrainingCommand::Start(Box::new(long_run(false))));
    let private = host
        .training_state(|state| state.progress.rounds >= 3)
        .await;
    assert!(private.sharing.is_none());
    // The helper is still asking every couple of seconds; give it time to try.
    tokio::time::sleep(Duration::from_secs(5)).await;
    host.training(TrainingCommand::Pause);
    let private = host
        .training_state(|state| state.phase == TrainingPhase::Paused)
        .await;
    assert!(private.sharing.is_none());
    assert_eq!(private.helped_rounds, 0);
    let asked = helper
        .training_page("the helper still waiting", |state| {
            helping(state, |help| help.phase.active())
        })
        .await;
    assert_eq!(asked.helping.as_ref().unwrap().accepted_rounds, 0);

    // Only "train with my group" lets the helper in.
    host.training(TrainingCommand::Start(Box::new(long_run(true))));
    let shared = host.training_state(|state| state.sharing.is_some()).await;
    assert!(
        shared
            .group
            .as_ref()
            .is_some_and(|group| group.peers.len() == 1)
    );
    let helped = helper
        .training_state(|state| helping(state, |help| help.accepted_rounds >= 2))
        .await;
    let acknowledged = helped
        .helping
        .as_ref()
        .unwrap()
        .progress
        .as_ref()
        .unwrap()
        .rounds;
    let saved_at = host
        .training_state(|state| state.sharing.is_some() && state.saved_at.is_some())
        .await
        .saved_at
        .expect("a saved run");
    assert!(
        saved_round(&saved_at) >= acknowledged,
        "a result is on disk before the helper is told it was accepted"
    );

    host.training(TrainingCommand::Pause);
    let pause_all = host
        .training_state(|state| state.phase == TrainingPhase::Paused)
        .await;
    helper
        .training_state(|state| helping(state, |help| help.notice.is_some()))
        .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        saved_round(&saved_at),
        pause_all.progress.rounds,
        "Pause stops remote merges as well as local work"
    );
    host.training(TrainingCommand::Resume);
    host.training_state(|state| state.phase == TrainingPhase::Running)
        .await;

    host.training(TrainingCommand::StopSharing);
    let revoked = host
        .training_state(|state| state.sharing.is_none() && state.helped_rounds > 0)
        .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let later = host
        .training_page("rounds after sharing stopped", |state| {
            state.progress.rounds >= revoked.progress.rounds + 3
        })
        .await;
    assert_eq!(later.helped_rounds, revoked.helped_rounds);
    assert_eq!(
        later.phase,
        TrainingPhase::Running,
        "the run carries on alone"
    );

    helper.training(TrainingCommand::StopHelping);
    let stopped = helper
        .training_state(|state| helping(state, |help| help.phase == HelpingPhase::Stopped))
        .await;
    assert_eq!(stopped.progress, own.progress);
    assert_eq!(stopped.saved_at, own.saved_at);
    assert_eq!(
        std::fs::read(own.saved_at.as_ref().unwrap()).unwrap(),
        own_bytes,
        "helping never touches this computer's own run"
    );

    host.training(TrainingCommand::Pause);
    let paused = host
        .training_state(|state| state.phase == TrainingPhase::Paused)
        .await;
    let kept = host.shut_down().await;
    let reopened = Computer::configured("training-host", 91, &[], true, false);
    let restored = reopened
        .training_page("the saved run", |state| {
            state.phase == TrainingPhase::Paused
        })
        .await;
    assert_eq!(restored.progress, paused.progress);
    assert!(restored.sharing.is_none());
    assert!(restored.helping.is_none());
    let _reopened = reopened.shut_down().await;
    let _helper = helper.shut_down().await;
    drop(kept);
}
