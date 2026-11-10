//! Two Sovra computers form a group and share over the real network stack.
//!
//! Each computer is a full runtime with the production adapters the app wires up:
//! an iroh endpoint (`sovra-mesh`, direct connections only, as with
//! `SOVRA_GROUP_REACH=local`) holding its own device key, and a `FileGroupStore` in
//! its own folder. Only the GUI is missing; every command goes through `Client` as the
//! UI sends it.

use std::sync::Arc;
use std::time::Duration;

use sovra_core::artifacts::ArtifactKind;
use sovra_core::group::{
    DetailStatus, FederatedSettings, GroupCommand, GroupEvent, GroupState, Role, SharedArtifact,
};
use sovra_core::{AgentConfig, Command, Event};
use sovra_runtime::{Client, GroupServices, RuntimeServices};

const WAIT: Duration = Duration::from_secs(60);

struct Computer {
    client: Client,
    events: async_channel::Receiver<Event>,
    dir: TempDir,
}

/// A folder removed when the test ends.
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Computer {
    fn start(name: &str, key: u8) -> Self {
        Self::start_remembering(name, key, &[])
    }

    /// A computer whose own memory already holds `facts` (in-memory `SurrealDB`).
    fn start_remembering(name: &str, key: u8, facts: &[&str]) -> Self {
        let facts: Vec<String> = facts.iter().map(|fact| (*fact).to_owned()).collect();
        let memory: Option<sovra_runtime::MemoryFactory> = (!facts.is_empty()).then(|| {
            Box::new(move || {
                Box::pin(async move {
                    use sovra_core::MemoryStore as _;
                    let store = sovra_store_surreal::SurrealStore::open(
                        sovra_store_surreal::StoreConfig::in_memory(),
                    )
                    .await?;
                    store.set_memory_enabled(true).await?;
                    // Notes keep their own source, as details saved from chat do.
                    for text in facts {
                        store.remember_note(sovra_core::MemoryNote { text }).await?;
                    }
                    Ok(Arc::new(store) as Arc<dyn sovra_core::MemoryStore>)
                }) as sovra_runtime::MemoryStoreFuture
            }) as sovra_runtime::MemoryFactory
        });
        let dir =
            std::env::temp_dir().join(format!("sovra-two-computers-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store_dir = dir.join("groups");
        let client = sovra_runtime::start_with_services(
            "two-computers",
            AgentConfig::default(),
            RuntimeServices {
                memory,
                groups: Some(GroupServices {
                    transport: Box::new(move || {
                        Box::pin(async move {
                            sovra_mesh::Mesh::bind([key; 32], sovra_mesh::Reach::LocalOnly)
                                .await
                                .map(|mesh| {
                                    Arc::new(mesh) as Arc<dyn sovra_core::group::GroupTransport>
                                })
                                .map_err(|error| error.to_string())
                        })
                    }),
                    store: Arc::new(sovra_mesh::FileGroupStore::new(store_dir)),
                }),
                ..RuntimeServices::default()
            },
        )
        .expect("runtime starts");
        let events = client.events();
        Self {
            client,
            events,
            dir: TempDir(dir),
        }
    }

    fn send(&self, command: GroupCommand) {
        self.client
            .send(Command::Group(command))
            .expect("runtime accepts commands");
    }

    async fn wait<T>(&self, what: &str, mut pred: impl FnMut(GroupEvent) -> Option<T>) -> T {
        tokio::time::timeout(WAIT, async {
            loop {
                if let Event::Group(event) = self.events.recv().await.expect("runtime running") {
                    if let GroupEvent::Problem(problem) = &*event {
                        assert!(
                            what.starts_with("problem"),
                            "unexpected problem while waiting for {what}: {}",
                            problem.message()
                        );
                    }
                    if let Some(found) = pred(*event) {
                        return found;
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
    }

    async fn roster(&self, what: &str, pred: impl Fn(&GroupState) -> bool) -> GroupState {
        self.wait(what, |event| match event {
            GroupEvent::Loaded {
                group: Some(state), ..
            }
            | GroupEvent::Changed(state)
                if pred(&state) =>
            {
                Some(state)
            }
            _ => None,
        })
        .await
    }

    async fn artifacts(
        &self,
        what: &str,
        pred: impl Fn(&[SharedArtifact]) -> bool,
    ) -> Vec<SharedArtifact> {
        self.wait(what, |event| match event {
            GroupEvent::Artifacts(artifacts) if pred(&artifacts) => Some(artifacts),
            _ => None,
        })
        .await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_computers_form_a_group_and_share_over_the_real_network() {
    let mum = Computer::start_remembering("mum", 31, &["User's dentist is Dr Patel"]);
    let dad = Computer::start("dad", 32);

    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Mum's laptop".into(),
    });
    mum.roster("the new group", |state| state.name == "Family")
        .await;

    mum.send(GroupCommand::InvitePerson {
        name: "Dad".into(),
        role: Role::Member,
    });
    let link = mum
        .wait("an invite link", |event| match event {
            GroupEvent::InviteReady { link, .. } => Some(link),
            _ => None,
        })
        .await;
    dad.send(GroupCommand::Join {
        link,
        computer_name: "Dad's desktop".into(),
    });
    let joined = dad
        .roster("Dad in the group", |state| state.people.len() == 2)
        .await;
    assert_eq!(joined.name, "Family");

    // A detail from Dad reaches Mum, attributed.
    dad.send(GroupCommand::Share {
        text: "The boiler service is booked for 12 March".into(),
        about: vec!["boiler".into()],
        with: None,
    });
    let details = mum
        .wait("Dad's detail on Mum's laptop", |event| match event {
            GroupEvent::Knowledge(details)
                if details.iter().any(|d| d.text.contains("12 March")) =>
            {
                Some(details)
            }
            _ => None,
        })
        .await;
    let boiler = details
        .iter()
        .find(|d| d.text.contains("12 March"))
        .expect("shared");
    assert_eq!(boiler.shared_by, "Dad");
    assert_eq!(boiler.computer, "Dad's desktop");

    // Mum shares her list; Dad receives it, edits his copy and shares it back.
    mum.send(GroupCommand::ShareArtifact {
        title: "Shopping list".into(),
        kind: ArtifactKind::Checklist,
        language: String::new(),
        content: "- milk\n- eggs".into(),
        new_version_of: None,
        local_id: "art-list".into(),
        with: None,
    });
    let received = dad
        .artifacts("Mum's list on Dad's desktop", |artifacts| {
            artifacts.iter().any(|a| a.title == "Shopping list")
        })
        .await;
    let list = received
        .iter()
        .find(|a| a.title == "Shopping list")
        .expect("shared")
        .clone();
    assert_eq!(list.shared_by, "Mum");
    dad.send(GroupCommand::ShareArtifact {
        title: "Shopping list".into(),
        kind: ArtifactKind::Checklist,
        language: String::new(),
        content: "- milk\n- eggs\n- bread".into(),
        new_version_of: Some(list.id),
        local_id: "art-dads-copy".into(),
        with: None,
    });
    let back = mum
        .artifacts("Dad's new version on Mum's laptop", |artifacts| {
            artifacts.iter().any(|a| a.content.contains("bread"))
        })
        .await;
    let newest = back
        .iter()
        .find(|a| a.content.contains("bread"))
        .expect("new version");
    assert_eq!(newest.shared_by, "Dad");
    assert_eq!(newest.replaces, vec![list.id]);
    assert!(matches!(
        back.iter().find(|a| a.id == list.id).expect("kept").status,
        DetailStatus::Replaced { .. }
    ));

    // What is saved on disk is sealed: none of the shared words appear in either record.
    for computer in [&mum, &dad] {
        let saved = std::fs::read_to_string(computer.dir.0.join("groups").join("group.json"))
            .expect("the group record is saved");
        for words in ["12 March", "bread", "Shopping list"] {
            assert!(!saved.contains(words), "{words:?} is readable on disk");
        }
    }

    // Federated recall: nothing comes back until Mum lets the group ask her memory.
    dad.send(GroupCommand::AskGroup {
        question: "who is the dentist".into(),
    });
    let none = dad
        .wait("answers while Mum isn't answering", |event| match event {
            GroupEvent::Answers { answers, .. } => Some(answers),
            _ => None,
        })
        .await;
    assert_eq!(none, Vec::new(), "answering starts off");
    mum.send(GroupCommand::SetFederated(FederatedSettings {
        answer: true,
        ask: false,
    }));
    mum.wait("Mum's answering switched on", |event| match event {
        GroupEvent::Federated { settings, .. } if settings.answer => Some(()),
        _ => None,
    })
    .await;
    // Dad's earlier unanswered ask put Mum on a short cooldown for automatic recall;
    // an explicit "Ask my group" always asks.
    dad.send(GroupCommand::AskGroup {
        question: "who is the dentist".into(),
    });
    let answers = dad
        .wait("Mum's answer", |event| match event {
            GroupEvent::Answers { answers, .. } if !answers.is_empty() => Some(answers),
            _ => None,
        })
        .await;
    assert_eq!(answers[0].from, "Mum");
    assert_eq!(answers[0].computer, "Mum's laptop");
    assert!(
        answers[0].statement.contains("Mum's dentist is Dr Patel"),
        "reworded for the group: {}",
        answers[0].statement
    );
    let log = mum
        .wait("Mum's log of the question", |event| match event {
            GroupEvent::Federated { recent, .. } if !recent.is_empty() => Some(recent),
            _ => None,
        })
        .await;
    assert_eq!(log[0].asked_by, "Dad");
    assert_eq!(log[0].question, "who is the dentist");
    assert!(log[0].answered >= 1);

    // Mum removes Dad; his computer leaves the group.
    let dad_person = joined
        .people
        .values()
        .find(|member| member.name == "Dad")
        .expect("Dad")
        .person;
    mum.send(GroupCommand::RemovePerson { person: dad_person });
    dad.wait("Dad's computer leaving", |event| {
        matches!(event, GroupEvent::LeftGroup).then_some(())
    })
    .await;
}
