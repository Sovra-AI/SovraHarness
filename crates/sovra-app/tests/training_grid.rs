//! Production Burn work packets crossing production iroh transport. This exercises
//! the transport/learner seam before the desktop enables consenting grid sessions.

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
        SAMPLE_TEXT, TrainingEngine, TrainingPeerRequest, TrainingPeerResponse, TrainingSession,
        TrainingTask,
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
            }
        })
        .await
        .expect("test learner task completes");
        PeerResponse::Training { work }
    }
}

async fn ask(worker: &Mesh, address: &PeerAddress, work: TrainingPeerRequest) -> PeerResponse {
    tokio::time::timeout(
        Duration::from_secs(15),
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
    let coordinator = Arc::new(Coordinator {
        members: Mutex::new(BTreeSet::from([fast.this_device(), gone.this_device()])),
        shared: AtomicBool::new(false),
        now: AtomicU64::new(0),
        session: Arc::new(Mutex::new(
            BurnTrainingEngine
                .create(SAMPLE_TEXT, ContentHash::of(b"network run"))
                .unwrap(),
        )),
    });
    let tasks = Arc::new(Mutex::new(tokio::task::JoinSet::new()));
    let owned = tasks.clone();
    let spawn: Spawn = Arc::new(move |task| {
        owned.lock().unwrap().spawn(task);
    });
    let serving = host.clone();
    let handler = coordinator.clone();
    let mut server = tokio::task::JoinSet::new();
    server.spawn(async move {
        serving.serve(handler, spawn).await;
    });
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
        let worker = fast.this_device();
        let result = tokio::task::spawn_blocking(move || BurnTrainingEngine.work(task, worker))
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
    server.shutdown().await;
    tasks.lock().unwrap().abort_all();
}
