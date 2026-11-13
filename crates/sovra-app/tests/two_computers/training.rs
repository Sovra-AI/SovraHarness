//! Desktop commands driving two real runtimes, Burn learners, stores and peers.

use sovra_core::training::{
    HelpingPhase, TrainingCommand, TrainingEvent, TrainingPhase, TrainingSource, TrainingState,
};

use super::*;

impl Computer {
    fn training(&self, command: TrainingCommand) {
        self.client
            .send(Command::Training(command))
            .expect("training command accepted");
    }

    async fn training_state(&self, predicate: impl Fn(&TrainingState) -> bool) -> TrainingState {
        tokio::time::timeout(WAIT, async {
            loop {
                if let Event::Training(TrainingEvent(state)) =
                    self.events.recv().await.expect("runtime events")
                    && predicate(&state)
                {
                    return state;
                }
            }
        })
        .await
        .expect("training state deadline")
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

fn saved_round(path: &str) -> u64 {
    let data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).expect("saved checkpoint"))
            .expect("checkpoint JSON");
    data["grid"]["round"].as_u64().expect("checkpoint round")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restarting_a_shared_run_does_not_resume_sharing_or_accept_work() {
    let host = Computer::configured("training-restart-host", 95, &[], true, true);
    let helper = Computer::configured("training-restart-helper", 96, &[], true, true);
    join(&host, &helper).await;
    host.training(TrainingCommand::StartShared(TrainingSource::Sample));
    let shared = host.training_state(|state| state.progress.rounds > 0).await;
    let Computer {
        client,
        events: _,
        dir: kept,
    } = host;
    tokio::task::spawn_blocking(move || client.shutdown_blocking().unwrap())
        .await
        .unwrap();
    let durable_round = saved_round(shared.saved_at.as_ref().unwrap());
    let reopened = Computer::configured("training-restart-host", 95, &[], true, false);
    reopened
        .roster("restored group", |state| state.people.len() == 2)
        .await;
    helper.training(TrainingCommand::Help(sovra_mesh::Mesh::device_for(
        &[95; 32],
    )));
    let response = helper
        .training_state(|state| {
            state
                .helping
                .as_ref()
                .is_some_and(|help| help.notice.is_some() || help.accepted_rounds > 0)
        })
        .await;
    assert_eq!(
        response.helping.as_ref().unwrap().accepted_rounds,
        0,
        "restart must not grant fresh training work from a paused run"
    );
    // Discard startup events; inspect the state after group and training startup
    // have both completed, rather than accepting an earlier paused snapshot.
    tokio::time::sleep(Duration::from_millis(350)).await;
    while reopened.events.try_recv().is_ok() {}
    reopened.training(TrainingCommand::Load);
    let restored = reopened
        .training_state(|state| state.phase == TrainingPhase::Paused)
        .await;
    assert!(
        restored.sharing.is_none(),
        "sharing starts off after restart"
    );
    assert_eq!(
        saved_round(shared.saved_at.as_ref().unwrap()),
        durable_round
    );
    tokio::task::spawn_blocking(move || reopened.client.shutdown_blocking().unwrap())
        .await
        .unwrap();
    tokio::task::spawn_blocking(move || helper.client.shutdown_blocking().unwrap())
        .await
        .unwrap();
    drop(kept);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leaving_the_group_stops_helping_without_a_separate_stop_command() {
    let host = Computer::configured("training-leave-host", 93, &[], true, true);
    let helper = Computer::configured("training-leave-helper", 94, &[], true, true);
    join(&host, &helper).await;
    helper.training(TrainingCommand::Help(sovra_mesh::Mesh::device_for(
        &[93; 32],
    )));
    helper
        .training_state(|state| {
            state
                .helping
                .as_ref()
                .is_some_and(|help| help.notice.is_some())
        })
        .await;
    helper.send(GroupCommand::Leave);
    helper
        .wait("helper left", |event| {
            matches!(event, GroupEvent::LeftGroup).then_some(())
        })
        .await;
    helper.training(TrainingCommand::Load);
    let stopped = tokio::time::timeout(
        Duration::from_secs(8),
        helper.training_state(|state| {
            state
                .helping
                .as_ref()
                .is_some_and(|help| help.phase == HelpingPhase::Stopped)
        }),
    )
    .await
    .expect("leaving must release the helping state");
    assert!(!stopped.busy());
    assert!(stopped.helping.as_ref().unwrap().notice.is_some());
    tokio::task::spawn_blocking(move || host.client.shutdown_blocking().unwrap())
        .await
        .unwrap();
    tokio::task::spawn_blocking(move || helper.client.shutdown_blocking().unwrap())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sharing_and_helping_require_consent_save_before_ack_and_stop_on_request() {
    let host = Computer::configured("training-host", 91, &[], true, true);
    let helper = Computer::configured("training-helper", 92, &[], true, true);
    join(&host, &helper).await;
    let host_id = sovra_mesh::Mesh::device_for(&[91; 32]);
    helper.training(TrainingCommand::Start(TrainingSource::Sample));
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
        .training_state(|state| {
            state
                .helping
                .as_ref()
                .is_some_and(|help| help.notice.is_some())
        })
        .await;
    assert_eq!(waiting.helping.as_ref().unwrap().accepted_rounds, 0);
    host.training(TrainingCommand::Load);
    assert_eq!(
        host.training_state(|_| true).await.phase,
        TrainingPhase::Ready
    );

    // A general group permission cannot silently expose a local-only run.
    host.send(GroupCommand::SetTrainingHelp(true));
    host.wait("group training allowed", |event| {
        matches!(event, GroupEvent::TrainingHelp(true)).then_some(())
    })
    .await;
    host.training(TrainingCommand::Start(TrainingSource::Sample));
    let private = host
        .training_state(|state| state.progress.rounds >= 3)
        .await;
    assert!(private.sharing.is_none());
    assert_eq!(private.helped_rounds, 0);
    host.training(TrainingCommand::Pause);
    host.training_state(|state| state.phase == TrainingPhase::Paused)
        .await;

    host.training(TrainingCommand::StartShared(TrainingSource::Sample));
    let shared = host.training_state(|state| state.sharing.is_some()).await;
    assert!(
        shared
            .group
            .as_ref()
            .is_some_and(|group| group.peers.len() == 1)
    );
    let helped = helper
        .training_state(|state| {
            state
                .helping
                .as_ref()
                .is_some_and(|help| help.accepted_rounds >= 2)
        })
        .await;
    let acknowledged = helped
        .helping
        .as_ref()
        .unwrap()
        .progress
        .as_ref()
        .unwrap()
        .rounds;
    assert!(saved_round(shared.saved_at.as_ref().unwrap()) >= u64::from(acknowledged));

    host.training(TrainingCommand::Pause);
    let pause_all = host
        .training_state(|state| state.phase == TrainingPhase::Paused)
        .await;
    helper
        .training_state(|state| {
            state
                .helping
                .as_ref()
                .is_some_and(|help| help.notice.is_some())
        })
        .await;
    assert_eq!(
        saved_round(shared.saved_at.as_ref().unwrap()),
        u64::from(pause_all.progress.rounds),
        "Pause stops remote merges as well as local work"
    );
    host.training(TrainingCommand::Resume);
    host.training_state(|state| state.phase == TrainingPhase::Running)
        .await;

    host.training(TrainingCommand::StopSharing);
    let revoked = host
        .training_state(|state| state.sharing.is_none() && state.helped_rounds > 0)
        .await;
    let later = host
        .training_state(|state| state.progress.rounds >= revoked.progress.rounds + 3)
        .await;
    assert_eq!(later.helped_rounds, revoked.helped_rounds);
    helper.training(TrainingCommand::Pause);
    let stopped = helper
        .training_state(|state| {
            state
                .helping
                .as_ref()
                .is_some_and(|help| help.phase == HelpingPhase::Stopped)
        })
        .await;
    assert_eq!(stopped.progress, own.progress);
    assert_eq!(stopped.saved_at, own.saved_at);
    assert_eq!(
        std::fs::read(own.saved_at.as_ref().unwrap()).unwrap(),
        own_bytes
    );

    host.training(TrainingCommand::Pause);
    let paused = host
        .training_state(|state| state.phase == TrainingPhase::Paused)
        .await;
    let Computer {
        client,
        events: _,
        dir: kept,
    } = host;
    tokio::task::spawn_blocking(move || client.shutdown_blocking().unwrap())
        .await
        .unwrap();
    let reopened = Computer::configured("training-host", 91, &[], true, false);
    let restored = reopened
        .training_state(|state| state.phase == TrainingPhase::Paused)
        .await;
    assert_eq!(restored.progress, paused.progress);
    assert!(restored.sharing.is_none());
    assert!(restored.helping.is_none());
    tokio::task::spawn_blocking(move || reopened.client.shutdown_blocking().unwrap())
        .await
        .unwrap();
    tokio::task::spawn_blocking(move || helper.client.shutdown_blocking().unwrap())
        .await
        .unwrap();
    drop(kept);
}
