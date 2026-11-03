//! Native knowledge review crosses the real runtime and reopened local file storage.
#![allow(clippy::unwrap_used, reason = "integration fixtures fail loudly")]

use sovra_core::{
    AgentConfig, Command, Event,
    file_index::{graph::*, *},
};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Observer;
impl ScanObserver for Observer {
    fn cancellation(&self) -> Option<ScanCancellation> {
        None
    }
    fn progress(&self, _: &IndexedRoot) {}
}
fn wait(events: &mpsc::Receiver<Event>, predicate: impl Fn(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let event = events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if predicate(&event) {
            return event;
        }
    }
}

#[test]
fn saved_file_relationship_review_reopens_evidence_and_revokes_it_through_runtime_events() {
    let fixture = Fixture(std::env::temp_dir().join(format!(
            "sovra-file-knowledge-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    let folder = fixture.0.join("sources");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("brief.txt"),
        "The Orchid garden project starts in spring.",
    )
    .unwrap();
    let store = sovra_files::LocalFileIndex::new(fixture.0.join("index"));
    let root = store.add_root(&folder).unwrap();
    store.set_auto_refresh(&root.id, false).unwrap();
    store.scan(&root.id, &Observer).unwrap();
    let passage = store
        .search(&IndexQuery {
            text: "Orchid".into(),
            roots: vec![root.id.clone()],
            path_prefix: None,
            limit: 1,
        })
        .unwrap()
        .remove(0);
    let proposal = store
        .propose(
            &GraphDraft {
                title: "Garden brief".into(),
                evidence: vec![passage.reference.clone()],
                entities: vec![EntityDraft {
                    local_id: "garden".into(),
                    kind: EntityKind::Project,
                    label: "Orchid".into(),
                    evidence: vec![0],
                }],
                relations: vec![RelationDraft {
                    subject: DraftEndpoint::New("garden".into()),
                    object: DraftEndpoint::Source(0),
                    relation: RelationKind::GroundedIn,
                    statement: "The Orchid project appears in this brief.".into(),
                    evidence: vec![0],
                    confidence: None,
                    valid_time: None,
                    replaces: None,
                }],
            },
            &GraphOrigin {
                session_id: "fixture".into(),
                invocation_id: "one".into(),
                agent: "Fixture helper".into(),
            },
        )
        .unwrap();
    store.decide(proposal.id, true).unwrap();
    drop(store);
    let store = Arc::new(sovra_files::LocalFileIndex::new(fixture.0.join("index")));
    let client = sovra_runtime::start_with_file_index(
        "test",
        AgentConfig::default(),
        None,
        None,
        None,
        None,
        Some(store),
    )
    .unwrap();
    let input = client.events();
    let (sender, events) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        while let Ok(event) = input.recv_blocking() {
            let stopped = event == Event::Stopped;
            if sender.send(event).is_err() || stopped {
                break;
            }
        }
    });
    wait(&events, |e| {
        matches!(e, Event::FileIndex(IndexEvent::KnowledgePending(_)))
    });
    client
        .send(Command::FileIndex(IndexCommand::BrowseKnowledge {
            request_id: 1,
            query: GraphQuery::default(),
        }))
        .unwrap();
    let Event::FileIndex(IndexEvent::KnowledgeBrowsed {
        result: Ok(page), ..
    }) = wait(&events, |e| {
        matches!(
            e,
            Event::FileIndex(IndexEvent::KnowledgeBrowsed { request_id: 1, .. })
        )
    })
    else {
        panic!("saved page")
    };
    assert_eq!(page.relations.len(), 1);
    assert_eq!(page.relations[0].basis.tier, EvidenceTier::ModelInference);
    let id = page.sources[0].id;
    client
        .send(Command::FileIndex(IndexCommand::KnowledgeEvidence {
            request_id: 2,
            id,
        }))
        .unwrap();
    let Event::FileIndex(IndexEvent::KnowledgeEvidence {
        result: Ok(evidence),
        ..
    }) = wait(&events, |e| {
        matches!(
            e,
            Event::FileIndex(IndexEvent::KnowledgeEvidence { request_id: 2, .. })
        )
    })
    else {
        panic!("saved evidence")
    };
    assert_eq!(evidence.passage.text, passage.text);
    client
        .send(Command::FileIndex(IndexCommand::Remove {
            root_id: root.id,
        }))
        .unwrap();
    wait(
        &events,
        |e| matches!(e,Event::FileIndex(IndexEvent::Roots(roots)) if roots.is_empty()),
    );
    client
        .send(Command::FileIndex(IndexCommand::KnowledgeEvidence {
            request_id: 3,
            id,
        }))
        .unwrap();
    let revoked = wait(&events, |e| {
        matches!(
            e,
            Event::FileIndex(IndexEvent::KnowledgeEvidence { request_id: 3, .. })
        )
    });
    assert!(matches!(
        revoked,
        Event::FileIndex(IndexEvent::KnowledgeEvidence { result: Err(_), .. })
    ));
    client
        .send(Command::FileIndex(IndexCommand::BrowseKnowledge {
            request_id: 4,
            query: GraphQuery::default(),
        }))
        .unwrap();
    let Event::FileIndex(IndexEvent::KnowledgeBrowsed {
        result: Ok(page), ..
    }) = wait(&events, |e| {
        matches!(
            e,
            Event::FileIndex(IndexEvent::KnowledgeBrowsed { request_id: 4, .. })
        )
    })
    else {
        panic!()
    };
    assert_eq!(page.relations.len(), 0);
    client.send(Command::Shutdown).unwrap();
    wait(&events, |e| *e == Event::Stopped);
    reader.join().unwrap();
}
