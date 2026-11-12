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
            context: sovra_core::ConversationContext::default(),
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
    let ArtifactEvent::Deployed { url, .. } = next_artifact(&events) else {
        panic!("browser previews use a reachable HTTP address, never a private temporary file");
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let body = runtime.block_on(async { reqwest::get(url).await.unwrap().text().await.unwrap() });
    assert!(body.contains("Meals"));

    client
        .send(Command::Artifacts(ArtifactCommand::Load))
        .unwrap();
    let ArtifactEvent::Loaded(all) = next_artifact(&events) else {
        panic!("loaded");
    };
    assert_eq!(all, [*edited]);
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

#[test]
fn website_serves_saved_source_and_stops_without_deleting_it() {
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::time::Duration;

    let (client, events) = started();
    let content = "<!doctype html><html lang=\"en\"><title>Garden</title><h1>Garden 🌿</h1></html>";
    client
        .send(Command::Artifacts(ArtifactCommand::Save(ArtifactDraft {
            title: "Garden".into(),
            kind: ArtifactKind::Webpage,
            language: "html".into(),
            content: content.into(),
            source: ArtifactSource::default(),
        })))
        .unwrap();
    let ArtifactEvent::Saved(saved) = next_artifact(&events) else {
        panic!("saved")
    };
    client
        .send(Command::Artifacts(ArtifactCommand::Deploy {
            id: saved.id.clone(),
        }))
        .unwrap();
    let ArtifactEvent::Deployed { id, url } = next_artifact(&events) else {
        panic!("running")
    };
    assert_eq!(id, saved.id);
    let address = url.strip_prefix("http://").unwrap().trim_end_matches('/');
    assert!(address.starts_with("127.0.0.1:"));
    let request = |method: &str, path: &str, host: &str| {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    };
    let response = request("GET", "/", address);
    assert!(response.starts_with("HTTP/1.1 200"));
    assert_eq!(response.split_once("\r\n\r\n").unwrap().1, content);
    assert!(response.contains(&format!("Content-Length: {}", content.len())));
    assert!(request("GET", "/", "example.com").starts_with("HTTP/1.1 403"));
    assert!(request("GET", "/../../etc/passwd", address).starts_with("HTTP/1.1 404"));
    assert!(request("POST", "/", address).starts_with("HTTP/1.1 405"));
    assert_eq!(
        request("HEAD", "/", address)
            .split_once("\r\n\r\n")
            .unwrap()
            .1,
        ""
    );
    client
        .send(Command::Artifacts(ArtifactCommand::StopDeployment {
            id: id.clone(),
        }))
        .unwrap();
    assert_eq!(
        next_artifact(&events),
        ArtifactEvent::DeploymentStopped { id }
    );
    assert!(
        TcpStream::connect(address).is_err(),
        "stopped listener is closed before acknowledgement"
    );
    client
        .send(Command::Artifacts(ArtifactCommand::Load))
        .unwrap();
    let ArtifactEvent::Loaded(artifacts) = next_artifact(&events) else {
        panic!("library")
    };
    assert_eq!(artifacts, vec![*saved]);
    client.send(Command::Shutdown).unwrap();
}

#[test]
fn organization_updates_metadata_without_rewriting_source() {
    let (client, events) = started();
    client
        .send(Command::Artifacts(ArtifactCommand::Save(ArtifactDraft {
            title: "Garden".into(),
            kind: ArtifactKind::Webpage,
            language: "html".into(),
            content: "<h1>Garden</h1>".into(),
            source: ArtifactSource::default(),
        })))
        .unwrap();
    let ArtifactEvent::Saved(original) = next_artifact(&events) else {
        panic!("saved")
    };
    client
        .send(Command::Artifacts(ArtifactCommand::Organize {
            id: original.id.clone(),
            collection: " Garden club ".into(),
            pinned: true,
        }))
        .unwrap();
    let ArtifactEvent::Saved(organized) = next_artifact(&events) else {
        panic!("organized")
    };
    assert_eq!(organized.collection, "Garden club");
    assert!(organized.pinned);
    assert_eq!(organized.content, original.content);
    assert_eq!(organized.versions, original.versions);
    client
        .send(Command::Artifacts(ArtifactCommand::Load))
        .unwrap();
    let ArtifactEvent::Loaded(loaded) = next_artifact(&events) else {
        panic!("loaded")
    };
    assert_eq!(loaded, [*organized]);
    client.send(Command::Shutdown).unwrap();
}
