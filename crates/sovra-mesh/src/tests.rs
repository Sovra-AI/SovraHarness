use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sovra_core::group::{
    GroupId, GroupRecord, GroupStore as _, OpHash, PeerHandler, PeerRequest, PeerResponse, Refusal,
    Spawn,
};

use super::*;

/// Answers every sync with an empty diff and counts calls; refuses pushes.
struct Recorder {
    seen_from: std::sync::Mutex<Vec<DeviceId>>,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl PeerHandler for Recorder {
    async fn handle(&self, from: DeviceId, request: PeerRequest) -> PeerResponse {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen_from.lock().expect("not poisoned").push(from);
        match request {
            PeerRequest::Sync { membership, .. } => PeerResponse::Synced {
                membership: Vec::new(),
                knowledge: Vec::new(),
                want_membership: membership,
                want_knowledge: BTreeSet::new(),
            },
            PeerRequest::Join { .. } | PeerRequest::Push { .. } => PeerResponse::Refused {
                reason: Refusal::NotAMember,
            },
        }
    }
}

/// Owned tasks, as the runtime does it: everything lands in one `JoinSet`.
fn spawner() -> (Spawn, Arc<std::sync::Mutex<tokio::task::JoinSet<()>>>) {
    let tasks = Arc::new(std::sync::Mutex::new(tokio::task::JoinSet::new()));
    let owner = tasks.clone();
    let spawn: Spawn = Arc::new(move |task| {
        owner.lock().expect("not poisoned").spawn(task);
    });
    (spawn, tasks)
}

/// Serves `mesh` on an owned task until it is closed.
fn serve(mesh: &Mesh, handler: Arc<dyn PeerHandler>) -> tokio::task::JoinSet<()> {
    let (spawn, _connections) = spawner();
    let serving = mesh.clone();
    let mut server = tokio::task::JoinSet::new();
    server.spawn(async move { serving.serve(handler, spawn).await });
    server
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_computers_talk_directly_and_see_each_others_keys() {
    let a = Mesh::bind([1; 32], Reach::LocalOnly).await.expect("binds");
    let b = Mesh::bind([2; 32], Reach::LocalOnly).await.expect("binds");
    assert_eq!(a.this_device(), Mesh::device_for(&[1; 32]));
    assert_ne!(a.this_device(), b.this_device());

    let recorder = Arc::new(Recorder {
        seen_from: std::sync::Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
    });
    let mut server = serve(&b, recorder.clone());

    let wanted = BTreeSet::from([OpHash([7; 32])]);
    let request = PeerRequest::Sync {
        group: GroupId([9; 32]),
        membership: wanted.clone(),
        knowledge: BTreeSet::new(),
        from: a.address(),
    };
    let address = b.address();
    assert!(
        !address.addrs.is_empty(),
        "a local endpoint has direct addresses"
    );
    for _ in 0..2 {
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            a.request(&address, request.clone()),
        )
        .await
        .expect("answers in time")
        .expect("delivered");
        assert!(matches!(
            response,
            PeerResponse::Synced { ref want_membership, .. } if *want_membership == wanted
        ));
    }
    assert_eq!(recorder.calls.load(Ordering::SeqCst), 2);
    assert!(
        recorder
            .seen_from
            .lock()
            .expect("not poisoned")
            .iter()
            .all(|from| *from == a.this_device()),
        "the handler learns the caller's authenticated key"
    );

    b.close().await;
    a.close().await;
    while server.join_next().await.is_some() {}
    assert_eq!(
        a.request(&address, request).await,
        Err(TransportError::Closed)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_computer_that_is_not_the_one_dialled_never_answers() {
    let a = Mesh::bind([3; 32], Reach::LocalOnly).await.expect("binds");
    let b = Mesh::bind([4; 32], Reach::LocalOnly).await.expect("binds");
    let recorder = Arc::new(Recorder {
        seen_from: std::sync::Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
    });
    let mut server = serve(&b, recorder);

    // Right addresses, wrong key: the QUIC handshake must fail.
    let mut impostor = b.address();
    impostor.device = Mesh::device_for(&[5; 32]);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        a.request(
            &impostor,
            PeerRequest::Sync {
                group: GroupId([0; 32]),
                membership: BTreeSet::new(),
                knowledge: BTreeSet::new(),
                from: a.address(),
            },
        ),
    )
    .await;
    assert!(
        !matches!(result, Ok(Ok(_))),
        "must not reach a different key"
    );
    b.close().await;
    a.close().await;
    while server.join_next().await.is_some() {}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signatures_verify_only_for_the_signing_computer() {
    let a = Mesh::bind([6; 32], Reach::LocalOnly).await.expect("binds");
    let signature = a.sign(b"hello group");
    let verifier = a.verifier();
    assert!(verifier.verify(&a.this_device(), b"hello group", &signature));
    assert!(!verifier.verify(&a.this_device(), b"hello groups", &signature));
    assert!(!verifier.verify(&Mesh::device_for(&[7; 32]), b"hello group", &signature));
    a.close().await;
}

#[tokio::test]
async fn the_group_record_survives_a_save_and_reload() {
    let dir = std::env::temp_dir().join(format!(
        "sovra-mesh-store-{}-{}",
        std::process::id(),
        line!()
    ));
    let store = FileGroupStore::new(&dir);
    assert_eq!(store.load().await, Ok(GroupRecord::default()));
    let record = GroupRecord {
        person_name: "Mum".into(),
        computer_name: "Mum's laptop".into(),
        group: Some(GroupId([1; 32])),
        ..GroupRecord::default()
    };
    store.save(&record).await.expect("saves");
    assert_eq!(store.load().await, Ok(record));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(dir.join("group.json"))
            .expect("exists")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    std::fs::remove_dir_all(dir).expect("cleans up");
}
