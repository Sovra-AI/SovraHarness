//! The Burn engine's work packets and finished models crossing the real iroh
//! transport, without a runtime in between: the seam between the learner and the
//! wire. One test uses the Starter model in the first release's packet form, the
//! other the largest model a group can train, whose packets are the biggest
//! messages Sovra sends.

use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use sovra_core::{
    group::{
        ContentHash, DeviceId, GroupId, GroupTransport, PeerAddress, PeerHandler, PeerRequest,
        PeerResponse, Refusal, Spawn,
    },
    training::{
        BaseModel, HardwareInfo, ModelAssembly, ModelCard, ModelOrigin, SAMPLE_TEXT, TrainingBase,
        TrainingEngine, TrainingPeerRequest, TrainingPeerResponse, TrainingPlan, TrainingPreset,
        TrainingRecipe, TrainingSession, TrainingSettings, TrainingSource, TrainingTask,
        model_part, offers,
    },
};
use sovra_mesh::{Mesh, Reach};
use sovra_train::BurnTrainingEngine;

const GROUP: GroupId = GroupId([41; 32]);

struct Coordinator {
    members: Mutex<BTreeSet<DeviceId>>,
    shared: AtomicBool,
    now: AtomicU64,
    session: Arc<Mutex<Box<dyn TrainingSession>>>,
    /// The finished model this computer shares with the group, once there is one.
    finished: Mutex<Option<(ModelCard, Arc<Vec<u8>>)>>,
}

impl Coordinator {
    fn new(members: &[&Mesh], session: Box<dyn TrainingSession>) -> Arc<Self> {
        Arc::new(Self {
            members: Mutex::new(members.iter().map(|mesh| mesh.this_device()).collect()),
            shared: AtomicBool::new(false),
            now: AtomicU64::new(0),
            session: Arc::new(Mutex::new(session)),
            finished: Mutex::new(None),
        })
    }

    /// Answers the requests that need no training session: who has which model.
    fn library(&self, work: &TrainingPeerRequest) -> Option<TrainingPeerResponse> {
        let finished = self.finished.lock().expect("test library lock");
        Some(match work {
            TrainingPeerRequest::Work | TrainingPeerRequest::Submit(_) => return None,
            TrainingPeerRequest::Status => TrainingPeerResponse::Unavailable,
            TrainingPeerRequest::Offers { have } => TrainingPeerResponse::Offered(
                finished
                    .iter()
                    .filter(|(card, _)| !have.contains(&card.id))
                    .map(|(card, _)| card.clone())
                    .collect(),
            ),
            TrainingPeerRequest::FetchModel { model, offset } => finished
                .as_ref()
                .filter(|(card, _)| card.id == *model)
                .map_or(TrainingPeerResponse::Unavailable, |(_, file)| {
                    model_part(file, *offset)
                }),
            TrainingPeerRequest::Received { .. } | TrainingPeerRequest::Announce { .. } => {
                TrainingPeerResponse::Thanks
            }
        })
    }
}

#[async_trait::async_trait]
impl PeerHandler for Coordinator {
    async fn handle(&self, from: DeviceId, request: PeerRequest) -> PeerResponse {
        let PeerRequest::Training { group, work } = request else {
            return PeerResponse::Refused {
                reason: Refusal::Malformed,
            };
        };
        if group != GROUP
            || !self
                .members
                .lock()
                .expect("test roster lock")
                .contains(&from)
        {
            return PeerResponse::Refused {
                reason: Refusal::NotAMember,
            };
        }
        if let Some(work) = self.library(&work) {
            return PeerResponse::Training { work };
        }
        if !self.shared.load(Ordering::SeqCst) {
            return PeerResponse::Training {
                work: TrainingPeerResponse::Unavailable,
            };
        }
        let session = self.session.clone();
        let now = self.now.load(Ordering::SeqCst);
        let work = tokio::task::spawn_blocking(move || {
            let mut session = session.lock().expect("test session lock");
            match work {
                TrainingPeerRequest::Work => session
                    .assign(from, now)
                    .map_or(TrainingPeerResponse::Rejected, |task| {
                        TrainingPeerResponse::Assigned(Box::new(task))
                    }),
                TrainingPeerRequest::Submit(result) => session
                    .merge(*result, from, now)
                    .map_or(TrainingPeerResponse::Rejected, |()| {
                        TrainingPeerResponse::Accepted(session.progress())
                    }),
                TrainingPeerRequest::Status
                | TrainingPeerRequest::Offers { .. }
                | TrainingPeerRequest::FetchModel { .. }
                | TrainingPeerRequest::Received { .. }
                | TrainingPeerRequest::Announce { .. } => TrainingPeerResponse::Unavailable,
            }
        })
        .await
        .expect("test learner task completes");
        PeerResponse::Training { work }
    }
}

/// A run on the built-in sample, started with the group, with the Quick try
/// settings for `base`.
fn session(base: BaseModel, run: &[u8]) -> Box<dyn TrainingSession> {
    let plan = TrainingPlan {
        name: "Network run".into(),
        base: TrainingBase::Fresh(base),
        source: TrainingSource::Sample,
        preset: Some(TrainingPreset::Quick),
        settings: TrainingSettings::recommended(
            TrainingPreset::Quick,
            base.architecture(),
            SAMPLE_TEXT.len(),
            &HardwareInfo::default(),
        ),
        with_group: true,
    };
    BurnTrainingEngine::processor()
        .create(&plan, SAMPLE_TEXT, None, ContentHash::of(run))
        .expect("the run starts")
}

type Tasks = Arc<Mutex<tokio::task::JoinSet<()>>>;

fn serve(host: &Mesh, coordinator: &Arc<Coordinator>) -> (tokio::task::JoinSet<()>, Tasks) {
    let tasks: Tasks = Arc::new(Mutex::new(tokio::task::JoinSet::new()));
    let owned = tasks.clone();
    let spawn: Spawn = Arc::new(move |task| {
        owned.lock().expect("test task set").spawn(task);
    });
    let serving = host.clone();
    let handler = coordinator.clone();
    let mut server = tokio::task::JoinSet::new();
    server.spawn(async move {
        serving.serve(handler, spawn).await;
    });
    (server, tasks)
}

async fn stop(mut server: tokio::task::JoinSet<()>, tasks: &Tasks) {
    server.shutdown().await;
    tasks.lock().expect("test task set").abort_all();
}

async fn ask(worker: &Mesh, address: &PeerAddress, work: TrainingPeerRequest) -> PeerResponse {
    tokio::time::timeout(
        Duration::from_secs(60),
        worker.request(address, PeerRequest::Training { group: GROUP, work }),
    )
    .await
    .expect("network deadline")
    .expect("delivered")
}

async fn assignment(worker: &Mesh, address: &PeerAddress) -> TrainingTask {
    match ask(worker, address, TrainingPeerRequest::Work).await {
        PeerResponse::Training {
            work: TrainingPeerResponse::Assigned(task),
        } => *task,
        response => panic!("expected work, got {response:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_peer_work_survives_worker_loss_and_binds_updates_to_authenticated_keys() {
    let host = Mesh::bind([61; 32], Reach::LocalOnly).await.unwrap();
    let fast = Mesh::bind([62; 32], Reach::LocalOnly).await.unwrap();
    let gone = Mesh::bind([63; 32], Reach::LocalOnly).await.unwrap();
    let outsider = Mesh::bind([64; 32], Reach::LocalOnly).await.unwrap();
    let coordinator =
        Coordinator::new(&[&fast, &gone], session(BaseModel::Starter, b"network run"));
    let (server, tasks) = serve(&host, &coordinator);
    let address = host.address();
    assert_eq!(
        ask(&fast, &address, TrainingPeerRequest::Work).await,
        PeerResponse::Training {
            work: TrainingPeerResponse::Unavailable
        }
    );
    coordinator.shared.store(true, Ordering::SeqCst);
    assert!(matches!(
        ask(&outsider, &address, TrainingPeerRequest::Work).await,
        PeerResponse::Refused { .. }
    ));
    let abandoned = assignment(&gone, &address).await;
    assert_eq!(abandoned.lease.worker, gone.this_device());
    gone.close().await;
    let mut old_result = None;
    for round in 0..5 {
        let task = assignment(&fast, &address).await;
        assert_eq!(task.lease.worker, fast.this_device());
        assert_eq!(
            task.recipe,
            TrainingRecipe::ByteBigramV1,
            "a Starter run on its usual settings keeps the first release's packets"
        );
        let worker = fast.this_device();
        let result =
            tokio::task::spawn_blocking(move || BurnTrainingEngine::processor().work(task, worker))
                .await
                .unwrap()
                .unwrap();
        old_result = Some(result.clone());
        assert!(matches!(
            ask(
                &outsider,
                &address,
                TrainingPeerRequest::Submit(Box::new(result.clone()))
            )
            .await,
            PeerResponse::Refused { .. }
        ));
        let response = ask(
            &fast,
            &address,
            TrainingPeerRequest::Submit(Box::new(result)),
        )
        .await;
        let PeerResponse::Training {
            work: TrainingPeerResponse::Accepted(progress),
        } = response
        else {
            panic!("merge failed");
        };
        assert_eq!(progress.rounds, round + 1);
        assert!(progress.loss_milli.unwrap() < 5546);
        coordinator.now.fetch_add(1, Ordering::SeqCst);
    }
    assert_eq!(
        ask(
            &fast,
            &address,
            TrainingPeerRequest::Submit(Box::new(old_result.unwrap()))
        )
        .await,
        PeerResponse::Training {
            work: TrainingPeerResponse::Rejected
        }
    );
    coordinator
        .members
        .lock()
        .unwrap()
        .remove(&fast.this_device());
    assert!(matches!(
        ask(&fast, &address, TrainingPeerRequest::Work).await,
        PeerResponse::Refused { .. }
    ));
    host.close().await;
    fast.close().await;
    outsider.close().await;
    stop(server, &tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_largest_group_model_trains_and_is_delivered_over_the_real_wire() {
    let host = Mesh::bind([65; 32], Reach::LocalOnly).await.unwrap();
    let member = Mesh::bind([66; 32], Reach::LocalOnly).await.unwrap();
    let outsider = Mesh::bind([67; 32], Reach::LocalOnly).await.unwrap();
    let coordinator = Coordinator::new(&[&member], session(BaseModel::Medium, b"medium run"));
    coordinator.shared.store(true, Ordering::SeqCst);
    let (server, tasks) = serve(&host, &coordinator);
    let address = host.address();

    // One packet out and its result back: each is a single message of about 7 MB.
    let task = assignment(&member, &address).await;
    let TrainingRecipe::ByteModelV2(recipe) = task.recipe else {
        panic!("a transformer packet names its shape");
    };
    assert_eq!(recipe.architecture, BaseModel::Medium.architecture());
    assert_eq!(task.model.0.len(), recipe.architecture.parameters() * 4);
    let worker = member.this_device();
    let result =
        tokio::task::spawn_blocking(move || BurnTrainingEngine::processor().work(task, worker))
            .await
            .unwrap()
            .unwrap();
    assert!(matches!(
        ask(
            &outsider,
            &address,
            TrainingPeerRequest::Submit(Box::new(result.clone()))
        )
        .await,
        PeerResponse::Refused { .. }
    ));
    let accepted = ask(
        &member,
        &address,
        TrainingPeerRequest::Submit(Box::new(result)),
    )
    .await;
    let PeerResponse::Training {
        work: TrainingPeerResponse::Accepted(progress),
    } = accepted
    else {
        panic!("merge failed: {accepted:?}");
    };
    assert_eq!(progress.rounds, 1);

    // The model as it stands becomes a file shared with the group.
    let file = coordinator.session.lock().unwrap().export().unwrap();
    let facts = BurnTrainingEngine::processor().inspect(&file).unwrap();
    let card = ModelCard::from_facts(
        &facts,
        "Medium over the wire".into(),
        "Host · Host computer".into(),
        ModelOrigin::TrainedHere,
        Some(GROUP),
        1,
    );
    *coordinator.finished.lock().unwrap() = Some((card.clone(), Arc::new(file.clone())));

    // A member asks what there is, then pulls the file part by part.
    let offered = ask(
        &member,
        &address,
        TrainingPeerRequest::Offers { have: vec![] },
    )
    .await;
    assert_eq!(
        offered,
        PeerResponse::Training {
            work: TrainingPeerResponse::Offered(vec![card.clone()])
        }
    );
    assert_eq!(
        offers(&[], GROUP, &[]),
        vec![],
        "an empty library offers nothing"
    );
    let mut assembly = ModelAssembly::new(card.clone(), GROUP).unwrap();
    let mut parts = 0;
    let fetched = loop {
        let PeerResponse::Training {
            work: TrainingPeerResponse::ModelPart { total, bytes },
        } = ask(&member, &address, assembly.request()).await
        else {
            panic!("expected a part of the model");
        };
        parts += 1;
        if let Some(whole) = assembly.accept(total, &bytes).unwrap() {
            break whole;
        }
    };
    assert_eq!(parts, 2, "3.5 MB arrives as two bounded parts");
    assert_eq!(fetched, file);
    assert_eq!(
        BurnTrainingEngine::processor()
            .inspect(&fetched)
            .unwrap()
            .id,
        card.id
    );
    let written = BurnTrainingEngine::processor()
        .generate(&fetched, "The garden", 40, 800, 3)
        .unwrap();
    assert_eq!(
        written,
        BurnTrainingEngine::processor()
            .generate(&file, "The garden", 40, 800, 3)
            .unwrap()
    );
    assert_eq!(
        ask(
            &member,
            &address,
            TrainingPeerRequest::Offers {
                have: vec![card.id]
            }
        )
        .await,
        PeerResponse::Training {
            work: TrainingPeerResponse::Offered(vec![])
        }
    );
    assert!(matches!(
        ask(&outsider, &address, assembly.request()).await,
        PeerResponse::Refused { .. }
    ));
    host.close().await;
    member.close().await;
    outsider.close().await;
    stop(server, &tasks).await;
}
