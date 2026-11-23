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
            PeerRequest::Join { .. }
            | PeerRequest::Push { .. }
            | PeerRequest::Ask { .. }
            | PeerRequest::JoinByCode { .. }
            | PeerRequest::ConfirmJoin { .. }
            | PeerRequest::Work { .. }
            | PeerRequest::WorkNext { .. }
            | PeerRequest::WorkCancel { .. }
            | PeerRequest::AuditSync { .. }
            | PeerRequest::Training { .. } => PeerResponse::Refused {
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

/// Answers a sync with a full, budget-sized batch of shared artifacts.
struct BigLibrary(Vec<sovra_core::group::KnowledgeOp>);

#[async_trait::async_trait]
impl PeerHandler for BigLibrary {
    async fn handle(&self, _from: DeviceId, request: PeerRequest) -> PeerResponse {
        let PeerRequest::Sync {
            knowledge: known, ..
        } = request
        else {
            return PeerResponse::Refused {
                reason: Refusal::NotAMember,
            };
        };
        let (knowledge, _) = sovra_core::group::within_budget(
            self.0
                .iter()
                .filter(|op| !known.contains(&op.hash()))
                .cloned()
                .collect(),
            sovra_core::group::MAX_REPLY_KNOWLEDGE_BYTES,
        );
        PeerResponse::Synced {
            membership: Vec::new(),
            knowledge,
            want_membership: BTreeSet::new(),
            want_knowledge: BTreeSet::new(),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notes_and_artifacts_cross_the_real_network_in_complete_signed_rounds() {
    use sovra_core::group::{KnowledgeBody, LaneId, UnsignedKnowledge};

    let a = Mesh::bind([5; 32], Reach::LocalOnly).await.expect("binds");
    let b = Mesh::bind([6; 32], Reach::LocalOnly).await.expect("binds");
    let group = GroupId([9; 32]);
    // Lists with many line breaks, which JSON escapes, so the budget must count
    // encoded bytes rather than characters.
    let content = "- milk\n- \"eggs\"\n".repeat(30_000);
    assert!(content.len() <= sovra_core::group::MAX_SHARED_ARTIFACT_BYTES);
    let mut previous = None;
    let ops: Vec<_> = (1..=40_u64)
        .map(|seq| {
            let unsigned = UnsignedKnowledge {
                group,
                lane: LaneId::whole_group(&group),
                author: b.this_device(),
                seq,
                prev: previous,
                deps: previous.into_iter().collect(),
                at: sovra_core::Timestamp(i64::try_from(seq).expect("small")),
                body: if seq % 2 == 0 {
                    KnowledgeBody::ShareArtifact {
                        title: format!("List {seq}"),
                        kind: sovra_core::artifacts::ArtifactKind::Checklist,
                        language: String::new(),
                        content: content.clone(),
                        replaces: Vec::new(),
                        origin: String::new(),
                    }
                } else {
                    KnowledgeBody::Share {
                        text: format!("Note {seq}: مرحباً 👋 café\nKeep every character."),
                        about: Vec::new(),
                        valid_at: None,
                        replaces: Vec::new(),
                    }
                },
            };
            let signature = b.sign(&unsigned.signing_bytes());
            let op = unsigned.sign(signature);
            previous = Some(op.hash());
            op
        })
        .collect();
    let mut server = serve(&b, Arc::new(BigLibrary(ops.clone())));

    let started = std::time::Instant::now();
    let mut received = Vec::new();
    let mut rounds = 0;
    while received.len() < ops.len() {
        assert!(rounds < ops.len(), "every round must make progress");
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            a.request(
                &b.address(),
                PeerRequest::Sync {
                    group,
                    membership: BTreeSet::new(),
                    knowledge: received
                        .iter()
                        .map(sovra_core::group::KnowledgeOp::hash)
                        .collect(),
                    from: a.address(),
                },
            ),
        )
        .await
        .expect("answers in time")
        .expect("budget-sized reply fits");
        let PeerResponse::Synced { knowledge, .. } = response else {
            panic!("expected a sync answer, got {response:?}");
        };
        assert!(!knowledge.is_empty(), "sync must make progress");
        if rounds == 0 {
            assert!(knowledge.len() < ops.len(), "exercise multiple rounds");
        }
        let verifier = a.verifier();
        assert!(knowledge.iter().all(|op| verifier.verify(
            &op.unsigned().author,
            &op.unsigned().signing_bytes(),
            op.signature()
        )));
        received.extend(knowledge);
        assert_eq!(
            received,
            ops[..received.len()],
            "exact signed content, oldest first"
        );
        rounds += 1;
    }
    tracing::info!(
        rounds,
        operations = received.len(),
        elapsed_ms = started.elapsed().as_millis(),
        "mixed group transfer verified"
    );

    b.close().await;
    a.close().await;
    while server.join_next().await.is_some() {}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_computer_showing_a_code_is_found_on_the_same_network() {
    let showing = Mesh::bind([51; 32], Reach::LocalOnly).await.expect("binds");
    let typing = Mesh::bind([52; 32], Reach::LocalOnly).await.expect("binds");
    let bystander = Mesh::bind([53; 32], Reach::LocalOnly).await.expect("binds");
    // As in the app: the group service is running, and the code is shown first.
    let listening = Arc::new(Recorder {
        seen_from: std::sync::Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
    });
    let mut server = serve(&typing, listening);
    showing.set_pairing_open(Some(5));
    bystander.set_pairing_open(Some(6));
    // The person reads the code aloud before the other presses Join: discovery has
    // already reported the computer by then, and must not need to report it again.
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    let mut found = Vec::new();
    for _ in 0..4 {
        found = typing.nearby_pairing(5, 3_000).await;
        if found
            .iter()
            .any(|peer| peer.device == showing.this_device())
        {
            break;
        }
    }
    if typing.nearby.is_none() {
        // No multicast here (some sandboxes): the feature reports nobody nearby.
        assert_eq!(found, Vec::<PeerAddress>::new());
    } else {
        let mine = found
            .iter()
            .find(|peer| peer.device == showing.this_device())
            .expect("the computer showing a code with this hint is found");
        assert!(!mine.addrs.is_empty(), "with an address to dial");
        assert!(
            found
                .iter()
                .all(|peer| peer.device != bystander.this_device()),
            "a computer showing a code with another hint is never contacted"
        );
    }

    showing.close().await;
    typing.close().await;
    bystander.close().await;
    while server.join_next().await.is_some() {}
}

/// Collects broadcasts, for the gossip test.
#[derive(Default)]
struct Heard(std::sync::Mutex<Vec<Vec<u8>>>);

#[async_trait::async_trait]
impl sovra_core::group::GossipHandler for Heard {
    async fn received(&self, message: Vec<u8>) {
        self.0.lock().expect("not poisoned").push(message);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn group_members_hear_each_others_broadcasts_over_iroh_gossip() {
    let a = Mesh::bind([31; 32], Reach::LocalOnly).await.expect("binds");
    let b = Mesh::bind([32; 32], Reach::LocalOnly).await.expect("binds");
    let quiet = Arc::new(Recorder {
        seen_from: std::sync::Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
    });
    let mut a_server = serve(&a, quiet.clone());
    let mut b_server = serve(&b, quiet);
    let (spawn_a, _a_tasks) = spawner();
    let (spawn_b, _b_tasks) = spawner();
    let heard_by_a = Arc::new(Heard::default());
    let heard_by_b = Arc::new(Heard::default());
    let topic = [5; 32];
    a.gossip_join(topic, vec![b.address()], heard_by_a.clone(), spawn_a)
        .await;
    b.gossip_join(topic, vec![a.address()], heard_by_b.clone(), spawn_b)
        .await;
    // Broadcasts reach only current neighbours, so keep announcing until heard,
    // as the runtime does on every sync.
    let heard = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            a.gossip_send(topic, b"checkpoint from a".to_vec()).await;
            if heard_by_b
                .0
                .lock()
                .expect("not poisoned")
                .iter()
                .any(|m| m == b"checkpoint from a")
            {
                return;
            }
            n0_future::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    })
    .await;
    assert!(heard.is_ok(), "b heard a's broadcast");
    b.gossip_send(topic, b"cosigned by b".to_vec()).await;
    let back = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if heard_by_a
                .0
                .lock()
                .expect("not poisoned")
                .iter()
                .any(|m| m == b"cosigned by b")
            {
                return;
            }
            n0_future::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(back.is_ok(), "a heard b's reply");
    // A computer never hears its own broadcast back.
    assert!(
        heard_by_a
            .0
            .lock()
            .expect("not poisoned")
            .iter()
            .all(|m| m != b"checkpoint from a")
    );
    a.close().await;
    b.close().await;
    while a_server.join_next().await.is_some() {}
    while b_server.join_next().await.is_some() {}
}

// ---- groups across the internet --------------------------------------------

/// Answers once `ready` is set; until then every answer is held back, like a
/// large document crawling through a relay.
struct Sleepy {
    ready: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl PeerHandler for Sleepy {
    async fn handle(&self, _from: DeviceId, _request: PeerRequest) -> PeerResponse {
        while !self.ready.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        PeerResponse::Accepted
    }
}

fn sync_request(mesh: &Mesh) -> PeerRequest {
    PeerRequest::Sync {
        group: GroupId([9; 32]),
        membership: BTreeSet::new(),
        knowledge: BTreeSet::new(),
        from: mesh.address(),
    }
}

fn pooled(mesh: &Mesh, device: &DeviceId) -> Option<usize> {
    mesh.pool
        .lock()
        .expect("not poisoned")
        .get(device)
        .map(|pooled| pooled.connection.stable_id())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requests_reuse_one_connection_and_a_restarted_computer_is_dialled_again() {
    let a = Mesh::bind([21; 32], Reach::LocalOnly).await.expect("binds");
    let b = Mesh::bind([22; 32], Reach::LocalOnly).await.expect("binds");
    assert_eq!(a.reachability(), Reachability::SameNetworkOnly);
    assert_eq!(a.route_to(&b.this_device()), None, "nothing open yet");
    let recorder = Arc::new(Recorder {
        seen_from: std::sync::Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
    });
    let mut server = serve(&b, recorder.clone());

    a.request(&b.address(), sync_request(&a))
        .await
        .expect("delivered");
    let first = pooled(&a, &b.this_device()).expect("the connection is kept");
    a.request(&b.address(), sync_request(&a))
        .await
        .expect("delivered");
    assert_eq!(
        pooled(&a, &b.this_device()),
        Some(first),
        "the second request travels on the same connection"
    );
    assert_eq!(a.route_to(&b.this_device()), Some(Route::Direct));

    // The other computer is switched off and on again (same key, new port).
    b.close().await;
    while server.join_next().await.is_some() {}
    let b = Mesh::bind([22; 32], Reach::LocalOnly).await.expect("binds");
    let mut server = serve(&b, recorder.clone());
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        a.request(&b.address(), sync_request(&a)),
    )
    .await
    .expect("answers in time")
    .expect("a fresh connection replaces the dead one");
    assert_ne!(pooled(&a, &b.this_device()), Some(first));
    assert_eq!(recorder.calls.load(Ordering::SeqCst), 3);

    a.close().await;
    assert_eq!(
        pooled(&a, &b.this_device()),
        None,
        "closing empties the pool"
    );
    b.close().await;
    while server.join_next().await.is_some() {}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn giving_up_on_one_request_does_not_cut_off_another_on_the_same_connection() {
    let a = Mesh::bind([23; 32], Reach::LocalOnly).await.expect("binds");
    let b = Mesh::bind([24; 32], Reach::LocalOnly).await.expect("binds");
    let sleepy = Arc::new(Sleepy {
        ready: std::sync::atomic::AtomicBool::new(false),
    });
    let mut server = serve(&b, sleepy.clone());
    let address = b.address();

    // A large document is on its way (here: an answer that takes its time).
    let mut slow = tokio::task::JoinSet::new();
    {
        let (a, address) = (a.clone(), address.clone());
        slow.spawn(async move { a.request(&address, sync_request(&a)).await });
    }
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while pooled(&a, &b.this_device()).is_none() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the connection opens");
    let shared = pooled(&a, &b.this_device());

    // Beside it, a small question runs out of time and is dropped by the runtime.
    let gave_up = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        a.request(&address, sync_request(&a)),
    )
    .await;
    assert!(gave_up.is_err(), "no answer came in time");
    assert_eq!(
        pooled(&a, &b.this_device()),
        shared,
        "the connection the document is travelling on stays open"
    );

    // The document still arrives, on that same connection.
    sleepy.ready.store(true, Ordering::SeqCst);
    let response = tokio::time::timeout(std::time::Duration::from_secs(20), slow.join_next())
        .await
        .expect("answers in time")
        .expect("the request was started")
        .expect("task finishes")
        .expect("the first request was not cut off");
    assert_eq!(response, PeerResponse::Accepted);
    assert_eq!(pooled(&a, &b.this_device()), shared);

    a.close().await;
    b.close().await;
    while server.join_next().await.is_some() {}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lone_request_that_is_given_up_on_does_not_leave_a_stuck_connection() {
    let a = Mesh::bind([25; 32], Reach::LocalOnly).await.expect("binds");
    let b = Mesh::bind([26; 32], Reach::LocalOnly).await.expect("binds");
    let sleepy = Arc::new(Sleepy {
        ready: std::sync::atomic::AtomicBool::new(false),
    });
    let mut server = serve(&b, sleepy.clone());

    // Nothing else is on the connection when the runtime stops waiting: it may
    // have gone quiet (the other computer restarted), so it is not kept.
    let gave_up = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        a.request(&b.address(), sync_request(&a)),
    )
    .await;
    assert!(gave_up.is_err(), "no answer came");
    assert_eq!(pooled(&a, &b.this_device()), None);

    sleepy.ready.store(true, Ordering::SeqCst);
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        a.request(&b.address(), sync_request(&a)),
    )
    .await
    .expect("answers in time")
    .expect("delivered on a new connection");
    assert_eq!(response, PeerResponse::Accepted);

    a.close().await;
    b.close().await;
    while server.join_next().await.is_some() {}
}

fn random_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).expect("the OS has randomness");
    key
}

/// Two computers that can never connect directly: every packet crosses n0's relays
/// on the real internet, as between two homes whose routers block everything else.
/// Run with `cargo test -p sovra-mesh far_apart -- --ignored`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the internet: talks through n0's public relays"]
async fn two_computers_far_apart_reach_each_other_through_a_relay() {
    const ONLINE_MS: u64 = 20_000;
    let a = Mesh::bind(random_key(), Reach::RelayOnly)
        .await
        .expect("binds");
    let b = Mesh::bind(random_key(), Reach::RelayOnly)
        .await
        .expect("binds");
    a.wait_until_reachable(ONLINE_MS).await;
    b.wait_until_reachable(ONLINE_MS).await;
    assert_eq!(a.reachability(), Reachability::Anywhere);
    assert_eq!(b.reachability(), Reachability::Anywhere);
    let recorder = Arc::new(Recorder {
        seen_from: std::sync::Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
    });
    let mut server = serve(&b, recorder.clone());

    // As an invite link carries it: the key and the relay, no usable direct address.
    let address = b.address();
    assert!(
        address.relay.is_some(),
        "a reachable computer names its relay"
    );
    assert!(
        address.addrs.is_empty(),
        "a relay-only computer offers no direct address"
    );
    let started = std::time::Instant::now();
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        a.request(&address, sync_request(&a)),
    )
    .await
    .expect("answers in time")
    .expect("delivered through the relay");
    let first = started.elapsed();
    assert_eq!(a.route_to(&b.this_device()), Some(Route::Relayed));
    let started = std::time::Instant::now();
    a.request(&address, sync_request(&a))
        .await
        .expect("delivered again");
    tracing::info!(?first, again = ?started.elapsed(), "relayed round trips");

    // As a saved group member is dialled after its relay changed: by key alone.
    let c = Mesh::bind(random_key(), Reach::RelayOnly)
        .await
        .expect("binds");
    c.wait_until_reachable(ONLINE_MS).await;
    let by_key = PeerAddress {
        device: b.this_device(),
        relay: None,
        addrs: Vec::new(),
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        c.request(&by_key, sync_request(&c)),
    )
    .await
    .expect("answers in time")
    .expect("found by key and delivered through the relay");
    assert_eq!(recorder.calls.load(Ordering::SeqCst), 3);

    a.close().await;
    c.close().await;
    b.close().await;
    while server.join_next().await.is_some() {}
}

// ---- deflated frames, and computers on an older Sovra ------------------------

/// A sealed document as it travels: bytes written as hex inside JSON.
fn big_question(len: usize) -> PeerRequest {
    let mut bytes = vec![0u8; len / 2];
    getrandom::fill(&mut bytes).expect("the OS has randomness");
    PeerRequest::Ask {
        group: GroupId([9; 32]),
        question: serde_json::to_string(&sovra_core::group::Bytes(bytes)).expect("encodes"),
    }
}

#[test]
fn a_deflated_frame_is_about_half_the_size_and_is_never_unpacked_past_the_limit() {
    use std::io::Write as _;
    let request = big_question(400_000);
    let plain = frame(&request, false);
    let packed = frame(&request, true);
    assert!(
        packed.len() * 10 < plain.len() * 6,
        "hex deflates to nearly the bytes it stands for: {} of {}",
        packed.len(),
        plain.len()
    );
    assert_eq!(unframe::<PeerRequest>(&packed, true), Ok(request.clone()));
    assert_eq!(unframe::<PeerRequest>(&plain, false), Ok(request));
    assert!(matches!(
        unframe::<PeerRequest>(b"not deflate at all", true),
        Err(TransportError::Protocol(_))
    ));

    // A few kilobytes on the wire that would unpack to far more than one message.
    let mut packer = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    packer
        .write_all(&vec![b' '; MAX_PEER_MESSAGE_BYTES * 4])
        .expect("writes to memory");
    let bomb = packer.finish().expect("finishes");
    assert!(bomb.len() < 200_000);
    assert_eq!(
        unframe::<PeerRequest>(&bomb, true),
        Err(TransportError::Protocol("message too large".into()))
    );
}

/// A bare endpoint that speaks only the first protocol name, as Sovra 0.8 does.
async fn older_sovra(key: u8) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::from_bytes(&[key; 32]))
        .alpns(vec![PEER_ALPN.to_vec()])
        .bind()
        .await
        .expect("binds")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computers_on_this_sovra_deflate_and_still_talk_to_an_older_one_both_ways() {
    let new = Mesh::bind([31; 32], Reach::LocalOnly).await.expect("binds");
    let other = Mesh::bind([32; 32], Reach::LocalOnly).await.expect("binds");
    let recorder = Arc::new(Recorder {
        seen_from: std::sync::Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
    });
    let mut server = serve(&other, recorder.clone());
    new.request(&other.address(), sync_request(&new))
        .await
        .expect("delivered");
    let negotiated = new
        .pool
        .lock()
        .expect("not poisoned")
        .get(&other.this_device())
        .map(|pooled| pooled.connection.alpn().to_vec());
    assert_eq!(negotiated.as_deref(), Some(PEER_ALPN_DEFLATE));

    // An older computer calls this one: plain JSON in, plain JSON back.
    let old = older_sovra(33).await;
    let connection = old
        .connect(Mesh::target(&other.address()).expect("valid"), PEER_ALPN)
        .await
        .expect("an older computer still connects");
    let (mut send, mut recv) = connection.open_bi().await.expect("opens");
    send.write_all(&encode(&sync_request(&new)))
        .await
        .expect("writes");
    send.finish().expect("finishes");
    let reply = recv
        .read_to_end(MAX_PEER_MESSAGE_BYTES)
        .await
        .expect("reads");
    assert!(matches!(
        decode::<PeerResponse>(&reply),
        Ok(PeerResponse::Synced { .. })
    ));
    connection.close(0u32.into(), b"done");

    // This computer calls an older one: it falls back to plain JSON.
    let answering = old.clone();
    let mut old_server = tokio::task::JoinSet::new();
    old_server.spawn(async move {
        let connection = answering
            .accept()
            .await
            .expect("a caller")
            .await
            .expect("handshake");
        let (mut send, mut recv) = connection.accept_bi().await.expect("a request");
        let bytes = recv
            .read_to_end(MAX_PEER_MESSAGE_BYTES)
            .await
            .expect("reads");
        assert!(decode::<PeerRequest>(&bytes).is_ok(), "plain JSON arrived");
        send.write_all(&encode(&PeerResponse::Accepted))
            .await
            .expect("writes");
        send.finish().expect("finishes");
        connection.closed().await;
    });
    let response = new
        .request(&peer_address(&old), sync_request(&new))
        .await
        .expect("delivered to the older computer");
    assert_eq!(response, PeerResponse::Accepted);

    new.close().await;
    other.close().await;
    old.close().await;
    while server.join_next().await.is_some() {}
    while old_server.join_next().await.is_some() {}
}
