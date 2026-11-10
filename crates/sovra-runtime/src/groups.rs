//! Groups: this computer's place in a group of people and their computers.
//!
//! Rules live in `sovra_core::group`; this module only does the IO around them:
//! signing with the transport's device key, saving the record, serving requests from
//! other computers, pushing new operations as soon as they exist, and syncing in the
//! background so a computer that was asleep catches up. Every local change is folded
//! *before* it is kept, so anything the rules reject is reported, never stored.
//! The shared state lock is never held across a network call.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use sovra_core::group::{AskLimiter, AskRecord, check_ask, federated_items, log_ask};
use sovra_core::group::{
    DeviceId, GroupCommand, GroupEvent, GroupId, GroupProblem, GroupRecord, GroupState, GroupStore,
    GroupTransport, Invite, InviteBook, InviteKind, JoinLink, KnowledgeBody, KnowledgeOp,
    KnowledgeRejection, KnowledgeView, LaneId, LinkError, MAX_REPLY_KNOWLEDGE_BYTES,
    MembershipBody, MembershipOp, PairingFailure, PairingStep, PeerAddress, PeerHandler,
    PeerRequest, PeerResponse, PersonId, RedeemError, Refusal, Rejection, SharedArtifact,
    SharedDetail, SignatureVerifier, Spawn, UnsignedOp, admits_request, check_artifact,
    check_share, fold, fold_knowledge, invite_proof, join_transcript, within_budget,
};
use sovra_core::{Event, Timestamp};
use tokio::sync::{Mutex, RwLock};

/// Builds the transport once the runtime's reactor is running (iroh needs it).
pub type TransportFuture =
    Pin<Box<dyn Future<Output = Result<Arc<dyn GroupTransport>, String>> + Send>>;
pub type TransportFactory = Box<dyn FnOnce() -> TransportFuture + Send>;

/// The adapters groups need, supplied by the composition root.
pub struct GroupServices {
    pub transport: TransportFactory,
    pub store: Arc<dyn GroupStore>,
}

impl std::fmt::Debug for GroupServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupServices").finish_non_exhaustive()
    }
}

/// How often a computer asks the others what it missed.
const SYNC_EVERY: Duration = Duration::from_secs(20);
/// A request to another computer gives up after this long.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Most catch-up rounds with one computer per sync; each round carries up to
/// `MAX_REPLY_KNOWLEDGE_BYTES`, so this bounds one sync at a few hundred megabytes.
const MAX_SYNC_ROUNDS: usize = 32;

/// What the Groups screens show of the shared lane.
struct SharedNow {
    details: Vec<SharedDetail>,
    artifacts: Vec<SharedArtifact>,
}

/// Opens this computer's own recall when another computer asks (federated recall).
/// A fresh reader each time, so pausing remembering takes effect at once.
pub(crate) type LocalMemory =
    Arc<dyn Fn() -> Option<Arc<dyn sovra_core::MemoryReader>> + Send + Sync>;

/// What helpers may read: the group's usable shared details, plus the way to ask the
/// group's computers (federated recall) and to answer them from this computer's memory.
#[derive(Clone, Default)]
pub(crate) struct GroupKnowledge(
    Arc<RwLock<KnowledgeView>>,
    Arc<std::sync::RwLock<Option<LocalMemory>>>,
    Arc<std::sync::RwLock<Option<std::sync::Weak<Groups>>>>,
);

impl GroupKnowledge {
    fn local_reader(&self) -> Option<Arc<dyn sovra_core::MemoryReader>> {
        let local = self
            .1
            .read()
            .expect("local memory lock is never poisoned")
            .clone();
        local.and_then(|open| open())
    }

    fn groups(&self) -> Option<Arc<Groups>> {
        self.2
            .read()
            .expect("groups handle lock is never poisoned")
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
    }

    /// Adds group knowledge to a turn that already reads personal memory. Turns
    /// without a reader (untrusted senders, remembering paused) get none either.
    pub(crate) fn wrap(
        &self,
        inner: Option<Arc<dyn sovra_core::MemoryReader>>,
    ) -> Option<Arc<dyn sovra_core::MemoryReader>> {
        inner.map(|inner| {
            Arc::new(GroupReader {
                inner,
                knowledge: self.clone(),
            }) as Arc<dyn sovra_core::MemoryReader>
        })
    }

    async fn add_to(&self, context: &mut sovra_core::MemoryContext, question: &str) {
        let (details, artifacts) = {
            let view = self.0.read().await;
            (
                view.search(question, sovra_core::MAX_GROUP_EVIDENCE_ITEMS)
                    .iter()
                    .map(|hit| evidence(&hit.detail))
                    .collect::<Vec<_>>(),
                view.search_artifacts(question, ARTIFACT_EVIDENCE_ITEMS)
                    .into_iter()
                    .map(artifact_evidence)
                    .collect::<Vec<_>>(),
            )
        };
        let asked = match self.groups() {
            Some(groups) => groups.ask_group(question).await,
            None => Vec::new(),
        };
        // Shared details lead; each other kind keeps a place before the rest fills in.
        let mut details = details.into_iter();
        let mut artifacts = artifacts.into_iter();
        let mut ordered: Vec<sovra_core::GroupEvidence> =
            details.by_ref().take(DETAILS_FIRST).collect();
        ordered.extend(artifacts.next());
        ordered.extend(asked.into_iter().take(ASKED_EVIDENCE_ITEMS));
        ordered.extend(artifacts);
        ordered.extend(details);
        for item in ordered {
            context.push_group(item);
        }
    }
}

/// Shared details that come before anything else in group recall.
const DETAILS_FIRST: usize = 3;
/// Most answers from other computers' memories one recall adds.
const ASKED_EVIDENCE_ITEMS: usize = 2;
/// How long recall waits for the group's computers to answer.
const ASK_TIMEOUT: Duration = Duration::from_millis(2_500);
/// A computer that did not answer is not asked again for this long.
const ASK_COOLDOWN_MS: i64 = 5 * 60 * 1_000;

/// Most shared artifacts one recall adds.
const ARTIFACT_EVIDENCE_ITEMS: usize = 2;
/// How much of a shared artifact a helper reads in recall.
const ARTIFACT_EXCERPT_CHARS: usize = 1_200;

/// A shared artifact as recall evidence: its title and opening, attributed.
fn artifact_evidence(artifact: &SharedArtifact) -> sovra_core::GroupEvidence {
    let mut excerpt: String = artifact
        .content
        .chars()
        .take(ARTIFACT_EXCERPT_CHARS)
        .collect();
    if artifact.content.chars().count() > ARTIFACT_EXCERPT_CHARS {
        excerpt.push('…');
    }
    sovra_core::GroupEvidence {
        citation: format!("group-artifact:{}", &artifact.id.to_hex()[..12]),
        statement: format!(
            "Shared {} “{}”:\n{excerpt}",
            artifact.kind.label().to_lowercase(),
            artifact.title
        ),
        shared_by: artifact.shared_by.clone(),
        computer: artifact.computer.clone(),
        shared_at: artifact.shared_at,
        valid_at: None,
        disputed: matches!(
            artifact.status,
            sovra_core::group::DetailStatus::Disputed { .. }
        ),
        agreed_by: 0,
    }
}

/// A shared detail as recall evidence, cited by a short stable id.
fn evidence(detail: &SharedDetail) -> sovra_core::GroupEvidence {
    sovra_core::GroupEvidence {
        citation: format!("group:{}", &detail.id.to_hex()[..12]),
        statement: detail.text.clone(),
        shared_by: detail.shared_by.clone(),
        computer: detail.computer.clone(),
        shared_at: detail.shared_at,
        valid_at: detail.valid_at,
        disputed: matches!(
            detail.status,
            sovra_core::group::DetailStatus::Disputed { .. }
        ),
        agreed_by: detail.agreed_by.len(),
    }
}

/// Personal recall plus the group's shared details for the same question.
struct GroupReader {
    inner: Arc<dyn sovra_core::MemoryReader>,
    knowledge: GroupKnowledge,
}

#[async_trait::async_trait]
impl sovra_core::MemoryReader for GroupReader {
    async fn search(
        &self,
        query: sovra_core::MemorySearch,
    ) -> Result<sovra_core::MemoryContext, sovra_core::MemoryError> {
        let question = query.text.clone();
        let mut context = self.inner.search(query).await?;
        self.knowledge.add_to(&mut context, &question).await;
        Ok(context)
    }

    async fn search_batch(
        &self,
        queries: Vec<sovra_core::MemorySearch>,
    ) -> Result<Vec<sovra_core::MemoryContext>, sovra_core::MemoryError> {
        let questions: Vec<String> = queries.iter().map(|query| query.text.clone()).collect();
        let mut contexts = self.inner.search_batch(queries).await?;
        for (context, question) in contexts.iter_mut().zip(&questions) {
            self.knowledge.add_to(context, question).await;
        }
        Ok(contexts)
    }
}

pub(crate) struct GroupRuntime {
    commands: Sender<GroupCommand>,
    pub(crate) knowledge: GroupKnowledge,
    /// Owns every group task; dropping the runtime cancels them all.
    _tasks: Arc<std::sync::Mutex<tokio::task::JoinSet<()>>>,
}

fn spawner(
    tasks: &Arc<std::sync::Mutex<tokio::task::JoinSet<()>>>,
    handle: &tokio::runtime::Handle,
) -> Spawn {
    let tasks = Arc::downgrade(tasks);
    let handle = handle.clone();
    Arc::new(move |task| {
        if let Some(tasks) = tasks.upgrade() {
            tasks
                .lock()
                .expect("group task set lock is never poisoned")
                .spawn_on(task, &handle);
        }
    })
}

impl GroupRuntime {
    pub(crate) fn start(
        services: Option<GroupServices>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (commands, receiver) = async_channel::bounded::<GroupCommand>(32);
        let knowledge = GroupKnowledge::default();
        let tasks = Arc::new(std::sync::Mutex::new(tokio::task::JoinSet::new()));
        let spawn = spawner(&tasks, handle);
        let readers = knowledge.clone();
        let launch = spawn.clone();
        launch(Box::pin(async move {
            let Some(services) = services else {
                while let Ok(command) = receiver.recv().await {
                    if !matches!(command, GroupCommand::Load) {
                        problem(&events, unavailable()).await;
                    }
                }
                return;
            };
            let transport = match (services.transport)().await {
                Ok(transport) => transport,
                Err(error) => {
                    tracing::warn!(%error, "group network unavailable");
                    while let Ok(command) = receiver.recv().await {
                        if !matches!(command, GroupCommand::Load) {
                            problem(
                                &events,
                                GroupProblem::Other(format!(
                                    "Groups can't connect on this computer right now: {error}"
                                )),
                            )
                            .await;
                        }
                    }
                    return;
                }
            };
            let groups = Arc::new(
                Groups::open(
                    transport,
                    services.store,
                    events,
                    readers.clone(),
                    spawn.clone(),
                )
                .await,
            );
            *readers
                .2
                .write()
                .expect("groups handle lock is never poisoned") = Some(Arc::downgrade(&groups));
            let serving = groups.clone();
            spawn(Box::pin(async move {
                let handler: Arc<dyn PeerHandler> = Arc::new(GroupHandler(serving.clone()));
                serving
                    .transport
                    .serve(handler, serving.spawn.clone())
                    .await;
            }));
            let syncing = groups.clone();
            spawn(Box::pin(async move {
                loop {
                    syncing.sync_all().await;
                    tokio::time::sleep(SYNC_EVERY).await;
                }
            }));
            while let Ok(command) = receiver.recv().await {
                groups.command(command).await;
            }
            groups.transport.close().await;
        }));
        Self {
            commands,
            knowledge,
            _tasks: tasks,
        }
    }

    /// Lets other computers' helpers ask this computer's memory (when the owner allows).
    pub(crate) fn attach_memory(&self, open: LocalMemory) {
        *self
            .knowledge
            .1
            .write()
            .expect("local memory lock is never poisoned") = Some(open);
    }

    pub(crate) fn send(&self, command: GroupCommand) {
        if self.commands.try_send(command).is_err() {
            tracing::warn!("group command dropped: the groups worker is busy or stopped");
        }
    }
}

fn unavailable() -> GroupProblem {
    GroupProblem::Other("Groups aren't available in this session.".into())
}

async fn problem(events: &Sender<Event>, problem: GroupProblem) {
    let _ = events
        .send(Event::Group(Box::new(GroupEvent::Problem(problem))))
        .await;
}

fn now() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    Timestamp(i64::try_from(millis).unwrap_or(i64::MAX))
}

fn random<const N: usize>() -> Option<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).ok().map(|()| bytes)
}

struct Inner {
    record: GroupRecord,
    roster: Option<GroupState>,
    view: KnowledgeView,
    invites: InviteBook,
    /// Questions other computers asked this one, newest first (not saved).
    asks: Vec<AskRecord>,
    limiter: AskLimiter,
    /// Computers that recently did not answer, and until when they are skipped.
    quiet_until: std::collections::BTreeMap<DeviceId, Timestamp>,
}

struct Groups {
    transport: Arc<dyn GroupTransport>,
    verifier: Arc<dyn SignatureVerifier + Send + Sync>,
    store: Arc<dyn GroupStore>,
    events: Sender<Event>,
    knowledge: GroupKnowledge,
    spawn: Spawn,
    inner: Mutex<Inner>,
}

impl Groups {
    async fn open(
        transport: Arc<dyn GroupTransport>,
        store: Arc<dyn GroupStore>,
        events: Sender<Event>,
        knowledge: GroupKnowledge,
        spawn: Spawn,
    ) -> Self {
        let record = match store.load().await {
            Ok(record) => record,
            Err(error) => {
                tracing::warn!(%error, "saved group could not be read; starting without one");
                GroupRecord::default()
            }
        };
        let verifier = transport.verifier();
        let groups = Self {
            transport,
            verifier,
            store,
            events,
            knowledge,
            spawn,
            inner: Mutex::new(Inner {
                record,
                roster: None,
                view: KnowledgeView::default(),
                invites: InviteBook::default(),
                asks: Vec::new(),
                limiter: AskLimiter::default(),
                quiet_until: std::collections::BTreeMap::new(),
            }),
        };
        {
            let mut inner = groups.inner.lock().await;
            groups.refold(&mut inner).await;
        }
        groups.publish_loaded().await;
        groups
    }

    fn me(&self) -> DeviceId {
        self.transport.this_device()
    }

    async fn emit(&self, event: GroupEvent) {
        let _ = self.events.send(Event::Group(Box::new(event))).await;
    }

    async fn refold(&self, inner: &mut Inner) {
        let verifier = self.verifier.as_ref();
        inner.roster = inner
            .record
            .group
            .map(|group| fold(group, &inner.record.membership, verifier));
        inner.view = match &inner.roster {
            Some(roster) => fold_knowledge(
                roster,
                LaneId::whole_group(&roster.group),
                &inner.record.knowledge,
                verifier,
            ),
            None => KnowledgeView::default(),
        };
        inner.view.clone_into(&mut *self.knowledge.0.write().await);
    }

    async fn persist(&self, inner: &Inner) {
        if let Err(error) = self.store.save(&inner.record).await {
            tracing::warn!(%error, "group record not saved");
            self.emit(GroupEvent::Problem(GroupProblem::Other(error.to_string())))
                .await;
        }
    }

    fn shared(view: &KnowledgeView) -> SharedNow {
        let mut details: Vec<SharedDetail> = view.details.values().cloned().collect();
        details.sort_by(|a, b| b.shared_at.cmp(&a.shared_at).then(b.id.cmp(&a.id)));
        SharedNow {
            details,
            artifacts: view.shared_artifacts(),
        }
    }

    async fn publish_shared(&self, shared: SharedNow) {
        self.emit(GroupEvent::Knowledge(shared.details)).await;
        self.emit(GroupEvent::Artifacts(shared.artifacts)).await;
    }

    async fn publish_loaded(&self) {
        let (roster, shared, settings, recent) = {
            let inner = self.inner.lock().await;
            (
                inner.roster.clone(),
                Self::shared(&inner.view),
                inner.record.federated,
                inner.asks.clone(),
            )
        };
        self.emit(GroupEvent::Loaded {
            this_computer: self.me(),
            group: roster,
        })
        .await;
        self.publish_shared(shared).await;
        self.emit(GroupEvent::Federated { settings, recent }).await;
    }

    async fn publish_changed(&self, roster: Option<GroupState>, shared: SharedNow) {
        if let Some(roster) = roster {
            self.emit(GroupEvent::Changed(roster)).await;
        }
        self.publish_shared(shared).await;
    }

    async fn publish_left(&self) {
        self.emit(GroupEvent::LeftGroup).await;
        self.publish_shared(SharedNow {
            details: Vec::new(),
            artifacts: Vec::new(),
        })
        .await;
    }

    fn address_of(record: &GroupRecord, device: DeviceId) -> PeerAddress {
        record
            .addresses
            .iter()
            .find(|address| address.device == device)
            .cloned()
            .unwrap_or(PeerAddress {
                device,
                relay: None,
                addrs: Vec::new(),
            })
    }

    fn remember(record: &mut GroupRecord, address: PeerAddress) {
        record
            .addresses
            .retain(|known| known.device != address.device);
        record.addresses.push(address);
    }

    /// Other computers currently in the roster, with the best known address.
    fn peers(&self, inner: &Inner) -> Vec<PeerAddress> {
        let me = self.me();
        inner
            .roster
            .iter()
            .flat_map(|roster| roster.people.values())
            .flat_map(|member| member.devices.keys().copied())
            .filter(|device| *device != me)
            .map(|device| Self::address_of(&inner.record, device))
            .collect()
    }

    async fn ask(&self, to: &PeerAddress, request: PeerRequest) -> Result<PeerResponse, String> {
        match tokio::time::timeout(REQUEST_TIMEOUT, self.transport.request(to, request)).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err("no answer in time".into()),
        }
    }

    /// Sends new operations to every other computer, each on its own owned task.
    fn broadcast(
        self: &Arc<Self>,
        group: GroupId,
        peers: Vec<PeerAddress>,
        membership: &[MembershipOp],
        knowledge: &[KnowledgeOp],
    ) {
        for peer in peers {
            let groups = self.clone();
            let request = PeerRequest::Push {
                group,
                membership: membership.to_vec(),
                knowledge: knowledge.to_vec(),
            };
            (self.spawn)(Box::pin(async move {
                if let Err(error) = groups.ask(&peer, request).await {
                    tracing::debug!(%error, peer = %peer.device.fingerprint(), "push not delivered; sync will retry");
                }
            }));
        }
    }

    /// Adds operations this computer did not have. Returns whether anything changed.
    async fn merge(&self, membership: Vec<MembershipOp>, knowledge: Vec<KnowledgeOp>) -> bool {
        let mut inner = self.inner.lock().await;
        let have_membership: BTreeSet<_> = inner
            .record
            .membership
            .iter()
            .map(MembershipOp::hash)
            .collect();
        let have_knowledge: BTreeSet<_> = inner
            .record
            .knowledge
            .iter()
            .map(KnowledgeOp::hash)
            .collect();
        let mut changed = false;
        for op in membership {
            if !have_membership.contains(&op.hash())
                && Some(op.unsigned.group) == inner.record.group
            {
                inner.record.membership.push(op);
                changed = true;
            }
        }
        for op in knowledge {
            if !have_knowledge.contains(&op.hash())
                && Some(op.unsigned().group) == inner.record.group
            {
                inner.record.knowledge.push(op);
                changed = true;
            }
        }
        if !changed {
            return false;
        }
        self.refold(&mut inner).await;
        let removed = inner
            .roster
            .as_ref()
            .is_some_and(|roster| !roster.admits(&self.me()));
        if removed {
            Self::forget_group(&mut inner.record);
            self.refold(&mut inner).await;
        }
        self.persist(&inner).await;
        let roster = inner.roster.clone();
        let shared = Self::shared(&inner.view);
        drop(inner);
        if removed {
            self.publish_left().await;
        } else {
            self.publish_changed(roster, shared).await;
        }
        true
    }

    fn forget_group(record: &mut GroupRecord) {
        record.group = None;
        record.membership.clear();
        record.knowledge.clear();
        record.addresses.clear();
    }

    /// Asks every other computer what this one missed, and sends what they missed.
    async fn sync_all(self: &Arc<Self>) {
        let peers = {
            let inner = self.inner.lock().await;
            if inner.record.group.is_none() {
                return;
            }
            self.peers(&inner)
        };
        for peer in peers {
            // A large history arrives over several rounds; stop once a round brings
            // nothing new.
            for _ in 0..MAX_SYNC_ROUNDS {
                if !self.sync_round(&peer).await {
                    break;
                }
            }
        }
    }

    /// One exchange with `peer`. Returns whether it brought anything new.
    async fn sync_round(self: &Arc<Self>, peer: &PeerAddress) -> bool {
        let (group, membership, knowledge) = {
            let inner = self.inner.lock().await;
            let Some(group) = inner.record.group else {
                return false;
            };
            (
                group,
                inner
                    .record
                    .membership
                    .iter()
                    .map(MembershipOp::hash)
                    .collect::<BTreeSet<_>>(),
                inner
                    .record
                    .knowledge
                    .iter()
                    .map(KnowledgeOp::hash)
                    .collect::<BTreeSet<_>>(),
            )
        };
        let request = PeerRequest::Sync {
            group,
            membership,
            knowledge,
            from: self.transport.address(),
        };
        match self.ask(peer, request).await {
            Ok(PeerResponse::Synced {
                membership: theirs,
                knowledge: their_knowledge,
                want_membership,
                want_knowledge,
            }) => {
                let changed = self.merge(theirs, their_knowledge).await;
                if !want_membership.is_empty() || !want_knowledge.is_empty() {
                    let (wanted, wanted_knowledge) = {
                        let inner = self.inner.lock().await;
                        (
                            inner
                                .record
                                .membership
                                .iter()
                                .filter(|op| want_membership.contains(&op.hash()))
                                .cloned()
                                .collect::<Vec<_>>(),
                            inner
                                .record
                                .knowledge
                                .iter()
                                .filter(|op| want_knowledge.contains(&op.hash()))
                                .cloned()
                                .collect::<Vec<_>>(),
                        )
                    };
                    // What does not fit is asked for again on the peer's next sync.
                    let (wanted_knowledge, _) =
                        within_budget(wanted_knowledge, MAX_REPLY_KNOWLEDGE_BYTES);
                    self.broadcast(group, vec![peer.clone()], &wanted, &wanted_knowledge);
                }
                changed
            }
            Ok(other) => {
                tracing::debug!(?other, "unexpected sync answer");
                false
            }
            Err(error) => {
                tracing::debug!(%error, peer = %peer.device.fingerprint(), "sync skipped");
                false
            }
        }
    }

    async fn command(self: &Arc<Self>, command: GroupCommand) {
        match command {
            GroupCommand::Load => {
                self.publish_loaded().await;
                // Opening Groups is also "catch me up now", not in up to 20 seconds.
                let groups = self.clone();
                (self.spawn)(Box::pin(async move { groups.sync_all().await }));
            }
            GroupCommand::Create {
                name,
                your_name,
                computer_name,
            } => self.create(name, your_name, computer_name).await,
            GroupCommand::InviteComputer => self.invite(InviteKind::OwnDevice).await,
            GroupCommand::InvitePerson { name, role } => {
                self.invite(InviteKind::Person { name, role }).await;
            }
            GroupCommand::Join {
                link,
                computer_name,
            } => {
                self.join(&link, computer_name).await;
                // A join hands over as much history as fits in one message; fetch the
                // rest (and meet the other computers) straight away.
                let groups = self.clone();
                (self.spawn)(Box::pin(async move { groups.sync_all().await }));
            }
            GroupCommand::ShowShortCode | GroupCommand::JoinWithCode { .. } => {
                self.emit(GroupEvent::Problem(GroupProblem::Other(
                    "Same-room codes aren't ready yet. Use the link or QR code instead.".into(),
                )))
                .await;
            }
            GroupCommand::ConfirmCode { .. } => {}
            GroupCommand::CancelPairing => {
                self.emit(GroupEvent::Pairing(PairingStep::Cancel)).await;
            }
            GroupCommand::Rename { name } => self.membership(MembershipBody::Rename { name }).await,
            GroupCommand::ChangeRole { person, role } => {
                self.membership(MembershipBody::ChangeRole { person, role })
                    .await;
            }
            GroupCommand::RemoveComputer { device } => {
                self.membership(MembershipBody::RemoveDevice { device })
                    .await;
            }
            GroupCommand::RemovePerson { person } => {
                self.membership(MembershipBody::RemovePerson { person })
                    .await;
            }
            GroupCommand::Leave => {
                let person = {
                    let inner = self.inner.lock().await;
                    inner
                        .roster
                        .as_ref()
                        .and_then(|roster| roster.owner_of(&self.me()))
                        .map(|member| member.person)
                };
                match person {
                    Some(person) => {
                        self.membership(MembershipBody::RemovePerson { person })
                            .await;
                    }
                    None => self.emit(GroupEvent::LeftGroup).await,
                }
            }
            GroupCommand::Share { text, about } => {
                self.knowledge(KnowledgeBody::Share {
                    text,
                    about,
                    valid_at: None,
                    replaces: Vec::new(),
                })
                .await;
            }
            GroupCommand::Correct { details, text } => {
                self.knowledge(KnowledgeBody::Share {
                    text,
                    about: Vec::new(),
                    valid_at: None,
                    replaces: details,
                })
                .await;
            }
            GroupCommand::Agree { detail } => {
                self.knowledge(KnowledgeBody::Corroborate { detail }).await;
            }
            GroupCommand::Withdraw { detail } => {
                self.knowledge(KnowledgeBody::Withdraw { detail }).await;
            }
            GroupCommand::Erase { detail } => {
                self.knowledge(KnowledgeBody::Redact { detail }).await;
            }
            GroupCommand::AskGroup { question } => {
                // Waiting on other computers must not hold up the next command.
                let groups = self.clone();
                (self.spawn)(Box::pin(async move {
                    let answers = groups
                        .ask_peers(&question, true)
                        .await
                        .into_iter()
                        .map(|(_, answer)| answer)
                        .collect();
                    groups.emit(GroupEvent::Answers { question, answers }).await;
                }));
            }
            GroupCommand::SetFederated(settings) => {
                let recent = {
                    let mut inner = self.inner.lock().await;
                    inner.record.federated = settings;
                    self.persist(&inner).await;
                    inner.asks.clone()
                };
                self.emit(GroupEvent::Federated { settings, recent }).await;
            }
            GroupCommand::ShareArtifact {
                title,
                kind,
                language,
                content,
                new_version_of,
                local_id,
            } => {
                let replaces = match new_version_of {
                    Some(original) => self.inner.lock().await.view.latest_versions_of(&original),
                    None => Vec::new(),
                };
                self.knowledge(KnowledgeBody::ShareArtifact {
                    title,
                    kind,
                    language,
                    content,
                    replaces,
                    origin: local_id,
                })
                .await;
            }
        }
    }

    async fn create(&self, name: String, your_name: String, computer_name: String) {
        let mut inner = self.inner.lock().await;
        if inner.record.group.is_some() {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "This computer is already in a group. Leave it first.".into(),
            )))
            .await;
            return;
        }
        let (Some(nonce), Some(person)) = (random::<16>(), random::<32>()) else {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "This computer couldn't make a secure group key. Try again.".into(),
            )))
            .await;
            return;
        };
        let me = self.me();
        let group = GroupId::derive(&me, &nonce);
        let person = PersonId(person);
        let unsigned = UnsignedOp {
            group,
            author: me,
            seq: 1,
            prev: None,
            deps: Vec::new(),
            at: now(),
            body: MembershipBody::Genesis {
                name,
                nonce,
                founder: person,
                founder_name: your_name.clone(),
                device_name: computer_name.clone(),
            },
        };
        let op = unsigned
            .clone()
            .sign(self.transport.sign(&unsigned.signing_bytes()));
        let trial = fold(group, [&op], self.verifier.as_ref());
        if let Some(rejection) = trial.rejected.get(&op.hash()) {
            drop(inner);
            self.emit(GroupEvent::Problem(membership_problem(*rejection)))
                .await;
            return;
        }
        inner.record = GroupRecord {
            person: Some(person),
            person_name: your_name,
            computer_name,
            group: Some(group),
            membership: vec![op],
            knowledge: Vec::new(),
            addresses: Vec::new(),
            federated: inner.record.federated,
        };
        self.refold(&mut inner).await;
        self.persist(&inner).await;
        drop(inner);
        self.publish_loaded().await;
    }

    async fn invite(&self, kind: InviteKind) {
        let mut inner = self.inner.lock().await;
        let me = self.me();
        let Some(roster) = inner.roster.clone() else {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "Create or join a group first.".into(),
            )))
            .await;
            return;
        };
        if matches!(kind, InviteKind::Person { .. })
            && !roster
                .role_of(&me)
                .is_some_and(sovra_core::group::Role::manages)
        {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::NotAllowed))
                .await;
            return;
        }
        let Some(secret) = random::<16>() else {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "This computer couldn't make a secure invite. Try again.".into(),
            )))
            .await;
            return;
        };
        let at = now();
        inner.invites.prune(at);
        let invite = Invite::new(roster.group, kind.clone(), secret, at);
        let expires_at = invite.expires_at;
        inner.invites.issue(invite);
        let address = self.transport.address();
        let link = JoinLink::new(
            roster.group,
            roster.name.clone(),
            me,
            roster
                .owner_of(&me)
                .map(|member| member.name.clone())
                .unwrap_or_default(),
            kind,
            secret,
            expires_at,
            address.relay,
            address.addrs,
        )
        .encode();
        drop(inner);
        self.emit(GroupEvent::InviteReady { link, expires_at })
            .await;
    }

    async fn join(&self, link: &str, computer_name: String) {
        let link = match JoinLink::parse(link, now()) {
            Ok(link) => link,
            Err(error) => {
                let problem = match error {
                    LinkError::Expired => GroupProblem::InviteExpired,
                    LinkError::NotALink | LinkError::TooLong | LinkError::Damaged => {
                        GroupProblem::BadInvite
                    }
                    LinkError::NewerVersion => GroupProblem::Other(error.to_string()),
                };
                self.emit(GroupEvent::Problem(problem)).await;
                return;
            }
        };
        if self.inner.lock().await.record.group.is_some() {
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "This computer is already in a group. Leave it first.".into(),
            )))
            .await;
            return;
        }
        let Some(person) = random::<32>() else {
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "This computer couldn't make a secure key. Try again.".into(),
            )))
            .await;
            return;
        };
        let me = self.me();
        let person_name = match &link.kind {
            InviteKind::Person { name, .. } => name.clone(),
            InviteKind::OwnDevice => link.inviter_name.clone(),
        };
        let inviter = PeerAddress {
            device: link.inviter,
            relay: link.relay.clone(),
            addrs: link.addrs.clone(),
        };
        self.emit(GroupEvent::Pairing(PairingStep::PeerConnected))
            .await;
        let request = PeerRequest::Join {
            group: link.group,
            proof: invite_proof(
                &link.secret,
                &join_transcript(&link.inviter, &me, &link.group),
            ),
            person: PersonId(person),
            person_name,
            computer_name: computer_name.clone(),
            from: self.transport.address(),
        };
        match self.ask(&inviter, request).await {
            Ok(PeerResponse::Joined {
                membership,
                knowledge,
            }) => {
                let mut inner = self.inner.lock().await;
                inner.record.group = Some(link.group);
                inner.record.membership = membership;
                inner.record.knowledge = knowledge;
                Self::remember(&mut inner.record, inviter);
                self.refold(&mut inner).await;
                let member = inner
                    .roster
                    .as_ref()
                    .and_then(|roster| roster.owner_of(&me))
                    .map(|member| (member.person, member.name.clone()));
                let Some((person, name)) = member else {
                    Self::forget_group(&mut inner.record);
                    self.refold(&mut inner).await;
                    drop(inner);
                    self.emit(GroupEvent::Pairing(PairingStep::Failed(
                        PairingFailure::Refused,
                    )))
                    .await;
                    self.emit(GroupEvent::Problem(GroupProblem::Other(
                        "The other computer didn't add this one. Ask for a new invite.".into(),
                    )))
                    .await;
                    return;
                };
                inner.record.person = Some(person);
                inner.record.person_name = name;
                inner.record.computer_name = computer_name;
                self.persist(&inner).await;
                drop(inner);
                self.emit(GroupEvent::Pairing(PairingStep::SkipCode)).await;
                self.emit(GroupEvent::Pairing(PairingStep::Admitted {
                    group: link.group,
                }))
                .await;
                self.publish_loaded().await;
            }
            Ok(PeerResponse::Refused { reason }) => {
                self.emit(GroupEvent::Pairing(PairingStep::Failed(
                    PairingFailure::Refused,
                )))
                .await;
                self.emit(GroupEvent::Problem(refusal_problem(reason)))
                    .await;
            }
            Ok(other) => {
                tracing::debug!(?other, "unexpected join answer");
                self.emit(GroupEvent::Pairing(PairingStep::Failed(
                    PairingFailure::Refused,
                )))
                .await;
                self.emit(GroupEvent::Problem(GroupProblem::BadInvite))
                    .await;
            }
            Err(error) => {
                tracing::info!(%error, "join request not delivered");
                self.emit(GroupEvent::Pairing(PairingStep::Failed(
                    PairingFailure::Unreachable,
                )))
                .await;
                self.emit(GroupEvent::Problem(GroupProblem::Unreachable))
                    .await;
            }
        }
    }

    /// Signs a membership change, keeps it only if the rules accept it, and sends it.
    async fn membership(self: &Arc<Self>, body: MembershipBody) {
        let mut inner = self.inner.lock().await;
        let Some(roster) = inner.roster.clone() else {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "Create or join a group first.".into(),
            )))
            .await;
            return;
        };
        let unsigned = roster.draft(self.me(), now(), body);
        let op = unsigned
            .clone()
            .sign(self.transport.sign(&unsigned.signing_bytes()));
        let mut trial_ops = inner.record.membership.clone();
        trial_ops.push(op.clone());
        let trial = fold(roster.group, &trial_ops, self.verifier.as_ref());
        if !trial.applied.contains(&op.hash()) {
            let rejection = trial
                .rejected
                .get(&op.hash())
                .copied()
                .unwrap_or(Rejection::NotAllowed);
            drop(inner);
            self.emit(GroupEvent::Problem(membership_problem(rejection)))
                .await;
            return;
        }
        let peers = self.peers(&inner);
        inner.record.membership.push(op.clone());
        self.refold(&mut inner).await;
        let left = inner
            .roster
            .as_ref()
            .is_some_and(|roster| !roster.admits(&self.me()));
        if left {
            Self::forget_group(&mut inner.record);
            self.refold(&mut inner).await;
        }
        self.persist(&inner).await;
        let roster_now = inner.roster.clone();
        let shared = Self::shared(&inner.view);
        drop(inner);
        self.broadcast(roster.group, peers, &[op], &[]);
        if left {
            self.publish_left().await;
        } else {
            self.publish_changed(roster_now, shared).await;
        }
    }

    /// Signs a knowledge change, keeps it only if the rules accept it, and sends it.
    async fn knowledge(self: &Arc<Self>, body: KnowledgeBody) {
        let checked = match &body {
            KnowledgeBody::Share { text, about, .. } => {
                check_share(text, about).map_err(|error| error.to_string())
            }
            KnowledgeBody::ShareArtifact {
                title,
                language,
                content,
                ..
            } => check_artifact(title, language, content)
                .map_err(|error| format!("That can't be shared: {error}.")),
            KnowledgeBody::Corroborate { .. }
            | KnowledgeBody::Withdraw { .. }
            | KnowledgeBody::Redact { .. } => Ok(()),
        };
        if let Err(message) = checked {
            self.emit(GroupEvent::Problem(GroupProblem::Other(message)))
                .await;
            return;
        }
        let mut inner = self.inner.lock().await;
        let Some(roster) = inner.roster.clone() else {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "Create or join a group first.".into(),
            )))
            .await;
            return;
        };
        let Some(unsigned) = inner.view.draft(self.me(), now(), body) else {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "Create or join a group first.".into(),
            )))
            .await;
            return;
        };
        let op = unsigned
            .clone()
            .sign(self.transport.sign(&unsigned.signing_bytes()));
        let mut trial_ops = inner.record.knowledge.clone();
        trial_ops.push(op.clone());
        let lane = LaneId::whole_group(&roster.group);
        let trial = fold_knowledge(&roster, lane, &trial_ops, self.verifier.as_ref());
        if let Some(rejection) = trial.rejected.get(&op.hash()) {
            drop(inner);
            self.emit(GroupEvent::Problem(knowledge_problem(*rejection)))
                .await;
            return;
        }
        let peers = self.peers(&inner);
        inner.record.knowledge.push(op.clone());
        self.refold(&mut inner).await;
        self.persist(&inner).await;
        let roster_now = inner.roster.clone();
        let shared = Self::shared(&inner.view);
        drop(inner);
        self.broadcast(roster.group, peers, &[], &[op]);
        self.publish_changed(roster_now, shared).await;
    }

    /// Admits a computer holding a live invite: signs its addition and hands it the history.
    async fn admit(
        self: &Arc<Self>,
        from: DeviceId,
        group: GroupId,
        proof: [u8; 32],
        person: PersonId,
        computer_name: String,
        address: PeerAddress,
    ) -> PeerResponse {
        let mut inner = self.inner.lock().await;
        let me = self.me();
        let invite = match inner
            .invites
            .redeem(&proof, &join_transcript(&me, &from, &group), now())
        {
            Ok(invite) => invite,
            Err(error) => {
                return PeerResponse::Refused {
                    reason: match error {
                        RedeemError::Unknown => Refusal::InviteUnknown,
                        RedeemError::Expired => Refusal::InviteExpired,
                        RedeemError::AlreadyUsed => Refusal::InviteUsed,
                    },
                };
            }
        };
        let Some(roster) = inner.roster.clone() else {
            return PeerResponse::Refused {
                reason: Refusal::WrongGroup,
            };
        };
        let Some(my_person) = roster.owner_of(&me).map(|member| member.person) else {
            return PeerResponse::Refused {
                reason: Refusal::NotAMember,
            };
        };
        let bodies = match invite.kind {
            InviteKind::OwnDevice => vec![MembershipBody::AddDevice {
                person: my_person,
                device: from,
                name: computer_name,
            }],
            InviteKind::Person { name, role } => vec![
                MembershipBody::AddPerson { person, name, role },
                MembershipBody::AddDevice {
                    person,
                    device: from,
                    name: computer_name,
                },
            ],
        };
        let peers: Vec<PeerAddress> = self.peers(&inner);
        let mut added = Vec::new();
        let mut state = roster;
        for body in bodies {
            let unsigned = state.draft(me, now(), body);
            let op = unsigned
                .clone()
                .sign(self.transport.sign(&unsigned.signing_bytes()));
            let mut trial_ops = inner.record.membership.clone();
            trial_ops.extend(added.iter().cloned());
            trial_ops.push(op.clone());
            let trial = fold(group, &trial_ops, self.verifier.as_ref());
            if !trial.applied.contains(&op.hash()) {
                tracing::info!(rejection = ?trial.rejected.get(&op.hash()), "invited computer could not be added");
                return PeerResponse::Refused {
                    reason: Refusal::NotAMember,
                };
            }
            state = trial;
            added.push(op);
        }
        inner.record.membership.extend(added.iter().cloned());
        Self::remember(&mut inner.record, address);
        self.refold(&mut inner).await;
        self.persist(&inner).await;
        // The joiner syncs straight after joining, so a history too large for one
        // message still arrives in full.
        let response = PeerResponse::Joined {
            membership: inner.record.membership.clone(),
            knowledge: within_budget(inner.record.knowledge.clone(), MAX_REPLY_KNOWLEDGE_BYTES).0,
        };
        let roster_now = inner.roster.clone();
        let shared = Self::shared(&inner.view);
        drop(inner);
        self.broadcast(
            group,
            peers
                .into_iter()
                .filter(|peer| peer.device != from)
                .collect(),
            &added,
            &[],
        );
        self.emit(GroupEvent::Pairing(PairingStep::PeerConnected))
            .await;
        self.emit(GroupEvent::Pairing(PairingStep::SkipCode)).await;
        self.emit(GroupEvent::Pairing(PairingStep::Admitted { group }))
            .await;
        self.publish_changed(roster_now, shared).await;
        response
    }
}

/// Serves other computers. Holds the shared handle because admitting a computer
/// broadcasts its addition on owned tasks.
struct GroupHandler(Arc<Groups>);

#[async_trait::async_trait]
impl PeerHandler for GroupHandler {
    async fn handle(&self, from: DeviceId, request: PeerRequest) -> PeerResponse {
        let groups = &self.0;
        let admitted = {
            let inner = groups.inner.lock().await;
            admits_request(inner.roster.as_ref(), &from, &request)
        };
        if let Err(reason) = admitted {
            return PeerResponse::Refused { reason };
        }
        match request {
            PeerRequest::Join {
                group,
                proof,
                person,
                computer_name,
                from: address,
                ..
            } => {
                groups
                    .admit(from, group, proof, person, computer_name, address)
                    .await
            }
            PeerRequest::Ask { question, .. } => groups.answer(from, &question).await,
            other => groups.serve_peer(other).await,
        }
    }
}

impl Groups {
    async fn serve_peer(&self, request: PeerRequest) -> PeerResponse {
        match request {
            PeerRequest::Sync {
                membership,
                knowledge,
                from: address,
                ..
            } => {
                let mut inner = self.inner.lock().await;
                Self::remember(&mut inner.record, address);
                let mine_membership: BTreeSet<_> = inner
                    .record
                    .membership
                    .iter()
                    .map(MembershipOp::hash)
                    .collect();
                let mine_knowledge: BTreeSet<_> = inner
                    .record
                    .knowledge
                    .iter()
                    .map(KnowledgeOp::hash)
                    .collect();
                PeerResponse::Synced {
                    membership: inner
                        .record
                        .membership
                        .iter()
                        .filter(|op| !membership.contains(&op.hash()))
                        .cloned()
                        .collect(),
                    knowledge: within_budget(
                        inner
                            .record
                            .knowledge
                            .iter()
                            .filter(|op| !knowledge.contains(&op.hash()))
                            .cloned()
                            .collect(),
                        MAX_REPLY_KNOWLEDGE_BYTES,
                    )
                    .0,
                    want_membership: membership.difference(&mine_membership).copied().collect(),
                    want_knowledge: knowledge.difference(&mine_knowledge).copied().collect(),
                }
            }
            PeerRequest::Push {
                membership,
                knowledge,
                ..
            } => {
                self.merge(membership, knowledge).await;
                PeerResponse::Accepted
            }
            PeerRequest::Join { .. } | PeerRequest::Ask { .. } => PeerResponse::Refused {
                reason: Refusal::Malformed,
            },
        }
    }

    /// Federated recall, answering side: searches this computer's own memory for a
    /// member's question, if the owner turned answering on, and logs what was asked.
    async fn answer(&self, from: DeviceId, question: &str) -> PeerResponse {
        let refused = PeerResponse::Refused {
            reason: Refusal::NotAnswering,
        };
        let Some(question) = check_ask(question) else {
            return PeerResponse::Refused {
                reason: Refusal::Malformed,
            };
        };
        let (owner, asked_by, computer) = {
            let mut inner = self.inner.lock().await;
            let Some(roster) = inner.roster.clone() else {
                return refused;
            };
            let reads = roster
                .role_of(&from)
                .is_some_and(sovra_core::group::Role::reads);
            if !inner.record.federated.answer || !reads || !inner.limiter.allow(from, now()) {
                return refused;
            }
            let me = self.me();
            let name_of = |device: &DeviceId| roster.owner_of(device).map(|m| m.name.clone());
            let computer = roster
                .owner_of(&from)
                .and_then(|member| member.devices.get(&from))
                .map(|entry| entry.name.clone())
                .unwrap_or_default();
            (
                name_of(&me).unwrap_or_default(),
                name_of(&from).unwrap_or_default(),
                computer,
            )
        };
        let found = match self.knowledge.local_reader() {
            Some(reader) => {
                let search = reader.search(sovra_core::MemorySearch {
                    text: question.clone(),
                    entity: None,
                    valid_at: None,
                    known_at: None,
                });
                match tokio::time::timeout(ASK_TIMEOUT, search).await {
                    Ok(Ok(context)) => context
                        .memories
                        .into_iter()
                        .map(|memory| (memory.statement, memory.learned_at))
                        .collect(),
                    Ok(Err(error)) => {
                        tracing::debug!(%error, "federated recall search failed");
                        Vec::new()
                    }
                    Err(_) => Vec::new(),
                }
            }
            // Remembering is paused or there is no memory: nothing to share.
            None => Vec::new(),
        };
        let items = federated_items(found, &owner);
        let (settings, recent) = {
            let mut inner = self.inner.lock().await;
            log_ask(
                &mut inner.asks,
                AskRecord {
                    asked_by,
                    computer,
                    question,
                    answered: items.len(),
                    at: now(),
                },
            );
            (inner.record.federated, inner.asks.clone())
        };
        self.emit(GroupEvent::Federated { settings, recent }).await;
        PeerResponse::Answered { items }
    }

    /// Federated recall, asking side: sends `question` to every other computer in the
    /// group at once and turns their answers into attributed evidence. Computers that
    /// do not answer are skipped for a while so recall never keeps waiting on them.
    async fn ask_group(&self, question: &str) -> Vec<sovra_core::GroupEvidence> {
        if !self.inner.lock().await.record.federated.ask {
            return Vec::new();
        }
        self.ask_peers(question, false)
            .await
            .into_iter()
            .enumerate()
            .map(|(index, (device, answer))| sovra_core::GroupEvidence {
                citation: format!("group-ask:{}:{index}", &device.to_hex()[..12]),
                statement: answer.statement,
                shared_by: answer.from,
                computer: answer.computer,
                shared_at: answer.recorded_at,
                valid_at: None,
                disputed: false,
                agreed_by: 0,
            })
            .collect()
    }

    /// Asks every other computer in the group at once. Used by recall (when asking is
    /// on) and by "Ask my group", which is itself the person's say-so for that question.
    /// `explicit` (the person pressed Ask) asks even computers on a cooldown.
    async fn ask_peers(
        &self,
        question: &str,
        explicit: bool,
    ) -> Vec<(DeviceId, sovra_core::group::GroupAnswer)> {
        let Some(question) = check_ask(question) else {
            return Vec::new();
        };
        let (group, peers) = {
            let inner = self.inner.lock().await;
            let Some(roster) = inner.roster.clone() else {
                return Vec::new();
            };
            let at = now();
            let peers: Vec<(PeerAddress, String, String)> = self
                .peers(&inner)
                .into_iter()
                .filter(|peer| {
                    explicit
                        || inner
                            .quiet_until
                            .get(&peer.device)
                            .is_none_or(|until| *until <= at)
                })
                .map(|peer| {
                    let member = roster.owner_of(&peer.device);
                    let person = member.map(|m| m.name.clone()).unwrap_or_default();
                    let computer = member
                        .and_then(|m| m.devices.get(&peer.device))
                        .map(|entry| entry.name.clone())
                        .unwrap_or_default();
                    (peer, person, computer)
                })
                .collect();
            (roster.group, peers)
        };
        let asks = peers.into_iter().map(|(peer, person, computer)| {
            let request = PeerRequest::Ask {
                group,
                question: question.clone(),
            };
            async move {
                let answer =
                    tokio::time::timeout(ASK_TIMEOUT, self.transport.request(&peer, request)).await;
                (peer.device, person, computer, answer)
            }
        });
        let answers = futures::future::join_all(asks).await;
        let mut evidence = Vec::new();
        let mut quiet = Vec::new();
        let mut answered = Vec::new();
        for (device, person, computer, answer) in answers {
            match answer {
                Ok(Ok(PeerResponse::Answered { items })) => {
                    answered.push(device);
                    for item in items {
                        evidence.push((
                            device,
                            sovra_core::group::GroupAnswer {
                                statement: item.statement,
                                from: person.clone(),
                                computer: computer.clone(),
                                recorded_at: item.recorded_at,
                            },
                        ));
                    }
                }
                _ => quiet.push(device),
            }
        }
        if !quiet.is_empty() || !answered.is_empty() {
            let until = Timestamp(now().0.saturating_add(ASK_COOLDOWN_MS));
            let mut inner = self.inner.lock().await;
            for device in quiet {
                inner.quiet_until.insert(device, until);
            }
            for device in answered {
                inner.quiet_until.remove(&device);
            }
        }
        evidence
    }
}

fn membership_problem(rejection: Rejection) -> GroupProblem {
    match rejection {
        Rejection::NotAllowed => GroupProblem::NotAllowed,
        Rejection::WouldLeaveNoAdmin => GroupProblem::LastAdmin,
        Rejection::InvalidName => GroupProblem::Other(format!(
            "Names need 1 to {} characters.",
            sovra_core::group::MAX_NAME_CHARS
        )),
        Rejection::AlreadyPresent => GroupProblem::Other("That's already in the group.".into()),
        Rejection::UnknownTarget => {
            GroupProblem::Other("That person or computer isn't in the group any more.".into())
        }
        other => GroupProblem::Other(format!("That change wasn't accepted ({other:?}).")),
    }
}

fn knowledge_problem(rejection: KnowledgeRejection) -> GroupProblem {
    match rejection {
        KnowledgeRejection::NotAllowed => {
            GroupProblem::Other("Only the person who shared it, or an admin, can do that.".into())
        }
        KnowledgeRejection::UnknownDetail => {
            GroupProblem::Other("That shared detail isn't on this computer yet.".into())
        }
        KnowledgeRejection::InvalidDetail => {
            GroupProblem::Other("That detail is empty or too long.".into())
        }
        other => GroupProblem::Other(format!("That wasn't shared ({other:?}).")),
    }
}

fn refusal_problem(reason: Refusal) -> GroupProblem {
    match reason {
        Refusal::InviteExpired => GroupProblem::InviteExpired,
        Refusal::InviteUsed => GroupProblem::InviteUsed,
        Refusal::InviteUnknown | Refusal::WrongGroup | Refusal::Malformed => {
            GroupProblem::BadInvite
        }
        Refusal::NotAMember => GroupProblem::NotAllowed,
        Refusal::NotAnswering => {
            GroupProblem::Other("That computer isn't answering questions from the group.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use sha2::{Digest as _, Sha256};
    use sovra_core::group::{KnowledgeView, UnsignedKnowledge};

    use super::*;

    fn signature(device: &DeviceId, message: &[u8]) -> sovra_core::group::Signature {
        let first: [u8; 32] =
            Sha256::digest([b"sig".as_slice(), device.as_bytes(), message].concat()).into();
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&first);
        bytes[32..].copy_from_slice(&first);
        sovra_core::group::Signature(bytes)
    }

    struct Fake;
    impl SignatureVerifier for Fake {
        fn verify(
            &self,
            signer: &DeviceId,
            message: &[u8],
            signature_bytes: &sovra_core::group::Signature,
        ) -> bool {
            signature(signer, message) == *signature_bytes
        }
    }

    /// Mum's group with one detail Dad shared.
    fn view_with_dads_detail() -> KnowledgeView {
        let (mum, dad) = (DeviceId([1; 32]), DeviceId([2; 32]));
        let nonce = [3; 16];
        let group = GroupId::derive(&mum, &nonce);
        let mut ops = Vec::new();
        let genesis = UnsignedOp {
            group,
            author: mum,
            seq: 1,
            prev: None,
            deps: Vec::new(),
            at: Timestamp(1),
            body: MembershipBody::Genesis {
                name: "Family".into(),
                nonce,
                founder: PersonId([10; 32]),
                founder_name: "Mum".into(),
                device_name: "Laptop".into(),
            },
        };
        ops.push(
            genesis
                .clone()
                .sign(signature(&mum, &genesis.signing_bytes())),
        );
        for body in [
            MembershipBody::AddPerson {
                person: PersonId([20; 32]),
                name: "Dad".into(),
                role: sovra_core::group::Role::Member,
            },
            MembershipBody::AddDevice {
                person: PersonId([20; 32]),
                device: dad,
                name: "Desktop".into(),
            },
        ] {
            let draft = fold(group, &ops, &Fake).draft(mum, Timestamp(2), body);
            ops.push(draft.clone().sign(signature(&mum, &draft.signing_bytes())));
        }
        let roster = fold(group, &ops, &Fake);
        let lane = LaneId::whole_group(&group);
        let share: UnsignedKnowledge = fold_knowledge(&roster, lane, &[], &Fake)
            .draft(
                dad,
                Timestamp(3),
                KnowledgeBody::Share {
                    text: "The boiler service is booked for 12 March".into(),
                    about: Vec::new(),
                    valid_at: None,
                    replaces: Vec::new(),
                },
            )
            .expect("lane");
        let op = share.clone().sign(signature(&dad, &share.signing_bytes()));
        let list = fold_knowledge(&roster, lane, std::slice::from_ref(&op), &Fake)
            .draft(
                dad,
                Timestamp(4),
                KnowledgeBody::ShareArtifact {
                    title: "Shopping list".into(),
                    kind: sovra_core::artifacts::ArtifactKind::Checklist,
                    language: String::new(),
                    content: "- oat milk\n- eggs\n- bread".into(),
                    replaces: Vec::new(),
                    origin: String::new(),
                },
            )
            .expect("lane");
        let list = list.clone().sign(signature(&dad, &list.signing_bytes()));
        fold_knowledge(&roster, lane, &[op, list], &Fake)
    }

    struct Empty;
    #[async_trait::async_trait]
    impl sovra_core::MemoryReader for Empty {
        async fn search(
            &self,
            _query: sovra_core::MemorySearch,
        ) -> Result<sovra_core::MemoryContext, sovra_core::MemoryError> {
            Ok(sovra_core::MemoryContext::default())
        }
    }

    #[tokio::test]
    async fn a_helpers_recall_carries_shared_group_details_with_who_shared_them() {
        let knowledge = GroupKnowledge::default();
        *knowledge.0.write().await = view_with_dads_detail();
        let reader = knowledge
            .wrap(Some(Arc::new(Empty)))
            .expect("wraps an existing reader");
        let context = reader
            .search(sovra_core::MemorySearch {
                text: "when is the boiler serviced".into(),
                entity: None,
                valid_at: None,
                known_at: None,
            })
            .await
            .expect("searches");
        assert_eq!(context.group.len(), 1);
        let evidence = &context.group[0];
        assert_eq!(evidence.shared_by, "Dad");
        assert_eq!(evidence.computer, "Desktop");
        assert!(evidence.statement.contains("12 March"));
        assert!(evidence.citation.starts_with("group:"));
        assert!(!evidence.disputed);

        let context = reader
            .search(sovra_core::MemorySearch {
                text: "what is on our shopping list".into(),
                entity: None,
                valid_at: None,
                known_at: None,
            })
            .await
            .expect("searches");
        let list = context
            .group
            .iter()
            .find(|evidence| evidence.citation.starts_with("group-artifact:"))
            .expect("the shared list is recalled");
        assert_eq!(list.shared_by, "Dad");
        assert!(list.statement.contains("Shopping list") && list.statement.contains("oat milk"));

        assert!(
            knowledge.wrap(None).is_none(),
            "no personal reader (untrusted sender, remembering paused) means no group reader"
        );
    }
}
