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
use sovra_core::group::check_template;
use sovra_core::group::{
    AskLimiter, AskRecord, Bytes, GroupCipher, OpHash, SharedHit, Space, check_ask,
    federated_items, fold_space, lanes_of, log_ask, sealed_aad, spaces_of,
};
use sovra_core::group::{
    DeviceId, GroupCommand, GroupEvent, GroupId, GroupProblem, GroupRecord, GroupState, GroupStore,
    GroupTransport, Invite, InviteBook, InviteKind, JoinLink, KnowledgeBody, KnowledgeOp,
    KnowledgeRejection, KnowledgeView, LinkError, MAX_REPLY_KNOWLEDGE_BYTES, MembershipBody,
    MembershipOp, PairingFailure, PairingStep, PeerAddress, PeerHandler, PeerRequest, PeerResponse,
    PersonId, RedeemError, Refusal, Rejection, SharedArtifact, SharedDetail, SignatureVerifier,
    Spawn, UnsignedOp, admits_request, check_artifact, check_share, fold, invite_proof,
    join_transcript, within_budget,
};
use sovra_core::group::{
    SHORT_CODE_LIFETIME_MS, ShortCodeWindow, code_hint, code_transcript, format_code,
    normalise_code, short_auth_string, short_code_secret,
};
use sovra_core::{Event, Timestamp};
use tokio::sync::{Mutex, RwLock};

/// Builds the transport once the runtime's reactor is running (iroh needs it).
pub type TransportFuture =
    Pin<Box<dyn Future<Output = Result<Arc<dyn GroupTransport>, String>> + Send>>;
pub type TransportFactory = Box<dyn FnOnce() -> TransportFuture + Send>;

/// Draws text as a scannable code (rows of dark modules). Supplied by the composition
/// root from the adapter that owns the QR library.
pub type QrEncoder = Arc<dyn Fn(&str) -> Vec<Vec<bool>> + Send + Sync>;

/// The adapters groups need, supplied by the composition root.
pub struct GroupServices {
    pub transport: TransportFactory,
    pub store: Arc<dyn GroupStore>,
    /// Turns invite links into QR codes; `None` shows the link only.
    pub qr: Option<QrEncoder>,
    /// A fixed AI to answer group members' work with (an always-on helper box, or a
    /// test). `None` answers with whichever AI this computer has selected.
    pub helper_model: Option<Arc<dyn sovra_core::group::WorkModel>>,
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
    templates: Vec<sovra_core::group::SharedTemplate>,
}

/// Opens this computer's own recall when another computer asks (federated recall).
/// A fresh reader each time, so pausing remembering takes effect at once.
pub(crate) type LocalMemory =
    Arc<dyn Fn() -> Option<Arc<dyn sovra_core::MemoryReader>> + Send + Sync>;

/// This computer's currently selected AI, for answering group members' chat jobs
/// (work handoff). Looked up per job so switching models takes effect at once.
pub(crate) type LocalModel = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Option<Arc<dyn sovra_core::group::WorkModel>>> + Send>>
        + Send
        + Sync,
>;

/// What helpers may read: the group's usable shared details, plus the way to ask the
/// group's computers (federated recall) and to answer them from this computer's memory.
#[derive(Clone, Default)]
pub(crate) struct GroupKnowledge(
    /// One view per space this computer's person is in; the whole group first.
    Arc<RwLock<Vec<KnowledgeView>>>,
    Arc<std::sync::RwLock<Option<LocalMemory>>>,
    Arc<std::sync::RwLock<Option<std::sync::Weak<Groups>>>>,
    Arc<std::sync::RwLock<Option<LocalModel>>>,
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
            let views = self.0.read().await;
            let mut hits: Vec<SharedHit> = views
                .iter()
                .flat_map(|view| view.search(question, sovra_core::MAX_GROUP_EVIDENCE_ITEMS))
                .collect();
            hits.sort_by(|a, b| b.score.total_cmp(&a.score));
            (
                hits.iter()
                    .map(|hit| evidence(&hit.detail))
                    .collect::<Vec<_>>(),
                views
                    .iter()
                    .flat_map(|view| view.search_artifacts(question, ARTIFACT_EVIDENCE_ITEMS))
                    .take(ARTIFACT_EVIDENCE_ITEMS)
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
        if let Some(model) = services
            .as_ref()
            .and_then(|services| services.helper_model.clone())
        {
            let open: LocalModel = Arc::new(move || {
                let model = model.clone();
                Box::pin(async move { Some(model) })
            });
            *knowledge
                .3
                .write()
                .expect("local model lock is never poisoned") = Some(open);
        }
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
                    services.qr,
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

    /// Lets group members hand chat work to this computer's selected AI (when the
    /// owner allows), unless the composition root gave a fixed helper AI.
    pub(crate) fn attach_model_unless_given(&self, open: LocalModel) {
        let mut slot = self
            .knowledge
            .3
            .write()
            .expect("local model lock is never poisoned");
        if slot.is_none() {
            *slot = Some(open);
        }
    }

    /// A way to hand a chat turn to a free computer in the group, when this computer's
    /// owner turned that on and it is in a group.
    pub(crate) async fn work_peer(&self) -> Option<Arc<dyn sovra_core::group::WorkPeer>> {
        let groups = self.knowledge.groups()?;
        let wanted = {
            let inner = groups.inner.lock().await;
            inner.record.work.use_group && inner.roster.is_some()
        };
        wanted.then(|| Arc::new(GroupWork(groups)) as Arc<dyn sovra_core::group::WorkPeer>)
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

/// How long a helping computer may take to answer one chat job.
const JOB_TIMEOUT: Duration = Duration::from_secs(120);

/// Asking side of work handoff: tries the group's other computers, most recently
/// heard from first, until one answers.
struct GroupWork(Arc<Groups>);

#[async_trait::async_trait]
impl sovra_core::group::WorkPeer for GroupWork {
    async fn run(
        &self,
        job: sovra_core::group::ChatJob,
    ) -> Result<sovra_core::group::JobAnswer, sovra_core::group::WorkError> {
        sovra_core::group::check_job(&job)?;
        let groups = &self.0;
        let (group, candidates) = {
            let inner = groups.inner.lock().await;
            let Some(roster) = inner.roster.clone() else {
                return Err(sovra_core::group::WorkError::NobodyFree);
            };
            let order = sovra_core::group::helper_order(
                groups.peers(&inner).into_iter().map(|peer| peer.device),
                &inner.last_seen,
                &groups.me(),
            );
            let candidates: Vec<(PeerAddress, String, String)> = order
                .into_iter()
                .map(|device| {
                    let member = roster.owner_of(&device);
                    (
                        Groups::address_of(&inner.record, device),
                        member.map(|m| m.name.clone()).unwrap_or_default(),
                        member
                            .and_then(|m| m.devices.get(&device))
                            .map(|entry| entry.name.clone())
                            .unwrap_or_default(),
                    )
                })
                .collect();
            (roster.group, candidates)
        };
        for (peer, helper, computer) in candidates {
            let request = PeerRequest::Work {
                group,
                job: job.clone(),
            };
            let answer = tokio::time::timeout(
                JOB_TIMEOUT + REQUEST_TIMEOUT,
                groups.transport.request(&peer, request),
            )
            .await;
            if let Ok(Ok(PeerResponse::Worked { text })) = answer {
                groups.saw(peer.device).await;
                return Ok(sovra_core::group::JobAnswer {
                    text,
                    helper,
                    computer,
                });
            }
        }
        Err(sovra_core::group::WorkError::NobodyFree)
    }
}

/// A receiver for the inviting person's picture answer, if a join is pending.
fn pending_answer(
    pending: Option<&PendingPair>,
) -> Option<tokio::sync::watch::Receiver<Option<bool>>> {
    pending.map(|pending| pending.answer.subscribe())
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
    /// The whole group's lane.
    view: KnowledgeView,
    /// Two-person spaces this computer's person is in.
    pairs: std::collections::BTreeMap<Space, KnowledgeView>,
    invites: InviteBook,
    /// Questions other computers asked this one, newest first (not saved).
    asks: Vec<AskRecord>,
    limiter: AskLimiter,
    /// When each other computer was last heard from (any successful exchange).
    last_seen: std::collections::BTreeMap<DeviceId, Timestamp>,
    /// Computers that recently did not answer, and until when they are skipped.
    quiet_until: std::collections::BTreeMap<DeviceId, Timestamp>,
    /// Chat jobs this computer answered for others, newest first (not saved).
    work_log: Vec<sovra_core::group::WorkRecord>,
    work_limiter: AskLimiter,
    /// Lane keys this computer holds (opened from its grants, or made here).
    lane_keys: std::collections::BTreeMap<OpHash, [u8; 32]>,
    /// Plaintext of sealed operations already opened, by operation hash.
    opened: std::collections::BTreeMap<OpHash, KnowledgeBody>,
    /// The same-room code showing here, if any.
    short: Option<ShortInvite>,
    /// A same-room join this computer invited, waiting for both people's answers.
    pending_pair: Option<PendingPair>,
    /// A same-room join this computer started, waiting for its person's answer.
    joining: Option<CodeJoin>,
}

/// A same-room code: its secret, its one-guess window and what it invites.
struct ShortInvite {
    secret: [u8; 16],
    window: ShortCodeWindow,
    kind: InviteKind,
}

/// Inviting side of a same-room join, between "code was right" and "both matched".
struct PendingPair {
    joiner: DeviceId,
    person: PersonId,
    computer_name: String,
    address: PeerAddress,
    kind: InviteKind,
    /// This computer's person's answer, which the joiner's request waits for.
    answer: tokio::sync::watch::Sender<Option<bool>>,
}

/// Joining side of a same-room join.
struct CodeJoin {
    inviter: PeerAddress,
    group: GroupId,
    computer_name: String,
}

/// How long a joining computer listens for a nearby code.
const NEARBY_LISTEN_MS: u64 = 4_000;
/// How long either side waits for the other person to compare pictures.
const PICTURE_CHECK: Duration = Duration::from_millis(SHORT_CODE_LIFETIME_MS.unsigned_abs());

struct Groups {
    qr: Option<QrEncoder>,
    transport: Arc<dyn GroupTransport>,
    verifier: Arc<dyn SignatureVerifier + Send + Sync>,
    cipher: Arc<dyn GroupCipher>,
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
        qr: Option<QrEncoder>,
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
        let cipher = transport.cipher();
        let groups = Self {
            qr,
            transport,
            verifier,
            cipher,
            store,
            events,
            knowledge,
            spawn,
            inner: Mutex::new(Inner {
                record,
                roster: None,
                view: KnowledgeView::default(),
                pairs: std::collections::BTreeMap::new(),
                invites: InviteBook::default(),
                asks: Vec::new(),
                limiter: AskLimiter::default(),
                quiet_until: std::collections::BTreeMap::new(),
                last_seen: std::collections::BTreeMap::new(),
                work_log: Vec::new(),
                work_limiter: AskLimiter::default(),
                lane_keys: std::collections::BTreeMap::new(),
                opened: std::collections::BTreeMap::new(),
                short: None,
                pending_pair: None,
                joining: None,
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
        if let Some(roster) = inner.roster.clone() {
            let me = roster.owner_of(&self.me()).map(|member| member.person);
            let spaces = me.map_or_else(|| vec![Space::Group], |me| spaces_of(&roster, &me));
            let mut views = self.fold_and_open(inner, &roster, &spaces);
            inner.view = views.remove(&Space::Group).unwrap_or_default();
            inner.pairs = views;
        } else {
            inner.lane_keys.clear();
            inner.opened.clear();
            inner.view = KnowledgeView::default();
            inner.pairs.clear();
        }
        let mut all = vec![inner.view.clone()];
        all.extend(inner.pairs.values().cloned());
        *self.knowledge.0.write().await = all;
    }

    /// Folds each space, opens any lane keys granted to this computer and any sealed
    /// operations it can now read, and folds again when that revealed something.
    fn fold_and_open(
        &self,
        inner: &mut Inner,
        roster: &GroupState,
        spaces: &[Space],
    ) -> std::collections::BTreeMap<Space, KnowledgeView> {
        let verifier = self.verifier.as_ref();
        let fold_all = |inner: &Inner| {
            spaces
                .iter()
                .map(|space| {
                    (
                        *space,
                        fold_space(
                            roster,
                            *space,
                            &inner.record.knowledge,
                            verifier,
                            &inner.opened,
                        ),
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let views = fold_all(inner);
        let me = self.me();
        for view in views.values() {
            for (id, key) in &view.keys {
                if inner.lane_keys.contains_key(id) {
                    continue;
                }
                if let Some(sealed) = key.grants.get(&me)
                    && let Some(opened) = self.cipher.open_sealed(&sealed.0)
                    && let Ok(bytes) = <[u8; 32]>::try_from(opened.as_slice())
                {
                    inner.lane_keys.insert(*id, bytes);
                }
            }
        }
        let mut revealed = false;
        for op in &inner.record.knowledge {
            let KnowledgeBody::Sealed { key, ciphertext } = &op.unsigned().body else {
                continue;
            };
            let hash = op.hash();
            if inner.opened.contains_key(&hash) {
                continue;
            }
            let Some(lane_key) = inner.lane_keys.get(key) else {
                continue;
            };
            let unsigned = op.unsigned();
            let aad = sealed_aad(&roster.group, &unsigned.lane, key, &unsigned.author);
            if let Some(plaintext) = self.cipher.decrypt(lane_key, &aad, &ciphertext.0)
                && let Ok(body) = serde_json::from_slice::<KnowledgeBody>(&plaintext)
            {
                inner.opened.insert(hash, body);
                revealed = true;
            }
        }
        if revealed { fold_all(inner) } else { views }
    }

    /// The folded view of `space` (none when this computer is not in it).
    fn view_of<'a>(inner: &'a Inner, space: &Space) -> Option<&'a KnowledgeView> {
        match space {
            Space::Group => Some(&inner.view),
            Space::Pair { .. } => inner.pairs.get(space),
        }
    }

    /// The space a shared detail or artifact lives in, among those this computer has.
    fn space_holding(inner: &Inner, id: &OpHash) -> Option<Space> {
        std::iter::once(&inner.view)
            .chain(inner.pairs.values())
            .find(|view| view.details.contains_key(id) || view.artifacts.contains_key(id))
            .map(|view| view.space)
    }

    /// This computer's next knowledge operation, signed, on top of what it has folded.
    fn sign_next(&self, inner: &Inner, space: &Space, body: KnowledgeBody) -> Option<KnowledgeOp> {
        let unsigned = Self::view_of(inner, space)?.draft(self.me(), now(), body)?;
        Some(
            unsigned
                .clone()
                .sign(self.transport.sign(&unsigned.signing_bytes())),
        )
    }

    /// A grant of lane key `id` sealed to each of `devices` (skipping any that fail).
    fn grant_body(&self, id: OpHash, key: &[u8; 32], devices: &[DeviceId]) -> KnowledgeBody {
        KnowledgeBody::KeyGrant {
            key: id,
            grants: devices
                .iter()
                .filter_map(|device| {
                    self.cipher
                        .seal_for(device, key)
                        .map(|sealed| (*device, Bytes(sealed)))
                })
                .collect(),
        }
    }

    /// Makes sure this computer holds a lane key that every current computer has and
    /// no removed computer ever had, making a fresh one when needed (the rotation).
    /// Returns the key and any grant operations it had to add to the record.
    async fn ensure_key(
        &self,
        inner: &mut Inner,
        roster: &GroupState,
        space: &Space,
    ) -> Option<(OpHash, [u8; 32], Vec<KnowledgeOp>)> {
        let usable = Self::view_of(inner, space)?
            .usable_key(roster)
            .filter(|id| inner.lane_keys.contains_key(id));
        let (id, key) = if let Some(id) = usable {
            (id, inner.lane_keys[&id])
        } else {
            let key = self.cipher.random_key()?;
            let id = OpHash(self.cipher.random_key()?);
            inner.lane_keys.insert(id, key);
            (id, key)
        };
        let holders: BTreeSet<DeviceId> = Self::view_of(inner, space)?
            .keys
            .get(&id)
            .map(|lane_key| lane_key.grants.keys().copied().collect())
            .unwrap_or_default();
        let missing: Vec<DeviceId> = space
            .audience(roster)
            .difference(&holders)
            .copied()
            .collect();
        let mut added = Vec::new();
        if !missing.is_empty() {
            let op = self.sign_next(inner, space, self.grant_body(id, &key, &missing))?;
            inner.record.knowledge.push(op.clone());
            self.refold(inner).await;
            added.push(op);
        }
        Some((id, key, added))
    }

    async fn persist(&self, inner: &Inner) {
        if let Err(error) = self.store.save(&inner.record).await {
            tracing::warn!(%error, "group record not saved");
            self.emit(GroupEvent::Problem(GroupProblem::Other(error.to_string())))
                .await;
        }
    }

    /// Everything shared in every space this computer is in, newest first.
    fn shared(inner: &Inner) -> SharedNow {
        let views: Vec<&KnowledgeView> = std::iter::once(&inner.view)
            .chain(inner.pairs.values())
            .collect();
        let mut details: Vec<SharedDetail> = views
            .iter()
            .flat_map(|view| view.details.values().cloned())
            .collect();
        details.sort_by(|a, b| b.shared_at.cmp(&a.shared_at).then(b.id.cmp(&a.id)));
        let mut artifacts: Vec<SharedArtifact> = views
            .iter()
            .flat_map(|view| view.shared_artifacts())
            .collect();
        artifacts.sort_by(|a, b| {
            b.is_latest()
                .cmp(&a.is_latest())
                .then(b.shared_at.cmp(&a.shared_at))
                .then(b.id.cmp(&a.id))
        });
        let mut templates: Vec<sovra_core::group::SharedTemplate> = views
            .iter()
            .flat_map(|view| view.shared_templates())
            .collect();
        templates.sort_by(|a, b| {
            b.is_latest()
                .cmp(&a.is_latest())
                .then(b.shared_at.cmp(&a.shared_at))
                .then(b.id.cmp(&a.id))
        });
        SharedNow {
            details,
            artifacts,
            templates,
        }
    }

    async fn publish_shared(&self, shared: SharedNow) {
        self.emit(GroupEvent::Knowledge(shared.details)).await;
        self.emit(GroupEvent::Artifacts(shared.artifacts)).await;
        self.emit(GroupEvent::Templates(shared.templates)).await;
    }

    async fn publish_loaded(&self) {
        let (roster, shared, settings, recent) = {
            let inner = self.inner.lock().await;
            (
                inner.roster.clone(),
                Self::shared(&inner),
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
        self.publish_work().await;
    }

    async fn publish_work(&self) {
        let (settings, recent) = {
            let inner = self.inner.lock().await;
            (inner.record.work, inner.work_log.clone())
        };
        self.emit(GroupEvent::Work { settings, recent }).await;
    }

    /// Helping side: answers a group member's chat job with this computer's AI, if
    /// the owner turned helping on. No tools; logged for the owner.
    async fn work(&self, from: DeviceId, job: sovra_core::group::ChatJob) -> PeerResponse {
        let refused = PeerResponse::Refused {
            reason: Refusal::NotHelping,
        };
        if sovra_core::group::check_job(&job).is_err() {
            return PeerResponse::Refused {
                reason: Refusal::Malformed,
            };
        }
        let (asked_by, computer) = {
            let mut inner = self.inner.lock().await;
            let Some(roster) = inner.roster.clone() else {
                return refused;
            };
            if !inner.record.work.help_others
                || !roster.admits(&from)
                || !inner
                    .work_limiter
                    .allow_up_to(from, now(), sovra_core::group::JOBS_PER_MINUTE)
            {
                return refused;
            }
            let member = roster.owner_of(&from);
            (
                member.map(|m| m.name.clone()).unwrap_or_default(),
                member
                    .and_then(|m| m.devices.get(&from))
                    .map(|entry| entry.name.clone())
                    .unwrap_or_default(),
            )
        };
        let open = self
            .knowledge
            .3
            .read()
            .expect("local model lock is never poisoned")
            .clone();
        let model = match open {
            Some(open) => open().await,
            None => None,
        };
        let answer = match model {
            Some(model) => tokio::time::timeout(JOB_TIMEOUT, model.answer(job))
                .await
                .ok()
                .and_then(Result::ok),
            None => None,
        };
        {
            let mut inner = self.inner.lock().await;
            sovra_core::group::log_work(
                &mut inner.work_log,
                sovra_core::group::WorkRecord {
                    asked_by,
                    computer,
                    at: now(),
                    answered: answer.is_some(),
                },
            );
        }
        self.publish_work().await;
        match answer {
            Some(text) => PeerResponse::Worked { text },
            None => refused,
        }
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
            templates: Vec::new(),
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

    /// Notes that `device` just answered or spoke to this computer.
    async fn saw(&self, device: DeviceId) {
        self.inner.lock().await.last_seen.insert(device, now());
    }

    async fn publish_presence(&self) {
        let seen = self.inner.lock().await.last_seen.clone();
        self.emit(GroupEvent::Presence(seen)).await;
    }

    async fn ask(&self, to: &PeerAddress, request: PeerRequest) -> Result<PeerResponse, String> {
        match tokio::time::timeout(REQUEST_TIMEOUT, self.transport.request(to, request)).await {
            Ok(Ok(response)) => {
                self.saw(to.device).await;
                Ok(response)
            }
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
            let membership = membership.to_vec();
            let knowledge = knowledge.to_vec();
            (self.spawn)(Box::pin(async move {
                // A two-person space's history only ever goes to that pair's computers.
                let knowledge = groups.for_device(knowledge, &peer.device).await;
                if membership.is_empty() && knowledge.is_empty() {
                    return;
                }
                let request = PeerRequest::Push {
                    group,
                    membership,
                    knowledge,
                };
                if let Err(error) = groups.ask(&peer, request).await {
                    tracing::debug!(%error, peer = %peer.device.fingerprint(), "push not delivered; sync will retry");
                }
            }));
        }
    }

    /// Only the operations whose space `device` belongs to.
    async fn for_device(&self, ops: Vec<KnowledgeOp>, device: &DeviceId) -> Vec<KnowledgeOp> {
        let inner = self.inner.lock().await;
        let Some(roster) = inner.roster.as_ref() else {
            return Vec::new();
        };
        Self::reachable(roster, ops, device)
    }

    fn reachable(
        roster: &GroupState,
        ops: Vec<KnowledgeOp>,
        device: &DeviceId,
    ) -> Vec<KnowledgeOp> {
        let lanes = lanes_of(roster);
        ops.into_iter()
            .filter(|op| {
                lanes
                    .get(&op.unsigned().lane)
                    .is_some_and(|space| space.reaches(roster, device))
            })
            .collect()
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
        // Keep only spaces this computer belongs to (judged with the roster as it now
        // stands, so a person added in the same batch counts).
        let roster = inner
            .record
            .group
            .map(|group| fold(group, &inner.record.membership, self.verifier.as_ref()));
        let knowledge = match &roster {
            Some(roster) => Self::reachable(roster, knowledge, &self.me()),
            None => Vec::new(),
        };
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
        let shared = Self::shared(&inner);
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
        self.publish_presence().await;
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
            GroupCommand::ShowShortCode { kind } => self.show_short_code(kind).await,
            GroupCommand::JoinWithCode {
                code,
                computer_name,
            } => {
                // Listening for nearby computers takes seconds; keep commands flowing.
                let groups = self.clone();
                (self.spawn)(Box::pin(async move {
                    groups.join_with_code(&code, computer_name).await;
                }));
            }
            GroupCommand::ConfirmCode { matches } => self.confirm_code(matches).await,
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
            GroupCommand::Share { text, about, with } => {
                let space = self.space_with(with).await;
                self.knowledge(
                    space,
                    KnowledgeBody::Share {
                        text,
                        about,
                        valid_at: None,
                        replaces: Vec::new(),
                    },
                )
                .await;
            }
            GroupCommand::Correct { details, text } => {
                let space = self.space_of(details.first()).await;
                self.knowledge(
                    space,
                    KnowledgeBody::Share {
                        text,
                        about: Vec::new(),
                        valid_at: None,
                        replaces: details,
                    },
                )
                .await;
            }
            GroupCommand::Agree { detail } => {
                let space = self.space_of(Some(&detail)).await;
                self.knowledge(space, KnowledgeBody::Corroborate { detail })
                    .await;
            }
            GroupCommand::Withdraw { detail } => {
                let space = self.space_of(Some(&detail)).await;
                self.knowledge(space, KnowledgeBody::Withdraw { detail })
                    .await;
            }
            GroupCommand::Erase { detail } => {
                let space = self.space_of(Some(&detail)).await;
                self.knowledge(space, KnowledgeBody::Redact { detail })
                    .await;
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
            GroupCommand::ShareTemplate { template, with } => {
                let space = self.space_with(with).await;
                self.knowledge(
                    space,
                    KnowledgeBody::ShareTemplate {
                        template,
                        replaces: Vec::new(),
                    },
                )
                .await;
            }
            GroupCommand::SetWork(settings) => {
                {
                    let mut inner = self.inner.lock().await;
                    inner.record.work = settings;
                    self.persist(&inner).await;
                }
                self.publish_work().await;
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
                with,
            } => {
                // A new version stays in its original's space.
                let (space, replaces) = match new_version_of {
                    Some(original) => {
                        let inner = self.inner.lock().await;
                        match Self::space_holding(&inner, &original) {
                            Some(space) => (
                                space,
                                Self::view_of(&inner, &space)
                                    .map(|view| view.latest_versions_of(&original))
                                    .unwrap_or_default(),
                            ),
                            None => (Space::Group, Vec::new()),
                        }
                    }
                    None => (self.space_with(with).await, Vec::new()),
                };
                self.knowledge(
                    space,
                    KnowledgeBody::ShareArtifact {
                        title,
                        kind,
                        language,
                        content,
                        replaces,
                        origin: local_id,
                    },
                )
                .await;
            }
        }
    }

    /// Shows a six-digit code and tells computers on this network a code is showing.
    async fn show_short_code(self: &Arc<Self>, kind: InviteKind) {
        let mut inner = self.inner.lock().await;
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
                .role_of(&self.me())
                .is_some_and(sovra_core::group::Role::manages)
        {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::NotAllowed))
                .await;
            return;
        }
        let Some(random) = random::<4>() else {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "This computer couldn't make a secure code. Try again.".into(),
            )))
            .await;
            return;
        };
        let (digits, shown) = format_code(u32::from_le_bytes(random));
        let window = ShortCodeWindow::open(now());
        let expires_at = window.expires_at;
        inner.short = Some(ShortInvite {
            secret: short_code_secret(&digits),
            window,
            kind,
        });
        drop(inner);
        self.transport.set_pairing_open(Some(code_hint(&digits)));
        self.emit(GroupEvent::ShortCodeReady {
            code: shown,
            expires_at,
        })
        .await;
        // Stop announcing when the code runs out, unless a newer code replaced it.
        let groups = self.clone();
        (self.spawn)(Box::pin(async move {
            tokio::time::sleep(PICTURE_CHECK).await;
            let mut inner = groups.inner.lock().await;
            if inner
                .short
                .as_ref()
                .is_some_and(|short| short.window.expires_at == expires_at)
            {
                inner.short = None;
                groups.transport.set_pairing_open(None);
            }
        }));
    }

    /// Joining side: finds the nearby computer showing this code and starts the
    /// picture check with it.
    async fn join_with_code(&self, code: &str, computer_name: String) {
        let Some(digits) = normalise_code(code) else {
            self.emit(GroupEvent::Problem(GroupProblem::BadInvite))
                .await;
            return;
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
        let nearby = self
            .transport
            .nearby_pairing(code_hint(&digits), NEARBY_LISTEN_MS)
            .await;
        if nearby.is_empty() {
            self.emit(GroupEvent::Pairing(PairingStep::Failed(
                PairingFailure::Unreachable,
            )))
            .await;
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "No computer nearby is showing a code. Check both computers are on the same Wi-Fi, then try again.".into(),
            )))
            .await;
            return;
        }
        let me = self.me();
        let secret = short_code_secret(&digits);
        for candidate in nearby {
            let request = PeerRequest::JoinByCode {
                proof: invite_proof(&secret, &code_transcript(&candidate.device, &me)),
                person: PersonId(person),
                computer_name: computer_name.clone(),
                from: self.transport.address(),
            };
            if let Ok(PeerResponse::CodeCheck { group, .. }) = self.ask(&candidate, request).await {
                let pictures = short_auth_string(
                    &join_transcript(&candidate.device, &me, &group),
                    &candidate.device,
                    &me,
                );
                self.inner.lock().await.joining = Some(CodeJoin {
                    inviter: candidate,
                    group,
                    computer_name,
                });
                self.emit(GroupEvent::Pairing(PairingStep::PeerConnected))
                    .await;
                self.emit(GroupEvent::Pairing(PairingStep::CodeReady(pictures)))
                    .await;
                return;
            }
        }
        self.emit(GroupEvent::Pairing(PairingStep::Failed(
            PairingFailure::Refused,
        )))
        .await;
        self.emit(GroupEvent::Problem(GroupProblem::Other(
            "That code didn't match. Ask for a new code and type it again.".into(),
        )))
        .await;
    }

    /// "They match" / "They don't", on either computer.
    async fn confirm_code(self: &Arc<Self>, matches: bool) {
        let (inviting, joining) = {
            let mut inner = self.inner.lock().await;
            let inviting = inner.pending_pair.as_ref().map(|pending| {
                pending.answer.send_replace(Some(matches));
            });
            (inviting.is_some(), inner.joining.take())
        };
        self.emit(GroupEvent::Pairing(PairingStep::ConfirmedHere(matches)))
            .await;
        if inviting {
            return;
        }
        let Some(join) = joining else {
            return;
        };
        let groups = self.clone();
        (self.spawn)(Box::pin(async move {
            // The inviting computer answers once its person has compared too.
            let answer = tokio::time::timeout(
                PICTURE_CHECK + REQUEST_TIMEOUT,
                groups
                    .transport
                    .request(&join.inviter, PeerRequest::ConfirmJoin { matches }),
            )
            .await;
            match answer {
                Ok(Ok(PeerResponse::Joined {
                    membership,
                    knowledge,
                })) if matches => {
                    groups
                        .emit(GroupEvent::Pairing(PairingStep::ConfirmedThere(true)))
                        .await;
                    groups
                        .finish_join(
                            join.group,
                            join.inviter,
                            membership,
                            knowledge,
                            join.computer_name,
                        )
                        .await;
                }
                _ if !matches => {}
                _ => {
                    groups
                        .emit(GroupEvent::Pairing(PairingStep::Failed(
                            PairingFailure::CodeMismatch,
                        )))
                        .await;
                    groups
                        .emit(GroupEvent::Problem(GroupProblem::Other(
                            "The computers weren't joined: the pictures didn't match, or the other person didn't answer in time.".into(),
                        )))
                        .await;
                }
            }
        }));
    }

    /// Inviting side: a nearby computer typed a code. One guess, then compare pictures.
    async fn code_join(
        &self,
        from: DeviceId,
        proof: [u8; 32],
        person: PersonId,
        computer_name: String,
        address: PeerAddress,
    ) -> PeerResponse {
        let me = self.me();
        let mut inner = self.inner.lock().await;
        let Some(roster) = inner.roster.clone() else {
            return PeerResponse::Refused {
                reason: Refusal::WrongGroup,
            };
        };
        let Some(mut short) = inner.short.take() else {
            return PeerResponse::Refused {
                reason: Refusal::InviteUnknown,
            };
        };
        // The code is spent by any attempt, right or wrong.
        self.transport.set_pairing_open(None);
        if !short.window.take_guess(now()) {
            return PeerResponse::Refused {
                reason: Refusal::InviteExpired,
            };
        }
        if invite_proof(&short.secret, &code_transcript(&me, &from)) != proof {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "Someone typed a wrong code, so it stopped working. Show a new code to try again."
                    .into(),
            )))
            .await;
            return PeerResponse::Refused {
                reason: Refusal::InviteUnknown,
            };
        }
        let inviter_name = roster
            .owner_of(&me)
            .map(|member| member.name.clone())
            .unwrap_or_default();
        let person_name = match &short.kind {
            InviteKind::Person { name, .. } => name.clone(),
            InviteKind::OwnDevice => inviter_name.clone(),
        };
        let (answer, _) = tokio::sync::watch::channel(None);
        inner.pending_pair = Some(PendingPair {
            joiner: from,
            person,
            computer_name,
            address,
            kind: short.kind,
            answer,
        });
        drop(inner);
        let pictures = short_auth_string(&join_transcript(&me, &from, &roster.group), &me, &from);
        self.emit(GroupEvent::Pairing(PairingStep::PeerConnected))
            .await;
        self.emit(GroupEvent::Pairing(PairingStep::CodeReady(pictures)))
            .await;
        PeerResponse::CodeCheck {
            group: roster.group,
            group_name: roster.name,
            inviter_name,
            person_name,
        }
    }

    /// Inviting side: the joining person answered. Admit only if both said they match.
    async fn confirm_join(self: &Arc<Self>, from: DeviceId, matches: bool) -> PeerResponse {
        let refused = PeerResponse::Refused {
            reason: Refusal::InviteUnknown,
        };
        let mut answer = {
            let mut inner = self.inner.lock().await;
            match inner.pending_pair.as_ref() {
                Some(pending) if pending.joiner == from => {
                    if !matches {
                        inner.pending_pair = None;
                    }
                    pending_answer(inner.pending_pair.as_ref())
                }
                _ => return refused,
            }
        };
        self.emit(GroupEvent::Pairing(PairingStep::ConfirmedThere(matches)))
            .await;
        if !matches {
            return refused;
        }
        let here = match answer.as_mut() {
            Some(answer) => tokio::time::timeout(PICTURE_CHECK, answer.wait_for(Option::is_some))
                .await
                .ok()
                .and_then(Result::ok)
                .and_then(|value| *value),
            None => None,
        };
        let mut inner = self.inner.lock().await;
        let Some(pending) = inner
            .pending_pair
            .take()
            .filter(|pending| pending.joiner == from)
        else {
            return refused;
        };
        if here != Some(true) {
            return refused;
        }
        let group = inner.roster.as_ref().map(|roster| roster.group);
        let Some(group) = group else {
            return refused;
        };
        self.admit_kind(
            inner,
            from,
            group,
            pending.kind,
            pending.person,
            pending.computer_name,
            pending.address,
        )
        .await
    }

    /// The whole group, or the two-person space with `with`.
    async fn space_with(&self, with: Option<PersonId>) -> Space {
        let inner = self.inner.lock().await;
        let me = inner
            .roster
            .as_ref()
            .and_then(|roster| roster.owner_of(&self.me()))
            .map(|member| member.person);
        match (with, me) {
            (Some(other), Some(me)) if other != me => Space::pair(me, other),
            _ => Space::Group,
        }
    }

    /// The space holding a shared detail or artifact (the whole group if unknown).
    async fn space_of(&self, id: Option<&OpHash>) -> Space {
        let inner = self.inner.lock().await;
        id.and_then(|id| Self::space_holding(&inner, id))
            .unwrap_or(Space::Group)
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
            work: inner.record.work,
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
        let qr = self
            .qr
            .as_ref()
            .map(|encode| encode(&link))
            .unwrap_or_default();
        self.emit(GroupEvent::InviteReady {
            link,
            expires_at,
            qr,
        })
        .await;
    }

    /// Takes the history a group handed this computer and becomes a member.
    async fn finish_join(
        &self,
        group: GroupId,
        inviter: PeerAddress,
        membership: Vec<MembershipOp>,
        knowledge: Vec<KnowledgeOp>,
        computer_name: String,
    ) {
        let me = self.me();
        let mut inner = self.inner.lock().await;
        inner.record.group = Some(group);
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
        self.emit(GroupEvent::Pairing(PairingStep::Admitted { group }))
            .await;
        self.publish_loaded().await;
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
                self.finish_join(link.group, inviter, membership, knowledge, computer_name)
                    .await;
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
        let shared = Self::shared(&inner);
        drop(inner);
        self.broadcast(roster.group, peers, &[op], &[]);
        if left {
            self.publish_left().await;
        } else {
            self.publish_changed(roster_now, shared).await;
        }
    }

    /// Signs a knowledge change, keeps it only if the rules accept it, and sends it.
    async fn knowledge(self: &Arc<Self>, space: Space, body: KnowledgeBody) {
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
            KnowledgeBody::ShareTemplate { template, .. } => {
                check_template(template).map_err(|error| format!("That can't be shared: {error}."))
            }
            KnowledgeBody::Corroborate { .. }
            | KnowledgeBody::Withdraw { .. }
            | KnowledgeBody::Redact { .. } => Ok(()),
            // Built here, never asked for by the UI.
            KnowledgeBody::KeyGrant { .. } | KnowledgeBody::Sealed { .. } => {
                Err("That can't be shared directly.".into())
            }
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
        if Self::view_of(&inner, &space).is_none() {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "That person isn't in your group.".into(),
            )))
            .await;
            return;
        }
        let lane = space.lane(&roster.group);
        // Everything new is sealed with a lane key only this space's computers hold.
        let Some((key_id, key, mut new_ops)) = self.ensure_key(&mut inner, &roster, &space).await
        else {
            drop(inner);
            self.emit(GroupEvent::Problem(GroupProblem::Other(
                "This computer couldn't make a secure key for the group. Try again.".into(),
            )))
            .await;
            return;
        };
        let me = self.me();
        let sealed = serde_json::to_vec(&body).ok().and_then(|plaintext| {
            let aad = sealed_aad(&roster.group, &lane, &key_id, &me);
            self.cipher.encrypt(&key, &aad, &plaintext)
        });
        let op = sealed.and_then(|ciphertext| {
            self.sign_next(
                &inner,
                &space,
                KnowledgeBody::Sealed {
                    key: key_id,
                    ciphertext: Bytes(ciphertext),
                },
            )
        });
        let peers = self.peers(&inner);
        let rejection = match &op {
            Some(op) => {
                inner.opened.insert(op.hash(), body);
                let mut trial_ops = inner.record.knowledge.clone();
                trial_ops.push(op.clone());
                let trial = fold_space(
                    &roster,
                    space,
                    &trial_ops,
                    self.verifier.as_ref(),
                    &inner.opened,
                );
                trial.rejected.get(&op.hash()).copied()
            }
            None => None,
        };
        let problem = match (&op, rejection) {
            (None, _) => Some(GroupProblem::Other(
                "This computer couldn't seal that for the group. Try again.".into(),
            )),
            (Some(op), Some(rejection)) => {
                inner.opened.remove(&op.hash());
                Some(knowledge_problem(rejection))
            }
            (Some(op), None) => {
                inner.record.knowledge.push(op.clone());
                new_ops.push(op.clone());
                None
            }
        };
        self.refold(&mut inner).await;
        self.persist(&inner).await;
        let roster_now = inner.roster.clone();
        let shared = Self::shared(&inner);
        drop(inner);
        // Key grants made on the way go out even when the share itself was refused.
        if !new_ops.is_empty() {
            self.broadcast(roster.group, peers, &[], &new_ops);
        }
        if let Some(problem) = problem {
            self.emit(GroupEvent::Problem(problem)).await;
        }
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
        self.admit_kind(
            inner,
            from,
            group,
            invite.kind,
            person,
            computer_name,
            address,
        )
        .await
    }

    /// Signs a new computer's addition (and its person's, for a person invite), hands
    /// it the keys and history of its spaces, and tells the other computers.
    #[expect(
        clippy::too_many_arguments,
        reason = "one admission's facts, used once each"
    )]
    async fn admit_kind(
        self: &Arc<Self>,
        mut inner: tokio::sync::MutexGuard<'_, Inner>,
        from: DeviceId,
        group: GroupId,
        kind: InviteKind,
        person: PersonId,
        computer_name: String,
        address: PeerAddress,
    ) -> PeerResponse {
        let me = self.me();
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
        let bodies = match kind {
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
        // The newcomer can read the history of every space it belongs to: hand it
        // every key held here for those spaces.
        let mut granted = Vec::new();
        let roster_after = inner.roster.clone();
        let held: Vec<(Space, OpHash, [u8; 32])> = std::iter::once(&inner.view)
            .chain(inner.pairs.values())
            .filter(|view| {
                roster_after
                    .as_ref()
                    .is_some_and(|roster| view.space.reaches(roster, &from))
            })
            .flat_map(|view| {
                view.keys
                    .iter()
                    .filter(|(_, lane_key)| !lane_key.grants.contains_key(&from))
                    .filter_map(|(id, _)| {
                        inner.lane_keys.get(id).map(|key| (view.space, *id, *key))
                    })
            })
            .collect();
        for (space, id, key) in held {
            if let Some(op) = self.sign_next(&inner, &space, self.grant_body(id, &key, &[from])) {
                inner.record.knowledge.push(op.clone());
                self.refold(&mut inner).await;
                granted.push(op);
            }
        }
        self.persist(&inner).await;
        // The joiner syncs straight after joining, so a history too large for one
        // message still arrives in full. It only ever sees spaces it belongs to.
        let visible = match &inner.roster {
            Some(roster) => Self::reachable(roster, inner.record.knowledge.clone(), &from),
            None => Vec::new(),
        };
        let response = PeerResponse::Joined {
            membership: inner.record.membership.clone(),
            knowledge: within_budget(visible, MAX_REPLY_KNOWLEDGE_BYTES).0,
        };
        let roster_now = inner.roster.clone();
        let shared = Self::shared(&inner);
        drop(inner);
        self.broadcast(
            group,
            peers
                .into_iter()
                .filter(|peer| peer.device != from)
                .collect(),
            &added,
            &granted,
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
        groups.saw(from).await;
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
            PeerRequest::JoinByCode {
                proof,
                person,
                computer_name,
                from: address,
            } => {
                groups
                    .code_join(from, proof, person, computer_name, address)
                    .await
            }
            PeerRequest::ConfirmJoin { matches } => groups.confirm_join(from, matches).await,
            PeerRequest::Work { job, .. } => groups.work(from, job).await,
            other => groups.serve_peer(from, other).await,
        }
    }
}

impl Groups {
    async fn serve_peer(&self, from: DeviceId, request: PeerRequest) -> PeerResponse {
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
                        inner.roster.as_ref().map_or_else(Vec::new, |roster| {
                            Self::reachable(
                                roster,
                                inner
                                    .record
                                    .knowledge
                                    .iter()
                                    .filter(|op| !knowledge.contains(&op.hash()))
                                    .cloned()
                                    .collect(),
                                &from,
                            )
                        }),
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
            PeerRequest::Join { .. }
            | PeerRequest::Ask { .. }
            | PeerRequest::JoinByCode { .. }
            | PeerRequest::ConfirmJoin { .. }
            | PeerRequest::Work { .. } => PeerResponse::Refused {
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
        Refusal::NotHelping => {
            GroupProblem::Other("That computer isn't helping with work right now.".into())
        }
        Refusal::NotAnswering => {
            GroupProblem::Other("That computer isn't answering questions from the group.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use sha2::{Digest as _, Sha256};
    use sovra_core::group::{KnowledgeView, LaneId, UnsignedKnowledge, fold_knowledge};

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
        *knowledge.0.write().await = vec![view_with_dads_detail()];
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
