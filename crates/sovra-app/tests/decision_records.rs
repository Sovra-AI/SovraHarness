//! Real Client, local accounts and disk persistence for the decision lifecycle.

use sovra_core::decisions::{
    DecisionCommand, DecisionDraft, DecisionEvent, DecisionEvidence, DecisionRecord, DecisionStop,
};
use sovra_core::{AgentConfig, AuthCommand, AuthEvent, Command, Event, MemoryStore};
use sovra_runtime::{Client, RuntimeServices};
use sovra_store_surreal::{StoreConfig, SurrealStore};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Driver {
    client: Option<Client>,
    events: mpsc::Receiver<Event>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Driver {
    fn start(path: &std::path::Path) -> Self {
        let config = StoreConfig::on_disk(path.to_owned());
        let client = sovra_runtime::start_with_services(
            "decisions-test",
            AgentConfig::default(),
            RuntimeServices {
                memory: Some(Box::new(move || {
                    Box::pin(async move {
                        Ok(Arc::new(SurrealStore::open(config).await?) as Arc<dyn MemoryStore>)
                    })
                })),
                ..RuntimeServices::default()
            },
        )
        .expect("decision test operation succeeds");
        let incoming = client.events();
        let (sender, events) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            while let Ok(event) = incoming.recv_blocking() {
                if sender.send(event).is_err() {
                    break;
                }
            }
        });
        let driver = Self {
            client: Some(client),
            events,
            reader: Some(reader),
        };
        driver.wait(|event| matches!(event, Event::Ready { .. }));
        driver
    }
    fn send(&self, command: Command) {
        self.client
            .as_ref()
            .expect("decision test operation succeeds")
            .send(command)
            .expect("decision test operation succeeds");
    }
    fn wait(&self, pick: impl Fn(&Event) -> bool) -> Event {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let event = self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("bounded runtime event wait");
            if pick(&event) {
                return event;
            }
        }
    }
    fn decision(&self, command: DecisionCommand) -> DecisionEvent {
        self.send(Command::Decisions(command));
        match self.wait(|event| matches!(event, Event::Decisions(_))) {
            Event::Decisions(event) => event,
            _ => unreachable!(),
        }
    }
    fn create_account(&self, name: &str) -> String {
        self.send(Command::Auth(AuthCommand::CreateAccount {
            full_name: name.into(),
            email: String::new(),
            password: String::new(),
        }));
        match self.wait(|event| matches!(event, Event::Auth(AuthEvent::SignedIn { .. }))) {
            Event::Auth(AuthEvent::SignedIn { session, .. }) => session.account_id,
            _ => unreachable!(),
        }
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            client
                .shutdown_blocking()
                .expect("decision test operation succeeds");
        }
        if let Some(reader) = self.reader.take() {
            reader.join().expect("decision test operation succeeds");
        }
    }
}

fn saved(event: DecisionEvent) -> DecisionRecord {
    match event {
        DecisionEvent::Saved(record) => *record,
        other => panic!("expected committed save: {other:?}"),
    }
}

#[test]
fn decisions_require_a_second_account_survive_restart_and_reject_stale_acceptance() {
    let path = std::env::temp_dir().join(format!(
        "sovra-decisions-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("decision test operation succeeds")
            .as_nanos()
    ));
    let driver = Driver::start(&path);
    assert!(matches!(
        driver.decision(DecisionCommand::Load { offset: 0 }),
        DecisionEvent::Error(_)
    ));
    let writer = driver.create_account("Decision test writer");
    let mut draft = DecisionDraft {
        question: "Which supplier should we choose?".into(),
        decision: "Choose Acme.".into(),
        criteria: "Delivery within two days.".into(),
        review_on: "2099-01-01".into(),
        ends_when: "A delivery takes more than two days.".into(),
        evidence: vec![DecisionEvidence {
            reference: "Quote page 2".into(),
            passage: "Delivery within two days.".into(),
        }],
        hard_stops: vec![DecisionStop {
            condition: "Contract reviewed".into(),
            cleared: false,
        }],
        ..DecisionDraft::default()
    };
    let first = saved(driver.decision(DecisionCommand::Save {
        id: None,
        expected_revision: 0,
        draft: Box::new(draft.clone()),
    }));
    assert_eq!(
        first
            .current()
            .expect("decision test operation succeeds")
            .drafted_by
            .account_id,
        writer
    );
    assert!(matches!(
        driver.decision(DecisionCommand::Accept {
            id: first.id.clone(),
            expected_revision: 1
        }),
        DecisionEvent::Error(_)
    ));
    let reviewer = driver.create_account("Decision test reviewer");
    assert!(
        matches!(
            driver.decision(DecisionCommand::Accept {
                id: first.id.clone(),
                expected_revision: 1
            }),
            DecisionEvent::Error(_)
        ),
        "unresolved hard stop blocks a different account too"
    );
    driver.send(Command::Auth(AuthCommand::ContinueLocalProfile {
        account_id: writer.clone(),
    }));
    driver.wait(|event| matches!(event, Event::Auth(AuthEvent::SignedIn { session, .. }) if session.account_id == writer));
    draft.hard_stops[0].cleared = true;
    let revised = saved(driver.decision(DecisionCommand::Save {
        id: Some(first.id.clone()),
        expected_revision: 1,
        draft: Box::new(draft),
    }));
    driver.send(Command::Auth(AuthCommand::ContinueLocalProfile {
        account_id: reviewer.clone(),
    }));
    // Ordered with the identity change, even when queued before its UI event arrives.
    let accepted = saved(driver.decision(DecisionCommand::Accept {
        id: first.id.clone(),
        expected_revision: revised.revision,
    }));
    assert_eq!(
        accepted
            .current()
            .expect("decision test operation succeeds")
            .accepted
            .as_ref()
            .expect("decision test operation succeeds")
            .by
            .account_id,
        reviewer
    );
    assert!(matches!(
        driver.decision(DecisionCommand::End {
            id: first.id.clone(),
            expected_revision: revised.revision,
            reason: "Late delivery".into()
        }),
        DecisionEvent::Error(_)
    ));
    driver.send(Command::Auth(AuthCommand::SignOut));
    assert!(matches!(
        driver.decision(DecisionCommand::Accept {
            id: first.id.clone(),
            expected_revision: accepted.revision
        }),
        DecisionEvent::Error(_)
    ));
    drop(driver);

    let driver = Driver::start(&path);
    driver.send(Command::Auth(AuthCommand::Load));
    driver.wait(|event| matches!(event, Event::Auth(AuthEvent::Loaded { .. })));
    driver.send(Command::Auth(AuthCommand::ContinueLocalProfile {
        account_id: reviewer,
    }));
    match driver.decision(DecisionCommand::Open {
        id: first.id.clone(),
    }) {
        DecisionEvent::Opened(record) => assert_eq!(*record, accepted),
        event => panic!("saved history must reopen: {event:?}"),
    }
    let ended = saved(driver.decision(DecisionCommand::End {
        id: first.id.clone(),
        expected_revision: accepted.revision,
        reason: "Delivery took a week.".into(),
    }));
    assert_eq!(
        ended
            .current()
            .expect("decision test operation succeeds")
            .accepted,
        accepted
            .current()
            .expect("decision test operation succeeds")
            .accepted
    );
    assert!(
        ended
            .current()
            .expect("decision test operation succeeds")
            .ended
            .is_some()
    );
    assert_eq!(ended.versions[0], first.versions[0]);
    drop(driver);
    std::fs::remove_dir_all(path).expect("decision test operation succeeds");
}
