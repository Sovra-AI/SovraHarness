//! Teams and artifacts driven through the runtime's `Client`, without a store.

use sovra_core::artifacts::{
    ArtifactCommand, ArtifactDraft, ArtifactEvent, ArtifactKind, ArtifactSource,
};
use sovra_core::teams::{TeamCommand, TeamEvent};
use sovra_core::{AgUiEvent, Command, Event};

fn started() -> (sovra_runtime::Client, async_channel::Receiver<Event>) {
    let client = sovra_runtime::start("9.9.9").expect("runtime starts");
    let events = client.events();
    assert!(matches!(events.recv_blocking(), Ok(Event::Ready { .. })));
    (client, events)
}

fn next_team(events: &async_channel::Receiver<Event>) -> TeamEvent {
    loop {
        if let Event::Teams(event) = events.recv_blocking().expect("event") {
            return event;
        }
    }
}

fn next_artifact(events: &async_channel::Receiver<Event>) -> ArtifactEvent {
    loop {
        if let Event::Artifacts(event) = events.recv_blocking().expect("event") {
            return event;
        }
    }
}

#[test]
fn team_roster_seeds_edits_switches_off_builtins_and_resets() {
    let (client, events) = started();
    client.send(Command::Teams(TeamCommand::Load)).unwrap();
    let TeamEvent::Loaded { members } = next_team(&events) else {
        panic!("roster loads");
    };
    assert_eq!(members, sovra_core::teams::default_team());

    let mut priya = members.iter().find(|m| m.id == "priya").unwrap().clone();
    priya.tagline = "Loves libraries".into();
    client
        .send(Command::Teams(TeamCommand::SaveMember(priya.clone())))
        .unwrap();
    assert_eq!(next_team(&events), TeamEvent::MemberSaved(priya));

    client
        .send(Command::Teams(TeamCommand::RemoveMember {
            id: "priya".into(),
        }))
        .unwrap();
    let TeamEvent::MemberSaved(off) = next_team(&events) else {
        panic!("built-in members are switched off, not deleted");
    };
    assert!(!off.active);

    client
        .send(Command::Teams(TeamCommand::ResetMember {
            id: "priya".into(),
        }))
        .unwrap();
    let TeamEvent::MemberSaved(reset) = next_team(&events) else {
        panic!("reset restores the original");
    };
    assert_eq!(Some(reset), sovra_core::teams::builtin_member("priya"));

    let mut helper = sovra_core::teams::helper_templates().remove(0);
    helper.id = "theo".into();
    client
        .send(Command::Teams(TeamCommand::SaveMember(helper)))
        .unwrap();
    assert!(matches!(next_team(&events), TeamEvent::MemberSaved(_)));
    client
        .send(Command::Teams(TeamCommand::RemoveMember {
            id: "theo".into(),
        }))
        .unwrap();
    assert_eq!(
        next_team(&events),
        TeamEvent::MemberRemoved { id: "theo".into() }
    );
    client.send(Command::Shutdown).unwrap();
}

#[test]
fn a_team_turn_without_an_ai_explains_how_to_connect_one() {
    let (client, events) = started();
    client
        .send(Command::Teams(TeamCommand::Run {
            session_id: "chat".into(),
            prompt: "What does everyone think?".into(),
            history: vec![],
        }))
        .unwrap();
    loop {
        match events.recv_blocking().expect("event") {
            Event::AgUi {
                session_id,
                event: AgUiEvent::RunError { message },
            } => {
                assert_eq!(session_id, "chat");
                assert!(message.contains("Models"), "{message}");
                break;
            }
            Event::AgUi { event, .. } => panic!("no run without a model: {event:?}"),
            _ => {}
        }
    }
    client.send(Command::Shutdown).unwrap();
}

#[test]
fn artifacts_save_edit_export_open_and_delete() {
    let (client, events) = started();
    client
        .send(Command::Artifacts(ArtifactCommand::Save(ArtifactDraft {
            title: "Meal plan".into(),
            kind: ArtifactKind::Table,
            language: String::new(),
            content: "| Day | Meal |\n|---|---|\n| Mon | Soup |".into(),
            source: ArtifactSource::default(),
        })))
        .unwrap();
    let ArtifactEvent::Saved(saved) = next_artifact(&events) else {
        panic!("saved");
    };
    assert!(saved.id.starts_with("art-"));

    client
        .send(Command::Artifacts(ArtifactCommand::Edit {
            id: saved.id.clone(),
            title: "Meals".into(),
            content: "| Day | Meal |\n|---|---|\n| Tue | Pie |".into(),
        }))
        .unwrap();
    let ArtifactEvent::Saved(edited) = next_artifact(&events) else {
        panic!("edited");
    };
    assert_eq!((edited.title.as_str(), edited.versions.len()), ("Meals", 1));

    let dir = std::env::temp_dir().join(format!("sovra-artifact-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("meals.csv");
    client
        .send(Command::Artifacts(ArtifactCommand::Export {
            id: saved.id.clone(),
            path: path.to_string_lossy().into_owned(),
        }))
        .unwrap();
    assert!(matches!(
        next_artifact(&events),
        ArtifactEvent::Exported { .. }
    ));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "Day,Meal\nTue,Pie\n"
    );

    client
        .send(Command::Artifacts(ArtifactCommand::Open {
            id: saved.id.clone(),
        }))
        .unwrap();
    let ArtifactEvent::ReadyToOpen { path, .. } = next_artifact(&events) else {
        panic!("a browser view is written");
    };
    assert!(std::fs::read_to_string(&path).unwrap().contains("Meals"));

    client
        .send(Command::Artifacts(ArtifactCommand::Load))
        .unwrap();
    let ArtifactEvent::Loaded(all) = next_artifact(&events) else {
        panic!("loaded");
    };
    assert_eq!(all, [edited]);
    client
        .send(Command::Artifacts(ArtifactCommand::Delete {
            id: saved.id.clone(),
        }))
        .unwrap();
    assert_eq!(
        next_artifact(&events),
        ArtifactEvent::Deleted {
            id: saved.id.clone()
        }
    );
    client
        .send(Command::Artifacts(ArtifactCommand::Open { id: saved.id }))
        .unwrap();
    assert!(matches!(next_artifact(&events), ArtifactEvent::Error(_)));
    std::fs::remove_dir_all(dir).ok();
    client.send(Command::Shutdown).unwrap();
}
