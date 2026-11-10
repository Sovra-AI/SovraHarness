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
    SharedArtifact, SharedDetail, Signature, SignatureVerifier, Spawn, TransportError,
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
    /// Computers showing a same-room code, with its hint.
    pairing: Mutex<BTreeMap<DeviceId, u8>>,
}

struct LoopTransport {
    me: DeviceId,
    net: Arc<Network>,
    closed: tokio::sync::Notify,
    cipher: Arc<FakeCipher>,
}

/// Deterministic stand-in for lane encryption: only the named computer opens a seal,
/// and content opens only with its key and associated data. Not cryptography.
struct FakeCipher {
    me: DeviceId,
    next: std::sync::atomic::AtomicU64,
}

/// A repeating pad from the key and place: cheap, and wrong with any other key.
fn stream(key: &[u8], aad: &[u8], len: usize) -> Vec<u8> {
    let pad = Sha256::digest([key, aad].concat());
    pad.iter().copied().cycle().take(len).collect()
}

impl sovra_core::group::GroupCipher for FakeCipher {
    fn seal_for(&self, device: &DeviceId, plaintext: &[u8]) -> Option<Vec<u8>> {
        let mut out = device.as_bytes().to_vec();
        out.extend(
            plaintext
                .iter()
                .zip(stream(device.as_bytes(), b"seal", plaintext.len()))
                .map(|(a, b)| a ^ b),
        );
        Some(out)
    }
    fn open_sealed(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        let (to, body) = sealed.split_at_checked(32)?;
        (to == self.me.as_bytes()).then(|| {
            body.iter()
                .zip(stream(to, b"seal", body.len()))
                .map(|(a, b)| a ^ b)
                .collect()
        })
    }
    fn encrypt(&self, key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Option<Vec<u8>> {
        let mut out: Vec<u8> = Sha256::digest([key.as_slice(), aad, plaintext].concat()).to_vec();
        out.extend(
            plaintext
                .iter()
                .zip(stream(key, aad, plaintext.len()))
                .map(|(a, b)| a ^ b),
        );
        Some(out)
    }
    fn decrypt(&self, key: &[u8; 32], aad: &[u8], ciphertext: &[u8]) -> Option<Vec<u8>> {
        let (tag, body) = ciphertext.split_at_checked(32)?;
        let plaintext: Vec<u8> = body
            .iter()
            .zip(stream(key, aad, body.len()))
            .map(|(a, b)| a ^ b)
            .collect();
        (Sha256::digest([key.as_slice(), aad, &plaintext].concat()).as_slice() == tag)
            .then_some(plaintext)
    }
    fn random_key(&self) -> Option<[u8; 32]> {
        let n = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Some(Sha256::digest([self.me.as_bytes().as_slice(), &n.to_le_bytes()].concat()).into())
    }
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

    fn cipher(&self) -> Arc<dyn sovra_core::group::GroupCipher> {
        self.cipher.clone()
    }

    fn set_pairing_open(&self, hint: Option<u8>) {
        let mut pairing = self.net.pairing.lock().expect("not poisoned");
        match hint {
            Some(hint) => {
                pairing.insert(self.me, hint);
            }
            None => {
                pairing.remove(&self.me);
            }
        }
    }

    async fn nearby_pairing(&self, hint: u8, _listen_ms: u64) -> Vec<PeerAddress> {
        let pairing = self.net.pairing.lock().expect("not poisoned").clone();
        pairing
            .into_iter()
            .filter(|(device, shown)| *device != self.me && *shown == hint)
            .map(|(device, _)| device)
            .map(|device| PeerAddress {
                device,
                relay: None,
                addrs: vec!["loopback".into()],
            })
            .collect()
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
    store: Arc<MemoryStore>,
}

impl Computer {
    fn start(net: &Arc<Network>, n: u8) -> Self {
        Self::start_with(net, n, None)
    }

    /// A computer whose AI (for helping the group) is `helper`.
    fn start_with(
        net: &Arc<Network>,
        n: u8,
        helper: Option<Arc<dyn sovra_core::group::WorkModel>>,
    ) -> Self {
        let device = DeviceId([n; 32]);
        let transport: Arc<dyn GroupTransport> = Arc::new(LoopTransport {
            me: device,
            net: net.clone(),
            closed: tokio::sync::Notify::new(),
            cipher: Arc::new(FakeCipher {
                me: device,
                next: std::sync::atomic::AtomicU64::new(0),
            }),
        });
        let store = Arc::new(MemoryStore::default());
        let client = sovra_runtime::start_with_services(
            "group-test",
            AgentConfig::default(),
            RuntimeServices {
                groups: Some(GroupServices {
                    transport: Box::new(move || Box::pin(async move { Ok(transport) })),
                    store: store.clone(),
                    qr: None,
                    helper_model: helper,
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
            store,
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

    async fn artifacts(&self, pred: impl Fn(&[SharedArtifact]) -> bool) -> Vec<SharedArtifact> {
        self.wait(|event| match event {
            GroupEvent::Artifacts(artifacts) if pred(&artifacts) => Some(artifacts),
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
        with: None,
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
        with: None,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_artifacts_reach_a_new_member_in_rounds_and_edits_become_new_versions() {
    let net = Arc::new(Network::default());
    let mum = Computer::start(&net, 21);
    let dad = Computer::start(&net, 22);

    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Laptop".into(),
    });
    mum.roster(|state| state.name == "Family").await;

    // More than one message can carry: 20 artifacts of 400 KB each.
    let big = "x".repeat(400 * 1024);
    for n in 0..20 {
        mum.send(GroupCommand::ShareArtifact {
            title: format!("Recipe {n}"),
            kind: sovra_core::artifacts::ArtifactKind::Document,
            language: String::new(),
            content: format!("{n}{big}"),
            new_version_of: None,
            local_id: String::new(),
            with: None,
        });
        mum.artifacts(|artifacts| artifacts.len() == n + 1).await;
    }
    mum.send(GroupCommand::ShareArtifact {
        title: "Shopping list".into(),
        kind: sovra_core::artifacts::ArtifactKind::Checklist,
        language: String::new(),
        content: "- milk".into(),
        new_version_of: None,
        local_id: String::new(),
        with: None,
    });
    let list = mum
        .artifacts(|artifacts| artifacts.len() == 21)
        .await
        .into_iter()
        .find(|artifact| artifact.title == "Shopping list")
        .expect("shared");
    assert_eq!(list.shared_by, "Mum");

    mum.send(GroupCommand::InvitePerson {
        name: "Dad".into(),
        role: Role::Member,
    });
    let link = mum.link().await;
    dad.send(GroupCommand::Join {
        link,
        computer_name: "Desktop".into(),
    });
    let received = dad.artifacts(|artifacts| artifacts.len() == 21).await;
    assert!(received.iter().all(SharedArtifact::is_latest));

    // Dad edits his copy of the list and shares it back as its newest version.
    dad.send(GroupCommand::ShareArtifact {
        title: "Shopping list".into(),
        kind: sovra_core::artifacts::ArtifactKind::Checklist,
        language: String::new(),
        content: "- milk\n- bread".into(),
        new_version_of: Some(list.id),
        local_id: String::new(),
        with: None,
    });
    let seen = mum
        .artifacts(|artifacts| artifacts.iter().any(|a| a.content.contains("bread")))
        .await;
    let newest = seen
        .iter()
        .find(|a| a.content.contains("bread"))
        .expect("new version");
    assert_eq!(newest.shared_by, "Dad");
    assert_eq!(newest.replaces, vec![list.id]);
    let old = seen.iter().find(|a| a.id == list.id).expect("kept");
    assert!(matches!(old.status, DetailStatus::Replaced { .. }));

    // Too big to share: refused before anything is signed.
    mum.send(GroupCommand::ShareArtifact {
        title: "Huge".into(),
        kind: sovra_core::artifacts::ArtifactKind::Data,
        language: String::new(),
        content: "x".repeat(sovra_core::group::MAX_SHARED_ARTIFACT_BYTES + 1),
        new_version_of: None,
        local_id: String::new(),
        with: None,
    });
    mum.wait(|event| matches!(event, GroupEvent::Problem(_)).then_some(()))
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_content_is_sealed_on_disk_and_removing_someone_rotates_the_key() {
    let net = Arc::new(Network::default());
    let mum = Computer::start(&net, 31);
    let dad = Computer::start(&net, 32);
    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Laptop".into(),
    });
    mum.roster(|state| state.name == "Family").await;
    mum.send(GroupCommand::InvitePerson {
        name: "Dad".into(),
        role: Role::Member,
    });
    let link = mum.link().await;
    dad.send(GroupCommand::Join {
        link,
        computer_name: "Desktop".into(),
    });
    let joined = dad.roster(|state| state.people.len() == 2).await;

    mum.send(GroupCommand::Share {
        text: "The spare key is under the blue pot".into(),
        about: Vec::new(),
        with: None,
    });
    dad.knowledge(|details| has(details, "blue pot")).await;
    for computer in [&mum, &dad] {
        let saved = serde_json::to_string(&*computer.store.0.lock().expect("not poisoned"))
            .expect("encodes");
        assert!(!saved.contains("blue pot"), "only sealed words are saved");
        assert!(saved.contains("key_grant") && saved.contains("sealed"));
    }
    let keys_before = lane_keys(&mum.store);
    assert_eq!(keys_before.len(), 1);
    assert_eq!(keys_before[0].1, BTreeSet::from([mum.device, dad.device]));

    let dad_person = joined.owner_of(&dad.device).expect("member").person;
    mum.send(GroupCommand::RemovePerson { person: dad_person });
    mum.roster(|state| !state.admits(&dad.device)).await;
    mum.send(GroupCommand::Share {
        text: "New wifi password is on the fridge".into(),
        about: Vec::new(),
        with: None,
    });
    mum.knowledge(|details| has(details, "fridge")).await;
    let keys_after = lane_keys(&mum.store);
    assert_eq!(keys_after.len(), 2, "a fresh key after the removal");
    let fresh = keys_after
        .iter()
        .find(|(id, _)| *id != keys_before[0].0)
        .expect("new key");
    assert_eq!(
        fresh.1,
        BTreeSet::from([mum.device]),
        "Dad never gets the new key"
    );
}

/// Each lane key in a saved record and the computers it was granted to.
fn lane_keys(store: &MemoryStore) -> Vec<(String, BTreeSet<DeviceId>)> {
    let saved = serde_json::to_value(&*store.0.lock().expect("not poisoned")).expect("encodes");
    let mut keys: BTreeMap<String, BTreeSet<DeviceId>> = BTreeMap::new();
    for op in saved["knowledge"].as_array().expect("ops") {
        let body = &op["body"];
        if body["op"] == "key_grant" {
            let entry = keys.entry(body["key"].to_string()).or_default();
            for grant in body["grants"].as_array().expect("grants") {
                entry.insert(serde_json::from_value(grant[0].clone()).expect("device"));
            }
        }
    }
    keys.into_iter().collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn something_shared_with_just_one_person_never_reaches_anyone_else() {
    let net = Arc::new(Network::default());
    let mum = Computer::start(&net, 41);
    let dad = Computer::start(&net, 42);
    let sam = Computer::start(&net, 43);
    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Laptop".into(),
    });
    mum.roster(|state| state.name == "Family").await;
    for (computer, name) in [(&dad, "Dad"), (&sam, "Sam")] {
        mum.send(GroupCommand::InvitePerson {
            name: name.into(),
            role: Role::Member,
        });
        let link = mum.link().await;
        computer.send(GroupCommand::Join {
            link,
            computer_name: format!("{name}'s computer"),
        });
        computer
            .roster(|state| state.admits(&computer.device))
            .await;
    }
    let roster = mum.roster(|state| state.people.len() == 3).await;
    let dad_person = roster.owner_of(&dad.device).expect("Dad").person;

    mum.send(GroupCommand::Share {
        text: "Sam's surprise party is on Friday".into(),
        about: Vec::new(),
        with: Some(dad_person),
    });
    let details = dad
        .knowledge(|details| has(details, "surprise party"))
        .await;
    let party = details
        .iter()
        .find(|detail| detail.text.contains("surprise party"))
        .expect("shared with Dad");
    assert!(matches!(party.space, sovra_core::group::Space::Pair { .. }));

    // A group-wide share after it, so Sam certainly syncs past the private one.
    mum.send(GroupCommand::Share {
        text: "Bins go out on Tuesday".into(),
        about: Vec::new(),
        with: None,
    });
    sam.knowledge(|details| has(details, "Bins go out")).await;
    sam.send(GroupCommand::Load);
    let seen = sam.knowledge(|details| has(details, "Bins go out")).await;
    assert!(!has(&seen, "surprise party"));
    let pair_lane = party.space.lane(&roster.group);
    let sams_record = sam.store.0.lock().expect("not poisoned").clone();
    assert!(
        sams_record
            .knowledge
            .iter()
            .all(|op| op.unsigned().lane != pair_lane),
        "nothing from Mum and Dad's space is stored on Sam's computer"
    );
}

impl Computer {
    async fn short_code(&self) -> String {
        self.wait(|event| match event {
            GroupEvent::ShortCodeReady { code, .. } => Some(code),
            _ => None,
        })
        .await
    }

    async fn pictures(&self) -> sovra_core::group::ShortAuthString {
        self.wait(|event| match event {
            GroupEvent::Pairing(sovra_core::group::PairingStep::CodeReady(code)) => Some(code),
            _ => None,
        })
        .await
    }
}

async fn founded(net: &Arc<Network>, n: u8) -> Computer {
    let mum = Computer::start(net, n);
    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Laptop".into(),
    });
    mum.roster(|state| state.name == "Family").await;
    mum
}

fn dad_invite() -> GroupCommand {
    GroupCommand::ShowShortCode {
        kind: sovra_core::group::InviteKind::Person {
            name: "Dad".into(),
            role: Role::Member,
        },
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_same_room_code_joins_after_both_people_see_the_same_pictures() {
    let net = Arc::new(Network::default());
    let mum = founded(&net, 61).await;
    let dad = Computer::start(&net, 62);

    mum.send(dad_invite());
    let code = mum.short_code().await;
    assert_eq!(code.len(), 7, "shown as 123 456");
    dad.send(GroupCommand::JoinWithCode {
        code,
        computer_name: "Desktop".into(),
    });
    let on_dads = dad.pictures().await;
    let on_mums = mum.pictures().await;
    assert_eq!(on_dads, on_mums, "both screens show the same pictures");

    dad.send(GroupCommand::ConfirmCode { matches: true });
    mum.send(GroupCommand::ConfirmCode { matches: true });
    let joined = dad.roster(|state| state.admits(&dad.device)).await;
    assert_eq!(
        joined
            .owner_of(&dad.device)
            .map(|member| member.name.clone()),
        Some("Dad".into())
    );
    mum.roster(|state| state.admits(&dad.device)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_code_burns_it_and_a_no_on_the_pictures_admits_nobody() {
    let net = Arc::new(Network::default());
    let mum = founded(&net, 71).await;
    let stranger = Computer::start(&net, 72);
    let dad = Computer::start(&net, 73);

    mum.send(dad_invite());
    let code = mum.short_code().await;
    // A wrong code that happens to share the right one's hint, so it reaches Mum.
    let digits = sovra_core::group::normalise_code(&code).expect("six digits");
    let hint = sovra_core::group::code_hint(&digits);
    let wrong = (0..1_000_000u32)
        .map(|n| format!("{n:06}"))
        .find(|guess| *guess != digits && sovra_core::group::code_hint(guess) == hint)
        .expect("some other code shares the hint");
    stranger.send(GroupCommand::JoinWithCode {
        code: wrong,
        computer_name: "Unknown".into(),
    });
    stranger
        .wait(|event| matches!(event, GroupEvent::Problem(_)).then_some(()))
        .await;
    // One guess only: now even the right code no longer works.
    dad.send(GroupCommand::JoinWithCode {
        code,
        computer_name: "Desktop".into(),
    });
    dad.wait(|event| matches!(event, GroupEvent::Problem(_)).then_some(()))
        .await;

    // A fresh code, but Mum sees different pictures and says so.
    mum.send(dad_invite());
    let code = mum.short_code().await;
    dad.send(GroupCommand::JoinWithCode {
        code,
        computer_name: "Desktop".into(),
    });
    dad.pictures().await;
    mum.pictures().await;
    mum.send(GroupCommand::ConfirmCode { matches: false });
    dad.send(GroupCommand::ConfirmCode { matches: true });
    dad.wait(|event| {
        matches!(
            event,
            GroupEvent::Pairing(sovra_core::group::PairingStep::Failed(_))
        )
        .then_some(())
    })
    .await;
    mum.send(GroupCommand::Load);
    let roster = mum.roster(|state| state.name == "Family").await;
    assert!(
        !roster.admits(&dad.device),
        "nobody joins unless both say they match"
    );
    assert!(!roster.admits(&stranger.device));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shared_helper_and_schedule_reach_others_as_templates_to_copy() {
    let net = Arc::new(Network::default());
    let mum = founded(&net, 91).await;
    let dad = Computer::start(&net, 92);
    mum.send(GroupCommand::InvitePerson {
        name: "Dad".into(),
        role: Role::Member,
    });
    let link = mum.link().await;
    dad.send(GroupCommand::Join {
        link,
        computer_name: "Desktop".into(),
    });
    dad.roster(|state| state.admits(&dad.device)).await;

    mum.send(GroupCommand::ShareTemplate {
        template: sovra_core::group::Template::Helper {
            name: "Gardener".into(),
            icon: "🌱".into(),
            role: "Garden helper".into(),
            tagline: "Knows our garden".into(),
            instructions: "Water the roses on Sundays.".into(),
            expertise: Vec::new(),
        },
        with: None,
    });
    mum.send(GroupCommand::ShareTemplate {
        template: sovra_core::group::Template::Schedule {
            name: "Bin reminder".into(),
            cadence: "every Monday at 7pm".into(),
            prompt: "Remind us to put the bins out.".into(),
        },
        with: None,
    });
    let templates = dad
        .wait(|event| match event {
            GroupEvent::Templates(templates) if templates.len() == 2 => Some(templates),
            _ => None,
        })
        .await;
    assert!(
        templates
            .iter()
            .all(|shared| shared.shared_by == "Mum" && shared.is_latest())
    );
    assert!(
        templates
            .iter()
            .any(|shared| shared.template.body() == "Water the roses on Sundays.")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_computer_knows_which_others_are_online() {
    let net = Arc::new(Network::default());
    let mum = founded(&net, 101).await;
    let dad = Computer::start(&net, 102);
    mum.send(GroupCommand::InvitePerson {
        name: "Dad".into(),
        role: Role::Member,
    });
    let link = mum.link().await;
    dad.send(GroupCommand::Join {
        link,
        computer_name: "Desktop".into(),
    });
    dad.roster(|state| state.admits(&dad.device)).await;

    // Opening Groups syncs at once, and the sync reports who answered.
    mum.send(GroupCommand::Load);
    let seen = mum
        .wait(|event| match event {
            GroupEvent::Presence(seen) if seen.contains_key(&dad.device) => Some(seen),
            _ => None,
        })
        .await;
    assert!(
        !seen.contains_key(&mum.device),
        "a computer does not list itself"
    );

    // Dad's computer goes off: it is no longer heard from.
    net.off.lock().expect("not poisoned").insert(dad.device);
    let before = seen[&dad.device];
    mum.send(GroupCommand::Load);
    let after = mum
        .wait(|event| match event {
            GroupEvent::Presence(seen) => Some(seen),
            _ => None,
        })
        .await;
    assert_eq!(
        after.get(&dad.device),
        Some(&before),
        "not refreshed while off"
    );
}

/// Stands in for a helping computer's AI.
struct EchoAi;

#[async_trait::async_trait]
impl sovra_core::group::WorkModel for EchoAi {
    async fn answer(
        &self,
        job: sovra_core::group::ChatJob,
    ) -> Result<String, sovra_core::group::WorkError> {
        assert!(job.turns.last().is_some_and(|turn| turn.from_person));
        Ok(format!(
            "Mum's computer says: {}",
            job.turns.last().map_or("", |turn| turn.text.as_str())
        ))
    }
}

async fn reply(computer: &Computer, session: &str) -> Event {
    tokio::time::timeout(WAIT, async {
        loop {
            match computer.events.recv().await.expect("runtime running") {
                event @ (Event::AgentChunk { .. } | Event::AgentError { .. })
                    if matches!(&event, Event::AgentChunk { session_id, .. } | Event::AgentError { session_id, .. } if session_id == session) =>
                {
                    return event;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("a reply in time")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_computer_without_ai_gets_its_answer_from_a_free_computer_in_the_group() {
    let net = Arc::new(Network::default());
    let mum = Computer::start_with(&net, 111, Some(Arc::new(EchoAi)));
    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Desktop".into(),
    });
    mum.roster(|state| state.name == "Family").await;
    let dad = Computer::start(&net, 112);
    mum.send(GroupCommand::InvitePerson {
        name: "Dad".into(),
        role: Role::Member,
    });
    let link = mum.link().await;
    dad.send(GroupCommand::Join {
        link,
        computer_name: "Old laptop".into(),
    });
    dad.roster(|state| state.admits(&dad.device)).await;
    dad.send(GroupCommand::SetWork(sovra_core::group::WorkSettings {
        help_others: false,
        use_group: true,
    }));
    dad.wait(|event| match event {
        GroupEvent::Work { settings, .. } if settings.use_group => Some(()),
        _ => None,
    })
    .await;

    // Mum has not offered to help yet: Dad is told nobody is free.
    dad.client
        .send(Command::RunPrompt {
            context: None,
            session_id: "before".into(),
            prompt: "What's for dinner?".into(),
        })
        .expect("accepted");
    assert!(matches!(
        reply(&dad, "before").await,
        Event::AgentError { .. }
    ));

    mum.send(GroupCommand::SetWork(sovra_core::group::WorkSettings {
        help_others: true,
        use_group: false,
    }));
    mum.wait(|event| match event {
        GroupEvent::Work { settings, .. } if settings.help_others => Some(()),
        _ => None,
    })
    .await;
    dad.client
        .send(Command::RunPrompt {
            context: None,
            session_id: "after".into(),
            prompt: "What's for dinner?".into(),
        })
        .expect("accepted");
    let answer = reply(&dad, "after").await;
    assert!(
        matches!(&answer, Event::AgentChunk { text, .. } if text.contains("Mum's computer says: What's for dinner?")),
        "{answer:?}"
    );
    let log = mum
        .wait(|event| match event {
            GroupEvent::Work { recent, .. } if !recent.is_empty() => Some(recent),
            _ => None,
        })
        .await;
    assert_eq!(log[0].asked_by, "Dad");
    assert_eq!(log[0].computer, "Old laptop");
    assert!(log[0].answered);
}
