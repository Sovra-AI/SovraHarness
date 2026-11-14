use super::*;
use crate::training::library::tests::{card_for, micro_file};

fn peer(n: u8) -> TrainingPeer {
    TrainingPeer {
        device: DeviceId([n; 32]),
        name: "Family computer".into(),
        online: true,
        helping: false,
        rounds: 0,
    }
}

fn plan() -> TrainingPlan {
    TrainingPlan::recommended(&HardwareInfo::default(), false)
}

#[test]
fn helping_keeps_local_start_blocked_until_stopped_or_finished() {
    let mut state = TrainingState {
        phase: TrainingPhase::Ready,
        ..TrainingState::default()
    };
    assert!(!state.busy());
    assert!(!state.progress.is_complete());
    for phase in [
        HelpingPhase::Working,
        HelpingPhase::Waiting,
        HelpingPhase::Stopping,
        HelpingPhase::Stopped,
        HelpingPhase::Finished,
    ] {
        state.helping = Some(TrainingHelp {
            group: GroupId([1; 32]),
            peer: peer(2),
            phase,
            accepted_rounds: 0,
            progress: None,
            run: None,
            notice: None,
        });
        assert_eq!(
            state.busy(),
            matches!(
                phase,
                HelpingPhase::Working | HelpingPhase::Waiting | HelpingPhase::Stopping
            )
        );
    }
    state.progress.rounds = 1;
    state.progress.total_rounds = 1;
    assert!(state.progress.is_complete());
}

#[test]
fn state_folds_progress_errors_and_pause_acknowledgement() {
    let mut state = TrainingState::default();
    assert!(state.phase.busy());
    for phase in [
        TrainingPhase::Ready,
        TrainingPhase::Preparing,
        TrainingPhase::Running,
        TrainingPhase::Pausing,
        TrainingPhase::Paused,
        TrainingPhase::Complete,
        TrainingPhase::Unavailable,
    ] {
        let event = TrainingEvent::new(TrainingState {
            phase,
            error: Some("test".into()),
            ..TrainingState::default()
        });
        state.apply(&event);
        assert_eq!(state, *event.0);
        assert_eq!(
            phase.busy(),
            matches!(
                phase,
                TrainingPhase::Preparing | TrainingPhase::Running | TrainingPhase::Pausing
            )
        );
    }
}

#[test]
fn a_snapshot_carries_the_whole_page_and_replaces_what_was_there() {
    let file = micro_file(0.25);
    let card = card_for(&file);
    let mut metrics = TrainingMetrics::default();
    metrics.push(MetricPoint {
        round: 3,
        loss_milli: 4100,
        check_loss_milli: Some(4200),
        lr_micro: 3000,
        tokens_per_second: 2500,
    });
    metrics.set_sample(b"The garden", 3);
    let plan = plan();
    let architecture = plan.base.architecture().unwrap();
    let full = TrainingState {
        phase: TrainingPhase::Running,
        progress: TrainingProgress {
            rounds: 3,
            total_rounds: 12,
            tokens: 3072,
            loss_milli: Some(4100),
        },
        saved_at: Some("/data/training/run".into()),
        error: None,
        notice: Some("One model was skipped: the library is full.".into()),
        group: Some(TrainingGroup {
            id: GroupId([1; 32]),
            name: "Family".into(),
            this_device: DeviceId([9; 32]),
            peers: vec![
                TrainingPeer {
                    helping: true,
                    rounds: 2,
                    ..peer(2)
                },
                peer(3),
            ],
        }),
        sharing: Some(GroupId([1; 32])),
        helped_rounds: 2,
        helping: None,
        hardware: Some(HardwareInfo::default()),
        plan: Some(plan.summary(architecture, SAMPLE_TEXT.len())),
        metrics,
        models: vec![LibraryModel {
            card: card.clone(),
            saved_at: "/data/training/models/x.model".into(),
            delivery: Vec::new(),
        }],
        trial: Some(Trial {
            model: card.id,
            prompt: "The sun".into(),
            output: None,
            running: true,
        }),
    };
    let mut state = TrainingState::default();
    state.apply(&TrainingEvent::new(full.clone()));
    assert_eq!(state, full);
    assert_eq!(state.progress.percent(), 25);
    assert_eq!(state.group.as_ref().unwrap().helpers(), 1);
    assert_eq!(state.model(card.id).unwrap().card, card);
    assert!(state.model(ContentHash::of(b"other")).is_none());
    assert!(state.trying());
    // The plan in the state says which text, never the text.
    let shown = format!("{:?}", state.plan);
    assert!(!shown.contains("The sun warms"));
    state.apply(&TrainingEvent::new(TrainingState::default()));
    assert_eq!(state, TrainingState::default());
    assert!(!state.trying());
}

#[test]
fn progress_reads_as_a_percentage_at_every_edge() {
    let at = |rounds, total_rounds| {
        TrainingProgress {
            rounds,
            total_rounds,
            ..TrainingProgress::default()
        }
        .percent()
    };
    assert_eq!(at(0, 0), 0);
    assert_eq!(at(5, 0), 0);
    assert_eq!(at(0, 80), 0);
    assert_eq!(at(79, 80), 98);
    assert_eq!(at(80, 80), 100);
    assert_eq!(at(900, 80), 100);
    assert_eq!(at(u32::MAX, u32::MAX), 100);
}

#[test]
fn commands_are_checked_before_they_reach_the_engine() {
    let good = plan();
    assert_eq!(
        TrainingCommand::Start(Box::new(good.clone())).validate(),
        Ok(())
    );
    let mut named = good.clone();
    named.name.clear();
    assert!(TrainingCommand::Start(Box::new(named)).validate().is_err());
    let mut rounds = good.clone();
    rounds.settings.rounds = 0;
    assert!(TrainingCommand::Start(Box::new(rounds)).validate().is_err());
    let mut large = good.clone();
    large.base = TrainingBase::Fresh(BaseModel::Large);
    large.settings = TrainingSettings::recommended(
        TrainingPreset::Quick,
        BaseModel::Large.architecture(),
        4000,
        &HardwareInfo::default(),
    );
    assert_eq!(
        TrainingCommand::Start(Box::new(large.clone())).validate(),
        Ok(())
    );
    large.with_group = true;
    assert!(TrainingCommand::Start(Box::new(large)).validate().is_err());
    // Continuing a library model: the name and source are checked now, the rest
    // once the runtime has the model's shape.
    let mut onward = good;
    onward.base = TrainingBase::Continue(ContentHash::of(b"model"));
    assert_eq!(
        TrainingCommand::Start(Box::new(onward.clone())).validate(),
        Ok(())
    );
    onward.source = TrainingSource::Text("short".into());
    assert_eq!(
        TrainingCommand::Start(Box::new(onward)).validate(),
        Err(TrainingError::InvalidInput)
    );

    let model = ContentHash::of(b"model");
    assert_eq!(
        TrainingCommand::Try {
            model,
            prompt: String::new()
        }
        .validate(),
        Ok(())
    );
    assert!(
        TrainingCommand::Try {
            model,
            prompt: "x".repeat(MAX_TRIAL_PROMPT_BYTES + 1)
        }
        .validate()
        .is_err()
    );
    for command in [
        TrainingCommand::Load,
        TrainingCommand::Pause,
        TrainingCommand::Resume,
        TrainingCommand::Stop,
        TrainingCommand::StopHelping,
        TrainingCommand::Help(DeviceId([2; 32])),
        TrainingCommand::Share(model),
        TrainingCommand::StopSharing,
    ] {
        assert_eq!(command.validate(), Ok(()));
    }
}

#[test]
fn errors_read_as_plain_sentences() {
    for error in [
        TrainingError::InvalidWork,
        TrainingError::InvalidInput,
        TrainingError::InvalidSetting("Rounds must be between 20 and 100,000.".into()),
        TrainingError::InvalidFile("Choose a plain text file ending in .txt or .md.".into()),
        TrainingError::InvalidModel,
        TrainingError::InvalidCheckpoint,
        TrainingError::Step,
    ] {
        let text = error.to_string();
        assert!(text.ends_with('.'), "{text}");
        assert!(!text.contains("Error") && !text.contains("error"), "{text}");
    }
    assert_eq!(
        TrainingError::InvalidInput.to_string(),
        "Use between 32 bytes and 4 MB of text for training."
    );
    // The saved-run cap holds the largest model's compact state and its text.
    let largest = BaseModel::Large.architecture().parameters();
    let needed = crate::swarm::learn::scavenger::GridCheckpoint::byte_len(largest)
        + MAX_TEXT_BYTES
        + 64 * 1024;
    assert!(needed <= MAX_CHECKPOINT_BYTES);
}
