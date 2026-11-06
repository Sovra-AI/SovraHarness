//! Three computers form a group and share knowledge through full runtimes.
//!
//! The network is an in-process loopback that implements the same `GroupTransport`
//! port as the iroh adapter, so every rule, signature check and runtime path runs
//! for real while delivery stays deterministic (and computers can be "switched off").

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest as _, Sha256};
use sovra_core::group::{
    DetailStatus, DeviceId, GroupCommand, GroupEvent, GroupRecord, GroupState, GroupStore,
    GroupStoreError, GroupTransport, PeerAddress, PeerHandler, PeerRequest, PeerResponse, Role,
    SharedDetail, Signature, SignatureVerifier, Spawn, TransportError,
};
use sovra_core::{AgentConfig, Command, Event};
use sovra_runtime::{Client, GroupServices, RuntimeServices};

const WAIT: Duration = Duration::from_secs(10);

fn fake_signature(device: &DeviceId, message: &[u8]) -> Signature {
    let first: [u8; 32] =
        Sha256::digest([b"sig".as_slice(), device.as_bytes(), message].concat()).into();
    let second: [u8; 32] = Sha256::digest(first).into();
    let mut bytes = [0u8; 64];
    bytes[..32].copy_from_slice(&first);
    bytes[32..].copy_from_slice(&second);
    Signature(bytes)
}

struct FakeVerifier;

impl SignatureVerifier for FakeVerifier {
    fn verify(&self, signer: &DeviceId, message: &[u8], signature: &Signature) -> bool {
        fake_signature(signer, message) == *signature
    }
}

/// Every computer's handler, plus which computers are switched off.
#[derive(Default)]
struct Network {
    handlers: Mutex<BTreeMap<DeviceId, Arc<dyn PeerHandler>>>,
    off: Mutex<BTreeSet<DeviceId>>,
}

struct LoopTransport {
    me: DeviceId,
    net: Arc<Network>,
    closed: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl GroupTransport for LoopTransport {
    fn this_device(&self) -> DeviceId {
        self.me
    }

    fn sign(&self, message: &[u8]) -> Signature {
        fake_signature(&self.me, message)
    }

    fn verifier(&self) -> Arc<dyn SignatureVerifier + Send + Sync> {
        Arc::new(FakeVerifier)
    }

    fn address(&self) -> PeerAddress {
        PeerAddress {
            device: self.me,
            relay: None,
            addrs: vec!["loopback".into()],
        }
    }

    async fn serve(&self, handler: Arc<dyn PeerHandler>, _spawn: Spawn) {
        self.net
            .handlers
            .lock()
            .expect("not poisoned")
            .insert(self.me, handler);
        self.closed.notified().await;
    }

    async fn request(
        &self,
        to: &PeerAddress,
        request: PeerRequest,
    ) -> Result<PeerResponse, TransportError> {
        let off = self.net.off.lock().expect("not poisoned").clone();
        if off.contains(&self.me) || off.contains(&to.device) {
            return Err(TransportError::Unreachable("switched off".into()));
        }
        let handler = self
            .net
            .handlers
            .lock()
            .expect("not poisoned")
            .get(&to.device)
            .cloned()
            .ok_or_else(|| TransportError::Unreachable("not listening".into()))?;
        // Round-trip through JSON, as the real wire does.
        let request: PeerRequest =
            serde_json::from_slice(&serde_json::to_vec(&request).expect("encodes"))
                .expect("decodes");
        let response = handler.handle(self.me, request).await;
        Ok(
            serde_json::from_slice(&serde_json::to_vec(&response).expect("encodes"))
                .expect("decodes"),
        )
    }

    async fn close(&self) {
        self.closed.notify_waiters();
    }
}

#[derive(Default)]
struct MemoryStore(Mutex<GroupRecord>);

#[async_trait::async_trait]
impl GroupStore for MemoryStore {
    async fn load(&self) -> Result<GroupRecord, GroupStoreError> {
        Ok(self.0.lock().expect("not poisoned").clone())
    }

    async fn save(&self, record: &GroupRecord) -> Result<(), GroupStoreError> {
        record.clone_into(&mut self.0.lock().expect("not poisoned"));
        Ok(())
    }
}

/// One computer: a full runtime with its events.
struct Computer {
    device: DeviceId,
    client: Client,
    events: async_channel::Receiver<Event>,
}

impl Computer {
    fn start(net: &Arc<Network>, n: u8) -> Self {
        let device = DeviceId([n; 32]);
        let transport: Arc<dyn GroupTransport> = Arc::new(LoopTransport {
            me: device,
            net: net.clone(),
            closed: tokio::sync::Notify::new(),
        });
        let client = sovra_runtime::start_with_services(
            "group-test",
            AgentConfig::default(),
            RuntimeServices {
                groups: Some(GroupServices {
                    transport: Box::new(move || Box::pin(async move { Ok(transport) })),
                    store: Arc::new(MemoryStore::default()),
                }),
                ..RuntimeServices::default()
            },
        )
        .expect("runtime starts");
        let events = client.events();
        Self {
            device,
            client,
            events,
        }
    }

    fn send(&self, command: GroupCommand) {
        self.client
            .send(Command::Group(command))
            .expect("runtime accepts commands");
    }

    /// Waits for a group event matching `pred` and returns what it extracted.
    async fn wait<T>(&self, mut pred: impl FnMut(GroupEvent) -> Option<T>) -> T {
        tokio::time::timeout(WAIT, async {
            loop {
                if let Event::Group(event) = self.events.recv().await.expect("runtime running") {
                    let event = *event;
                    if let Some(found) = pred(event) {
                        return found;
                    }
                }
            }
        })
        .await
        .expect("expected group event in time")
    }

    async fn roster(&self, pred: impl Fn(&GroupState) -> bool) -> GroupState {
        self.wait(|event| match event {
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

    async fn knowledge(&self, pred: impl Fn(&[SharedDetail]) -> bool) -> Vec<SharedDetail> {
        self.wait(|event| match event {
            GroupEvent::Knowledge(details) if pred(&details) => Some(details),
            _ => None,
        })
        .await
    }

    async fn link(&self) -> String {
        self.wait(|event| match event {
            GroupEvent::InviteReady { link, .. } => Some(link),
            _ => None,
        })
        .await
    }
}

fn has(details: &[SharedDetail], text: &str) -> bool {
    details
        .iter()
        .any(|detail| detail.text.contains(text) && detail.usable())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_family_forms_a_group_and_shares_knowledge_across_computers() {
    let net = Arc::new(Network::default());
    let mum = Computer::start(&net, 1);
    let dad = Computer::start(&net, 2);
    let mum_imac = Computer::start(&net, 3);

    // Mum founds the group.
    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Mum's laptop".into(),
    });
    let founded = mum.roster(|state| state.name == "Family").await;
    assert_eq!(founded.role_of(&mum.device), Some(Role::Admin));

    // Dad joins with a person invite.
    mum.send(GroupCommand::InvitePerson {
        name: "Dad".into(),
        role: Role::Member,
    });
    let link = mum.link().await;
    dad.send(GroupCommand::Join {
        link,
        computer_name: "Dad's desktop".into(),
    });
    let joined = dad.roster(|state| state.people.len() == 2).await;
    assert_eq!(joined.role_of(&dad.device), Some(Role::Member));
    mum.roster(|state| state.admits(&dad.device)).await;

    // Mum adds her own second computer; it joins as Mum, not as a new person.
    mum.send(GroupCommand::InviteComputer);
    let link = mum.link().await;
    mum_imac.send(GroupCommand::Join {
        link,
        computer_name: "Mum's iMac".into(),
    });
    let roster = mum_imac
        .roster(|state| state.admits(&mum_imac.device))
        .await;
    assert_eq!(roster.people.len(), 2);
    assert_eq!(
        roster.owner_of(&mum_imac.device).map(|m| m.name.clone()),
        Some("Mum".into())
    );
    dad.roster(|state| state.admits(&mum_imac.device)).await;

    // Dad shares a detail; both of Mum's computers receive it, attributed to Dad.
    dad.send(GroupCommand::Share {
        text: "The boiler service is booked for 12 March".into(),
        about: vec!["boiler".into()],
    });
    let mut boiler = None;
    for computer in [&mum, &mum_imac] {
        let details = computer.knowledge(|details| has(details, "12 March")).await;
        let detail = details
            .iter()
            .find(|detail| detail.text.contains("12 March"))
            .expect("shared");
        assert_eq!(detail.shared_by, "Dad");
        assert_eq!(detail.computer, "Dad's desktop");
        boiler = Some(detail.id);
    }
    let boiler = boiler.expect("seen on Mum's computers");

    // Mum's iMac is switched off; Dad shares again; it catches up when opened.
    net.off
        .lock()
        .expect("not poisoned")
        .insert(mum_imac.device);
    dad.send(GroupCommand::Share {
        text: "Gran arrives on Friday by train".into(),
        about: Vec::new(),
    });
    mum.knowledge(|details| has(details, "Gran arrives")).await;
    net.off
        .lock()
        .expect("not poisoned")
        .remove(&mum_imac.device);
    mum_imac.send(GroupCommand::Load);
    mum_imac
        .knowledge(|details| has(details, "Gran arrives"))
        .await;

    // Mum corrects Dad's detail; everyone sees the correction replace it.
    mum.send(GroupCommand::Correct {
        details: vec![boiler],
        text: "The boiler service moved to 19 March".into(),
    });
    let details = dad.knowledge(|details| has(details, "19 March")).await;
    let old = details
        .iter()
        .find(|detail| detail.id == boiler)
        .expect("kept in history");
    assert!(matches!(old.status, DetailStatus::Replaced { .. }));
    assert!(!old.usable());

    // Mum removes Dad: his computer leaves the group and can no longer share.
    let dad_person = joined.owner_of(&dad.device).expect("member").person;
    mum.send(GroupCommand::RemovePerson { person: dad_person });
    dad.wait(|event| matches!(event, GroupEvent::LeftGroup).then_some(()))
        .await;
    let after = mum.roster(|state| !state.admits(&dad.device)).await;
    assert_eq!(after.people.len(), 1);
    // His earlier details are kept but no longer used.
    let details = mum
        .knowledge(|details| details.iter().any(|d| d.from_former_member))
        .await;
    assert!(!has(&details, "Gran arrives"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_used_or_foreign_invite_is_refused_and_members_cannot_invite_people() {
    let net = Arc::new(Network::default());
    let mum = Computer::start(&net, 11);
    let dad = Computer::start(&net, 12);
    let stranger = Computer::start(&net, 13);

    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Laptop".into(),
    });
    let family = mum.roster(|state| state.name == "Family").await;
    mum.send(GroupCommand::InvitePerson {
        name: "Dad".into(),
        role: Role::Member,
    });
    let link = mum.link().await;
    dad.send(GroupCommand::Join {
        link: link.clone(),
        computer_name: "Desktop".into(),
    });
    dad.roster(|state| state.people.len() == 2).await;

    // The same link a second time: refused.
    stranger.send(GroupCommand::Join {
        link,
        computer_name: "Unknown".into(),
    });
    stranger
        .wait(|event| {
            matches!(
                event,
                GroupEvent::Problem(
                    sovra_core::group::GroupProblem::BadInvite
                        | sovra_core::group::GroupProblem::InviteUsed
                )
            )
            .then_some(())
        })
        .await;

    // A member asking to invite a person is told only admins can.
    dad.send(GroupCommand::InvitePerson {
        name: "Friend".into(),
        role: Role::Member,
    });
    dad.wait(|event| {
        matches!(
            event,
            GroupEvent::Problem(sovra_core::group::GroupProblem::NotAllowed)
        )
        .then_some(())
    })
    .await;

    // A stranger pushing knowledge is refused at the door.
    let handler = net
        .handlers
        .lock()
        .expect("not poisoned")
        .get(&mum.device)
        .cloned()
        .expect("mum listening");
    let response = handler
        .handle(
            stranger.device,
            PeerRequest::Push {
                group: family.group,
                membership: Vec::new(),
                knowledge: Vec::new(),
            },
        )
        .await;
    assert_eq!(
        response,
        PeerResponse::Refused {
            reason: sovra_core::group::Refusal::NotAMember
        }
    );
}
