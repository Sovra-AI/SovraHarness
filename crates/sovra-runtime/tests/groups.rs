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
use sovra_core::training::{
    BYTE_MODEL_PARAMETERS, TrainingCommand, TrainingEngine, TrainingError, TrainingEvent,
    TrainingPeerRequest, TrainingPeerResponse, TrainingPhase, TrainingProgress, TrainingRecipe,
    TrainingResult, TrainingSession, TrainingSource, TrainingState, TrainingTask,
    encode_parameters,
};
use sovra_core::{AgentConfig, Command, Event};
use sovra_runtime::{Client, GroupServices, RuntimeServices, TrainingConfig};

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
    /// Broadcast subscribers: who joined which topic, and where to deliver.
    gossip: Mutex<BTreeMap<DeviceId, Subscriber>>,
    deduplicate_gossip: bool,
    audit_visible_prefix: Mutex<Option<u64>>,
    completed_checkpoints: Mutex<BTreeSet<(DeviceId, u64)>>,
    delivered_gossip: Mutex<BTreeSet<GossipDelivery>>,
}

/// Recipient, topic and content hash, matching content-based gossip suppression.
type GossipDelivery = (DeviceId, [u8; 32], [u8; 32]);

/// One computer's broadcast subscription: topic, handler and its spawner.
type Subscriber = ([u8; 32], Arc<dyn sovra_core::group::GossipHandler>, Spawn);

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
    async fn gossip_join(
        &self,
        topic: [u8; 32],
        _peers: Vec<PeerAddress>,
        handler: Arc<dyn sovra_core::group::GossipHandler>,
        spawn: Spawn,
    ) {
        self.net
            .gossip
            .lock()
            .expect("not poisoned")
            .insert(self.me, (topic, handler, spawn));
    }

    /// Delivers to every other switched-on computer on the topic, on its own task.
    async fn gossip_send(&self, topic: [u8; 32], message: Vec<u8>) {
        let off = self.net.off.lock().expect("not poisoned").clone();
        if off.contains(&self.me) {
            return;
        }
        let targets: Vec<_> = self
            .net
            .gossip
            .lock()
            .expect("not poisoned")
            .iter()
            .filter(|(device, (t, _, _))| {
                **device != self.me && *t == topic && !off.contains(device)
            })
            .map(|(device, (_, handler, spawn))| (*device, handler.clone(), spawn.clone()))
            .collect();
        for (device, handler, spawn) in targets {
            if self.net.deduplicate_gossip
                && !self
                    .net
                    .delivered_gossip
                    .lock()
                    .expect("not poisoned")
                    .insert((device, topic, Sha256::digest(&message).into()))
            {
                continue;
            }
            let message = message.clone();
            let net = self.net.clone();
            spawn(Box::pin(async move {
                handler.received(message.clone()).await;
                if let Some(sovra_core::swarm::audit::AuditGossip::Checkpoint(signed)) =
                    sovra_core::swarm::audit::AuditGossip::from_bytes(&message)
                {
                    net.completed_checkpoints
                        .lock()
                        .expect("not poisoned")
                        .insert((device, signed.checkpoint.size));
                }
            }));
        }
    }

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
        let mut response = handler.handle(self.me, request).await;
        if let PeerResponse::AuditEntries { entries } = &mut response
            && let Some(prefix) = *self.net.audit_visible_prefix.lock().expect("not poisoned")
        {
            entries.retain(|entry| entry.body.seq <= prefix);
        }
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
struct MemoryStore(Mutex<GroupRecord>, std::sync::atomic::AtomicBool);

#[async_trait::async_trait]
impl GroupStore for MemoryStore {
    async fn load(&self) -> Result<GroupRecord, GroupStoreError> {
        Ok(self.0.lock().expect("not poisoned").clone())
    }

    async fn save(&self, record: &GroupRecord) -> Result<(), GroupStoreError> {
        if self.1.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(GroupStoreError::Write("test disk is full".into()));
        }
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
        Self::start_full(net, n, helper, Arc::new(MemoryStore::default()))
    }

    /// The same computer switched on again with what it saved.
    fn restart(net: &Arc<Network>, n: u8, store: Arc<MemoryStore>) -> Self {
        Self::start_full(net, n, None, store)
    }

    fn start_full(
        net: &Arc<Network>,
        n: u8,
        helper: Option<Arc<dyn sovra_core::group::WorkModel>>,
        store: Arc<MemoryStore>,
    ) -> Self {
        Self::open(net, n, helper, store, None)
    }

    fn open(
        net: &Arc<Network>,
        n: u8,
        helper: Option<Arc<dyn sovra_core::group::WorkModel>>,
        store: Arc<MemoryStore>,
        training: Option<TrainingConfig>,
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
                training,
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

impl Computer {
    async fn audit(
        &self,
        pred: impl Fn(&sovra_core::swarm::audit::AuditSummary) -> bool,
    ) -> sovra_core::swarm::audit::AuditSummary {
        self.wait(|event| match event {
            GroupEvent::Audit(summary) if pred(&summary) => Some(summary),
            _ => None,
        })
        .await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_share_is_signed_into_an_audit_history_that_is_rechecked_on_start() {
    let net = Arc::new(Network::default());
    let mum = Computer::start(&net, 41);
    mum.send(GroupCommand::Create {
        name: "Smith Plumbing".into(),
        your_name: "Mum".into(),
        computer_name: "Office".into(),
    });
    mum.roster(|state| state.name == "Smith Plumbing").await;
    mum.send(GroupCommand::Share {
        text: "The van's MOT is due in May".into(),
        about: Vec::new(),
        with: None,
    });
    let summary = mum.audit(|s| s.shared == 1).await;
    assert!(summary.checked);
    assert_eq!(summary.records, 1);
    assert_eq!(summary.recent[0].words, "Shared a detail with the group");
    let saved = mum.store.0.lock().expect("not poisoned").clone();
    assert_eq!(saved.audit.len(), 1);
    // Only the hash is kept, never the words.
    let json = serde_json::to_string(&saved.audit).expect("encodes");
    assert!(!json.contains("MOT"));
    let fingerprint = summary.fingerprint.clone();
    let store = mum.store.clone();
    drop(mum);

    // Switched on again: the history re-verifies and keeps growing.
    let again = Computer::restart(&net, 41, store.clone());
    again.send(GroupCommand::Load);
    let reloaded = again.audit(|s| s.records == 1).await;
    assert!(reloaded.checked);
    assert_eq!(reloaded.fingerprint, fingerprint);
    drop(again);

    // Someone edits the saved history: it no longer verifies and is not extended.
    {
        let mut record = store.0.lock().expect("not poisoned");
        record.audit[0].body.at = sovra_core::Timestamp(0);
    }
    let tampered = Computer::restart(&net, 41, store.clone());
    tampered.send(GroupCommand::Load);
    let refused = tampered.audit(|s| !s.checked).await;
    assert_eq!(refused.records, 0);
    tampered.send(GroupCommand::Share {
        text: "Another detail".into(),
        about: Vec::new(),
        with: None,
    });
    tampered
        .knowledge(|details| has(details, "Another detail"))
        .await;
    assert_eq!(store.0.lock().expect("not poisoned").audit.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_group_shares_one_checked_audit_history_across_computers() {
    let net = Arc::new(Network::default());
    let mum = Computer::start(&net, 51);
    let dad = Computer::start(&net, 52);
    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Mum's laptop".into(),
    });
    mum.roster(|state| state.name == "Family").await;
    mum.send(GroupCommand::InvitePerson {
        name: "Dad".into(),
        role: Role::Member,
    });
    let link = mum.link().await;
    dad.send(GroupCommand::Join {
        link,
        computer_name: "Dad's desktop".into(),
    });
    dad.roster(|state| state.people.len() == 2).await;
    mum.send(GroupCommand::Share {
        text: "The van's MOT is due in May".into(),
        about: Vec::new(),
        with: None,
    });
    mum.audit(|s| s.records == 1).await;

    // Dad's next sync brings a checked copy of Mum's history.
    dad.send(GroupCommand::Load);
    let dads = dad.audit(|s| s.other_records >= 1).await;
    assert_eq!(dads.other_computers, 1);
    assert_eq!(dads.forked, Vec::<String>::new());
    let saved = dad.store.0.lock().expect("not poisoned").clone();
    assert_eq!(saved.peer_audit.len(), 1);
    assert_eq!(saved.peer_audit[0].body.author, mum.device);
    assert!(saved.audit.is_empty(), "Dad's own history is his alone");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computers_witness_each_others_history_over_the_broadcast_topic() {
    let net = Arc::new(Network {
        deduplicate_gossip: true,
        audit_visible_prefix: Mutex::new(Some(0)),
        ..Network::default()
    });
    let mum = Computer::start(&net, 71);
    let dad = Computer::start(&net, 72);
    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Mum's laptop".into(),
    });
    mum.roster(|state| state.name == "Family").await;
    mum.send(GroupCommand::InvitePerson {
        name: "Dad".into(),
        role: Role::Member,
    });
    let link = mum.link().await;
    dad.send(GroupCommand::Join {
        link,
        computer_name: "Dad's desktop".into(),
    });
    dad.roster(|state| state.people.len() == 2).await;
    mum.roster(|state| state.admits(&dad.device)).await;
    for text in ["The van's MOT is due in May", "Water the garden on Sunday"] {
        mum.send(GroupCommand::Share {
            text: text.into(),
            about: Vec::new(),
            with: None,
        });
    }
    mum.audit(|s| s.records == 2).await;
    // A checkpoint must be enough even when its receiver has no history yet.
    // Real gossip suppresses a repeated payload, so another Load cannot repair it.
    assert_eq!(
        dad.store.0.lock().expect("not poisoned").peer_audit,
        Vec::new()
    );
    tokio::time::timeout(WAIT, async {
        while !net
            .gossip
            .lock()
            .expect("not poisoned")
            .contains_key(&dad.device)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("Dad joined the broadcast topic");
    mum.send(GroupCommand::Load);
    tokio::time::timeout(WAIT, async {
        while !net
            .completed_checkpoints
            .lock()
            .expect("not poisoned")
            .contains(&(dad.device, 2))
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the checkpoint arrived before its history");
    *net.audit_visible_prefix.lock().expect("not poisoned") = Some(1);
    dad.send(GroupCommand::Load);
    dad.audit(|s| s.other_records == 1).await;
    assert!(
        mum.store
            .0
            .lock()
            .expect("not poisoned")
            .audit_witnesses
            .is_empty(),
        "a partial history is not enough to sign"
    );
    // A later bounded sync completes the history without another checkpoint.
    *net.audit_visible_prefix.lock().expect("not poisoned") = Some(2);
    dad.send(GroupCommand::Load);
    let vouched = mum.audit(|s| s.witnesses >= 1).await;
    assert_eq!(vouched.witnessed_records, 2);
    let saved = mum.store.0.lock().expect("not poisoned").clone();
    assert_eq!(saved.audit_witnesses.len(), 1);
    assert_eq!(saved.audit_witnesses[0].witness, dad.device);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_department_share_reaches_only_its_members() {
    let net = Arc::new(Network::default());
    let mum = Computer::start(&net, 61);
    let dad = Computer::start(&net, 62);
    let gran = Computer::start(&net, 63);
    mum.send(GroupCommand::Create {
        name: "Smith Plumbing".into(),
        your_name: "Mum".into(),
        computer_name: "Office".into(),
    });
    mum.roster(|state| state.name == "Smith Plumbing").await;
    for (who, name) in [(&dad, "Dad"), (&gran, "Gran")] {
        mum.send(GroupCommand::InvitePerson {
            name: name.into(),
            role: Role::Member,
        });
        let link = mum.link().await;
        who.send(GroupCommand::Join {
            link,
            computer_name: format!("{name}'s laptop"),
        });
        who.roster(|state| state.admits(&who.device)).await;
    }
    let roster = mum.roster(|state| state.people.len() == 3).await;
    let person_of = |device: &DeviceId| roster.owner_of(device).expect("member").person;
    let legal = sovra_core::swarm::department::DepartmentId::new("legal").expect("valid");
    mum.send(GroupCommand::SetDepartment {
        slug: legal.clone(),
        name: "Legal".into(),
        members: BTreeSet::from([person_of(&mum.device), person_of(&gran.device)]),
    });
    let roster = mum.roster(|state| !state.departments.is_empty()).await;
    let id = sovra_core::group::department_ref(&roster.group, &legal);
    // Dad isn't in Legal: he is told so and nothing is shared.
    dad.roster(|state| !state.departments.is_empty()).await;
    dad.send(GroupCommand::ShareInDepartment {
        department: id,
        text: "Dad tries Legal".into(),
        about: Vec::new(),
    });
    dad.wait(|event| match event {
        GroupEvent::Problem(_) => Some(()),
        _ => None,
    })
    .await;
    gran.roster(|state| !state.departments.is_empty()).await;
    mum.send(GroupCommand::ShareInDepartment {
        department: id,
        text: "The Jones contract renews in June".into(),
        about: Vec::new(),
    });
    mum.knowledge(|details| has(details, "Jones contract"))
        .await;
    gran.send(GroupCommand::Load);
    let seen = gran
        .knowledge(|details| has(details, "Jones contract"))
        .await;
    let detail = seen
        .iter()
        .find(|d| d.text.contains("Jones"))
        .expect("shared");
    assert_eq!(detail.space, sovra_core::group::Space::Department { id });
    // Dad syncs too, and gets the whole group's details but never Legal's.
    mum.send(GroupCommand::Share {
        text: "The van is booked for Friday".into(),
        about: Vec::new(),
        with: None,
    });
    dad.send(GroupCommand::Load);
    let dads = dad.knowledge(|details| has(details, "van is booked")).await;
    assert!(!dads.iter().any(|d| d.text.contains("Jones")));
    assert!(!dads.iter().any(|d| d.text.contains("Dad tries")));
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

    // Admission is published before the joiner's catch-up finishes. Wait for its
    // completed sync so an in-flight join response cannot refresh Mum's presence
    // after the fixture switches Dad off.
    dad.wait(|event| match event {
        GroupEvent::Presence(seen) if seen.contains_key(&mum.device) => Some(()),
        _ => None,
    })
    .await;
    // Loaded discards any earlier queued presence snapshots; the next one reports
    // the completed sync requested here, rather than an earlier online timestamp.
    mum.send(GroupCommand::Load);
    mum.wait(|event| matches!(event, GroupEvent::Loaded { .. }).then_some(()))
        .await;
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
    mum.wait(|event| matches!(event, GroupEvent::Loaded { .. }).then_some(()))
        .await;
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
        out: &(dyn for<'a> Fn(&'a str) + Send + Sync),
        _cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(), sovra_core::group::WorkError> {
        assert!(job.turns.last().is_some_and(|turn| turn.from_person));
        out("Mum's computer says: ");
        out(job.turns.last().map_or("", |turn| turn.text.as_str()));
        Ok(())
    }
}

/// The whole streamed answer for `session` (pieces joined), or its error.
async fn reply(computer: &Computer, session: &str) -> Result<String, String> {
    tokio::time::timeout(WAIT, async {
        let mut text = String::new();
        loop {
            match computer.events.recv().await.expect("runtime running") {
                Event::AgentChunk {
                    session_id,
                    text: piece,
                } if session_id == session => {
                    text.push_str(&piece);
                }
                Event::AgentError {
                    session_id,
                    message,
                } if session_id == session => {
                    return Err(message);
                }
                Event::AgentCompleted { session_id } if session_id == session => {
                    return Ok(text);
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
    assert!(reply(&dad, "before").await.is_err(), "nobody is free yet");

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
    let answer = reply(&dad, "after").await.expect("answered");
    assert!(
        answer.contains("Mum's computer says: What's for dinner?"),
        "streamed in pieces and joined: {answer:?}"
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

/// A slow helper AI: one word, then it keeps going until it is told to stop, and
/// reports how long the stop took to arrive.
struct SlowAi(async_channel::Sender<Duration>);

#[async_trait::async_trait]
impl sovra_core::group::WorkModel for SlowAi {
    async fn answer(
        &self,
        _job: sovra_core::group::ChatJob,
        out: &(dyn for<'a> Fn(&'a str) + Send + Sync),
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(), sovra_core::group::WorkError> {
        out("Once upon a time ");
        let wrote = std::time::Instant::now();
        let mut stopped = None;
        for _ in 0..300 {
            if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                stopped = Some(wrote.elapsed());
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if let Some(after) = stopped {
            let _ = self.0.send(after).await;
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_a_handed_off_chat_stops_the_helping_computer_within_a_second() {
    let net = Arc::new(Network::default());
    let (stops, stopped) = async_channel::bounded(1);
    let mum = Computer::start_with(&net, 121, Some(Arc::new(SlowAi(stops))));
    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Desktop".into(),
    });
    mum.roster(|state| state.name == "Family").await;
    let dad = Computer::start(&net, 122);
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
    mum.send(GroupCommand::SetWork(sovra_core::group::WorkSettings {
        help_others: true,
        use_group: false,
    }));
    dad.send(GroupCommand::SetWork(sovra_core::group::WorkSettings {
        help_others: false,
        use_group: true,
    }));
    for computer in [&mum, &dad] {
        computer
            .wait(|event| match event {
                GroupEvent::Work { settings, .. } if settings.help_others || settings.use_group => {
                    Some(())
                }
                _ => None,
            })
            .await;
    }

    dad.client
        .send(Command::RunPrompt {
            context: None,
            session_id: "story".into(),
            prompt: "Tell me a long story".into(),
        })
        .expect("accepted");
    // The first words arrive while the helper is still writing…
    tokio::time::timeout(WAIT, async {
        loop {
            if let Event::AgentChunk { session_id, text } =
                dad.events.recv().await.expect("runtime running")
                && session_id == "story"
            {
                assert!(text.contains("Once upon a time"));
                return;
            }
        }
    })
    .await
    .expect("streamed words");
    // …and pressing Stop on Dad's computer stops Mum's AI.
    dad.client
        .send(Command::InterruptSession {
            session_id: "story".into(),
        })
        .expect("accepted");
    let after = tokio::time::timeout(WAIT, stopped.recv())
        .await
        .expect("the helper was told to stop")
        .expect("reported");
    assert!(
        after < Duration::from_millis(1_500),
        "stopped {after:?} after writing"
    );
}

/// Start and join take the group-facing name they already ask for.
/// Neither command carries an email or a password, and neither waits for a login.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn starting_and_joining_a_group_needs_no_password_account() {
    let net = Arc::new(Network::default());
    let mum = Computer::start(&net, 91);
    let dad = Computer::start(&net, 92);
    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Kitchen".into(),
    });
    let started = mum.roster(|state| state.name == "Family").await;
    assert!(
        started.people.values().any(|person| person.name == "Mum"),
        "the group-facing name was not kept"
    );
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
    let names: Vec<_> = joined
        .people
        .values()
        .map(|person| person.name.as_str())
        .collect();
    assert!(names.contains(&"Mum"), "{names:?}");
    assert!(names.contains(&"Dad"), "{names:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_person_alone_can_leave_and_start_a_different_group() {
    let net = Arc::new(Network::default());
    let mum = Computer::start(&net, 91);
    mum.send(GroupCommand::Create {
        name: "Practice group".into(),
        your_name: "Mum".into(),
        computer_name: "Laptop".into(),
    });
    mum.roster(|state| state.name == "Practice group").await;
    mum.send(GroupCommand::Leave);
    mum.wait(|event| matches!(event, GroupEvent::LeftGroup).then_some(()))
        .await;
    assert_eq!(mum.store.0.lock().expect("not poisoned").group, None);
    mum.send(GroupCommand::Create {
        name: "Family".into(),
        your_name: "Mum".into(),
        computer_name: "Laptop".into(),
    });
    let state = mum.roster(|state| state.name == "Family").await;
    assert!(state.admits(&mum.device));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_shares_are_not_visible_or_saved_by_a_later_retry() {
    let net = Arc::new(Network::default());
    let mum = Computer::start(&net, 71);
    mum.send(GroupCommand::Create {
        name: "Storage failure test".into(),
        your_name: "Mum".into(),
        computer_name: "Laptop".into(),
    });
    mum.roster(|state| state.name == "Storage failure test")
        .await;
    let commands = [
        GroupCommand::Share {
            text: "Keep this draft".into(),
            about: Vec::new(),
            with: None,
        },
        GroupCommand::ShareArtifact {
            title: "Keep this document".into(),
            kind: sovra_core::artifacts::ArtifactKind::Document,
            language: String::new(),
            content: "Words that must not disappear".into(),
            new_version_of: None,
            local_id: String::new(),
            with: None,
        },
    ];
    for command in &commands {
        mum.store.1.store(true, std::sync::atomic::Ordering::SeqCst);
        mum.send(command.clone());
        mum.wait(|event| match event {
            GroupEvent::Problem(problem) if problem.message().contains("test disk is full") => {
                Some(())
            }
            _ => None,
        })
        .await;
        // Loaded is a barrier after the failed command; inspect its following snapshot.
        mum.send(GroupCommand::Load);
        mum.wait(|event| matches!(event, GroupEvent::Loaded { .. }).then_some(()))
            .await;
        assert_eq!(mum.knowledge(|_| true).await, Vec::<SharedDetail>::new());
        assert_eq!(mum.artifacts(|_| true).await, Vec::<SharedArtifact>::new());
        let saved = mum.store.0.lock().expect("not poisoned");
        assert_eq!(
            saved.knowledge,
            Vec::<sovra_core::group::KnowledgeOp>::new()
        );
        assert_eq!(
            saved.audit,
            Vec::<sovra_core::swarm::ledger::LedgerEntry>::new()
        );
    }
    mum.store
        .1
        .store(false, std::sync::atomic::Ordering::SeqCst);
    for command in commands {
        mum.send(command);
    }
    assert_eq!(
        mum.knowledge(|items| items.len() == 1).await[0].text,
        "Keep this draft"
    );
    assert_eq!(
        mum.artifacts(|items| items.len() == 1).await[0].content,
        "Words that must not disappear"
    );
    let saved = mum.store.0.lock().expect("not poisoned");
    assert_eq!(
        saved.audit.len(),
        2,
        "failed shares must not enter the history"
    );
}

use sovra_core::group::ContentHash;
use sovra_core::swarm::learn::scavenger::{GridSettings, GridSyncer};

/// Counts assignments. `work` is the learner; the coordinator must not call it.
struct CountingEngine {
    assignments: Arc<std::sync::atomic::AtomicU64>,
    learner_calls: Arc<std::sync::atomic::AtomicU64>,
}

impl CountingEngine {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            assignments: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            learner_calls: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }
}

impl TrainingEngine for CountingEngine {
    fn create(
        &self,
        text: &str,
        run: ContentHash,
    ) -> Result<Box<dyn TrainingSession>, TrainingError> {
        Ok(Box::new(LeaseSession::start(
            text,
            run,
            self.assignments.clone(),
        )?))
    }

    fn restore(&self, _: &[u8]) -> Result<Box<dyn TrainingSession>, TrainingError> {
        Err(TrainingError::InvalidCheckpoint)
    }

    fn work(&self, _: TrainingTask, _: DeviceId) -> Result<TrainingResult, TrainingError> {
        self.learner_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(TrainingError::InvalidWork)
    }
}

struct LeaseBatch {
    lease: sovra_core::swarm::learn::scavenger::WorkLease,
    tokens: u32,
    cursor_before: usize,
    cursor_after: usize,
}

struct LeaseSession {
    text: String,
    cursor: usize,
    grid: GridSyncer,
    pending: BTreeMap<u64, LeaseBatch>,
    assignments: Arc<std::sync::atomic::AtomicU64>,
}

impl LeaseSession {
    fn start(
        text: &str,
        run: ContentHash,
        assignments: Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<Self, TrainingError> {
        let grid = GridSyncer::new(
            run,
            vec![0.0; BYTE_MODEL_PARAMETERS],
            GridSettings {
                local_steps: 4,
                ..GridSettings::default()
            },
        )
        .map_err(|_| TrainingError::Step)?;
        Ok(Self {
            text: text.to_owned(),
            cursor: 0,
            grid,
            pending: BTreeMap::new(),
            assignments,
        })
    }
}

impl TrainingSession for LeaseSession {
    fn advance(&mut self, _: u64) -> Result<(), TrainingError> {
        // These tests isolate received work from local background progress.
        Ok(())
    }

    fn progress(&self) -> TrainingProgress {
        TrainingProgress {
            rounds: u32::try_from(self.grid.round()).unwrap_or(u32::MAX),
            total_rounds: 10_000,
            tokens: self.grid.tokens(),
            loss_milli: None,
        }
    }

    fn checkpoint(&self) -> Result<Vec<u8>, TrainingError> {
        serde_json::to_vec(&serde_json::json!({
            "rounds": self.grid.round(),
            "next_lease": self.grid.next_lease(),
        }))
        .map_err(|_| TrainingError::Step)
    }

    fn abandon(&mut self, worker: DeviceId) {
        let id = self
            .pending
            .iter()
            .find(|(_, batch)| batch.lease.worker == worker)
            .map(|(id, _)| *id);
        if let Some(id) = id
            && let Some(batch) = self.pending.remove(&id)
            && self.cursor == batch.cursor_after
        {
            self.cursor = batch.cursor_before;
        }
        self.grid.release(worker);
    }

    fn assign(&mut self, worker: DeviceId, now_ms: u64) -> Result<TrainingTask, TrainingError> {
        if self.progress().is_complete() {
            return Err(TrainingError::InvalidWork);
        }
        self.assignments
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.abandon(worker);
        let cursor_before = self.cursor;
        let start = if self.text.len() - self.cursor < 2 {
            self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(index, _)| index)
        } else {
            self.cursor
        };
        let mut end = (start + 512).min(self.text.len());
        while !self.text.is_char_boundary(end) {
            end -= 1;
        }
        let lease = self
            .grid
            .issue(worker, now_ms)
            .map_err(|_| TrainingError::InvalidWork)?;
        self.pending
            .retain(|_, batch| self.grid.has_lease(&batch.lease));
        let task = TrainingTask {
            recipe: TrainingRecipe::ByteBigramV1,
            lease,
            model: encode_parameters(self.grid.params())?,
            text: self.text[start..end].to_owned(),
        };
        let cursor_after = if end == self.text.len() {
            0
        } else {
            self.text[..end]
                .char_indices()
                .next_back()
                .map_or(end, |(index, _)| index)
        };
        self.cursor = cursor_after;
        self.pending.insert(
            task.lease.id,
            LeaseBatch {
                lease: task.lease.clone(),
                tokens: task.expected_tokens()?,
                cursor_before,
                cursor_after,
            },
        );
        Ok(task)
    }

    fn merge(
        &mut self,
        result: TrainingResult,
        worker: DeviceId,
        now_ms: u64,
    ) -> Result<(), TrainingError> {
        let Some(batch) = self.pending.get(&result.lease.id) else {
            return Err(TrainingError::InvalidWork);
        };
        if batch.lease != result.lease || batch.tokens != result.tokens {
            return Err(TrainingError::InvalidWork);
        }
        let contribution = result.contribution(worker)?;
        self.grid
            .merge(std::slice::from_ref(&contribution), now_ms)
            .map_err(|_| TrainingError::InvalidWork)?;
        self.pending.remove(&result.lease.id);
        Ok(())
    }
}

impl Computer {
    async fn training(&self, pred: impl Fn(&TrainingState) -> bool) -> TrainingState {
        tokio::time::timeout(WAIT, async {
            loop {
                if let Event::Training(TrainingEvent(state)) =
                    self.events.recv().await.expect("runtime running")
                    && pred(&state)
                {
                    return state;
                }
            }
        })
        .await
        .expect("training state in time")
    }
}

fn read_saved(path: &str) -> (u64, u64) {
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).expect("saved training")).expect("json");
    (
        value["rounds"].as_u64().expect("rounds"),
        value["next_lease"].as_u64().expect("next lease"),
    )
}

fn refusal(response: PeerResponse) -> sovra_core::group::Refusal {
    match response {
        PeerResponse::Refused { reason } => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

async fn ask_training(
    net: &Arc<Network>,
    from: DeviceId,
    to: DeviceId,
    group: sovra_core::group::GroupId,
    work: TrainingPeerRequest,
) -> Result<PeerResponse, TransportError> {
    let transport = LoopTransport {
        me: from,
        net: net.clone(),
        closed: tokio::sync::Notify::new(),
        cipher: Arc::new(FakeCipher {
            me: from,
            next: std::sync::atomic::AtomicU64::new(0),
        }),
    };
    transport
        .request(
            &PeerAddress {
                device: to,
                relay: None,
                addrs: vec!["loopback".into()],
            },
            PeerRequest::Training { group, work },
        )
        .await
}

struct TrainedFamily {
    _dir: tempfile::TempDir,
    net: Arc<Network>,
    mum: Computer,
    dad: Computer,
    outsider: Computer,
    group: sovra_core::group::GroupId,
    engine: Arc<CountingEngine>,
    paused: TrainingState,
    rounds: u64,
    next_lease: u64,
    assignments: u64,
}

impl TrainedFamily {
    async fn start(n: u8) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let engine = CountingEngine::new();
        let net = Arc::new(Network::default());
        let mum = Computer::open(
            &net,
            n,
            None,
            Arc::new(MemoryStore::default()),
            Some(TrainingConfig {
                engine: engine.clone(),
                directory: dir.path().join("training"),
            }),
        );
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
        let dad = Computer::start(&net, n + 1);
        dad.send(GroupCommand::Join {
            link,
            computer_name: "Dad's desktop".into(),
        });
        let roster = mum.roster(|state| state.people.len() == 2).await;
        let outsider = Computer::start(&net, n + 2);
        mum.client
            .send(Command::Training(TrainingCommand::Start(
                TrainingSource::Sample,
            )))
            .expect("start");
        mum.training(|state| state.phase == TrainingPhase::Running)
            .await;
        mum.client
            .send(Command::Training(TrainingCommand::Pause))
            .expect("pause");
        let paused = mum
            .training(|state| state.phase == TrainingPhase::Paused)
            .await;
        let path = paused.saved_at.clone().expect("saved path");
        let (rounds, next_lease) = read_saved(&path);
        assert_eq!(rounds, u64::from(paused.progress.rounds));
        Self {
            _dir: dir,
            net,
            mum,
            dad,
            outsider,
            group: roster.group,
            assignments: engine.assignments.load(std::sync::atomic::Ordering::SeqCst),
            engine,
            paused,
            rounds,
            next_lease,
        }
    }

    fn saved(&self) -> (u64, u64) {
        read_saved(self.paused.saved_at.as_ref().expect("saved path"))
    }

    async fn share(&mut self) {
        self.mum
            .client
            .send(Command::Training(TrainingCommand::StartShared(
                TrainingSource::Sample,
            )))
            .expect("share selected text");
        self.paused = self
            .mum
            .training(|state| {
                state.phase == TrainingPhase::Running && state.sharing == Some(self.group)
            })
            .await;
        (self.rounds, self.next_lease) = self.saved();
        self.assignments = self
            .engine
            .assignments
            .load(std::sync::atomic::Ordering::SeqCst);
    }

    async fn ask(&self, from: DeviceId, work: TrainingPeerRequest) -> PeerResponse {
        ask_training(&self.net, from, self.mum.device, self.group, work)
            .await
            .expect("delivered")
    }
}

fn zero_result(task: &TrainingTask) -> TrainingResult {
    TrainingResult {
        lease: task.lease.clone(),
        tokens: task.expected_tokens().expect("token count"),
        pseudo_gradient: encode_parameters(&vec![0.0; BYTE_MODEL_PARAMETERS]).expect("gradient"),
    }
}

fn rejected(response: &PeerResponse) -> bool {
    matches!(
        response,
        PeerResponse::Training {
            work: TrainingPeerResponse::Rejected
        }
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn group_training_help_starts_off_and_only_a_matching_update_is_saved() {
    let mut family = TrainedFamily::start(81).await;
    assert!(!family.mum.store.0.lock().expect("store").training_help);

    assert_eq!(
        refusal(
            family
                .ask(family.outsider.device, TrainingPeerRequest::Work)
                .await
        ),
        sovra_core::group::Refusal::NotAMember
    );
    assert_eq!(
        refusal(
            ask_training(
                &family.net,
                family.dad.device,
                family.mum.device,
                sovra_core::group::GroupId([8; 32]),
                TrainingPeerRequest::Work,
            )
            .await
            .expect("delivered")
        ),
        sovra_core::group::Refusal::WrongGroup
    );
    assert_eq!(
        refusal(
            family
                .ask(family.dad.device, TrainingPeerRequest::Work)
                .await
        ),
        sovra_core::group::Refusal::NotTraining
    );

    family.mum.send(GroupCommand::SetFederated(
        sovra_core::group::FederatedSettings {
            answer: true,
            ask: false,
        },
    ));
    family
        .mum
        .wait(|event| match event {
            GroupEvent::Federated { settings, .. } if settings.answer => Some(()),
            _ => None,
        })
        .await;
    family
        .mum
        .send(GroupCommand::SetWork(sovra_core::group::WorkSettings {
            help_others: true,
            use_group: false,
        }));
    family
        .mum
        .wait(|event| match event {
            GroupEvent::Work { settings, .. } if settings.help_others => Some(()),
            _ => None,
        })
        .await;
    assert_eq!(
        refusal(
            family
                .ask(family.dad.device, TrainingPeerRequest::Work)
                .await
        ),
        sovra_core::group::Refusal::NotTraining
    );
    assert_eq!(
        family
            .engine
            .assignments
            .load(std::sync::atomic::Ordering::SeqCst),
        family.assignments
    );
    assert_eq!(
        family
            .engine
            .learner_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(family.saved(), (family.rounds, family.next_lease));
    assert!(!family.mum.store.0.lock().expect("store").training_help);

    family.mum.send(GroupCommand::SetTrainingHelp(true));
    family
        .mum
        .wait(|event| matches!(event, GroupEvent::TrainingHelp(true)).then_some(()))
        .await;
    assert!(family.mum.store.0.lock().expect("store").training_help);
    assert!(
        matches!(
            family
                .ask(family.dad.device, TrainingPeerRequest::Work)
                .await,
            PeerResponse::Training {
                work: TrainingPeerResponse::Unavailable
            }
        ),
        "a general permission does not expose a paused local-only run"
    );
    family.share().await;

    let assigned = family
        .ask(family.dad.device, TrainingPeerRequest::Work)
        .await;
    let PeerResponse::Training {
        work: TrainingPeerResponse::Assigned(task),
    } = assigned
    else {
        panic!("expected one assignment, got {assigned:?}");
    };
    assert_eq!(task.lease.worker, family.dad.device);
    assert!(task.parameters(family.dad.device).is_ok());
    assert!(task.parameters(DeviceId([9; 32])).is_err());
    assert_eq!(
        family
            .engine
            .assignments
            .load(std::sync::atomic::Ordering::SeqCst),
        family.assignments + 1
    );
    assert_eq!(
        family
            .engine
            .learner_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    let (rounds, leased) = family.saved();
    assert_eq!(rounds, family.rounds);
    assert!(leased > family.next_lease);

    let good = zero_result(&task);
    let mut changed_worker = good.clone();
    changed_worker.lease.worker = DeviceId([9; 32]);
    let mut short = good.clone();
    short.pseudo_gradient.0.pop();
    let mut non_finite = good.clone();
    non_finite.pseudo_gradient.0[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    let mut wrong_tokens = good.clone();
    wrong_tokens.tokens = wrong_tokens.tokens.saturating_add(1);
    let mut changed_model = good.clone();
    changed_model.lease.base_model = ContentHash::of(b"changed-model");
    for bad in [
        changed_worker,
        short,
        non_finite,
        wrong_tokens,
        changed_model,
    ] {
        let response = family
            .ask(
                family.dad.device,
                TrainingPeerRequest::Submit(Box::new(bad)),
            )
            .await;
        assert!(rejected(&response), "{response:?}");
        assert_eq!(family.saved(), (rounds, leased));
    }

    let accepted = family
        .ask(
            family.dad.device,
            TrainingPeerRequest::Submit(Box::new(good.clone())),
        )
        .await;
    let PeerResponse::Training {
        work: TrainingPeerResponse::Accepted(progress),
    } = accepted
    else {
        panic!("expected the matching update to be saved, got {accepted:?}");
    };
    assert_eq!(u64::from(progress.rounds), rounds + 1);
    assert_eq!(family.saved().0, rounds + 1);
    let replay = family
        .ask(
            family.dad.device,
            TrainingPeerRequest::Submit(Box::new(good)),
        )
        .await;
    assert!(rejected(&replay), "{replay:?}");
    assert_eq!(family.saved().0, rounds + 1);
    assert_eq!(
        family
            .engine
            .learner_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_training_helper_does_not_change_the_saved_run() {
    let mut family = TrainedFamily::start(91).await;
    family.share().await;
    let first = family
        .ask(family.dad.device, TrainingPeerRequest::Work)
        .await;
    let PeerResponse::Training {
        work: TrainingPeerResponse::Assigned(first),
    } = first
    else {
        panic!("expected an assignment, got {first:?}");
    };
    let (rounds, leased) = family.saved();
    assert_eq!(rounds, family.rounds);
    assert!(leased > family.next_lease);

    family
        .net
        .off
        .lock()
        .expect("off")
        .insert(family.dad.device);
    assert!(
        ask_training(
            &family.net,
            family.dad.device,
            family.mum.device,
            family.group,
            TrainingPeerRequest::Work,
        )
        .await
        .is_err(),
        "a switched-off computer cannot deliver an update"
    );
    assert_eq!(family.saved(), (rounds, leased));

    family
        .net
        .off
        .lock()
        .expect("off")
        .remove(&family.dad.device);
    let again = family
        .ask(family.dad.device, TrainingPeerRequest::Work)
        .await;
    let PeerResponse::Training {
        work: TrainingPeerResponse::Assigned(again),
    } = again
    else {
        panic!("expected the same interval again, got {again:?}");
    };
    assert_eq!(again.text, first.text);
    assert!(again.lease.id > first.lease.id);
    assert_eq!(family.saved().0, rounds);

    let accepted = family
        .ask(
            family.dad.device,
            TrainingPeerRequest::Submit(Box::new(zero_result(&again))),
        )
        .await;
    let PeerResponse::Training {
        work: TrainingPeerResponse::Accepted(progress),
    } = accepted
    else {
        panic!("expected the later update to be saved, got {accepted:?}");
    };
    assert_eq!(u64::from(progress.rounds), rounds + 1);
    let stale = family
        .ask(
            family.dad.device,
            TrainingPeerRequest::Submit(Box::new(zero_result(&first))),
        )
        .await;
    assert!(rejected(&stale), "{stale:?}");
    assert_eq!(family.saved().0, rounds + 1);
    assert_eq!(
        family
            .engine
            .learner_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}
