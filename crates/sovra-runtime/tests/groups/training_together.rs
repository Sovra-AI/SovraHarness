//! A group trains together and everyone gets the finished model.
//!
//! Three computers of one group and one outside it, each a full runtime with
//! its own training folder. The engine is a stand-in whose helper step changes
//! nothing, so these tests are about who may do what, not about learning.

use std::path::{Path, PathBuf};

use sovra_core::group::{GroupId, Refusal};
use sovra_core::training::{
    HelpingPhase, LibraryModel, MAX_MODEL_BYTES, ModelCard, ModelId, ModelOrigin, model_id,
};

use super::*;

/// One computer with training switched on.
struct Trainer {
    computer: Computer,
    engine: Arc<CountingEngine>,
    directory: PathBuf,
    n: u8,
}

impl std::ops::Deref for Trainer {
    type Target = Computer;

    fn deref(&self) -> &Computer {
        &self.computer
    }
}

impl Trainer {
    fn open(net: &Arc<Network>, n: u8, root: &Path, store: Arc<MemoryStore>) -> Self {
        let engine = CountingEngine::helpful();
        let directory = root.join(format!("computer-{n}"));
        let computer = Computer::open(
            net,
            n,
            None,
            store,
            Some(TrainingConfig {
                engine: engine.clone(),
                directory: directory.clone(),
            }),
        );
        Self {
            computer,
            engine,
            directory,
            n,
        }
    }

    /// The same computer switched off and on again, with everything it saved.
    async fn restart(self, net: &Arc<Network>) -> Self {
        let Self {
            computer,
            directory,
            n,
            ..
        } = self;
        let store = computer.store.clone();
        let client = computer.client.clone();
        drop(computer);
        tokio::task::spawn_blocking(move || client.shutdown_blocking().expect("stops"))
            .await
            .expect("stopped");
        let root = directory.parent().expect("inside the test folder");
        Self::open(net, n, root, store)
    }

    fn train(&self, command: TrainingCommand) {
        self.client
            .send(Command::Training(command))
            .expect("runtime accepts commands");
    }

    /// Turns "Help the group train" on or off and waits until it is saved.
    async fn permit(&self, on: bool) {
        self.send(GroupCommand::SetTrainingHelp(on));
        tokio::time::timeout(WAIT, async {
            while self.store.0.lock().expect("store").help_train != on {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the permission is saved");
    }

    /// Model files in this computer's library folder.
    fn model_files(&self) -> usize {
        std::fs::read_dir(self.directory.join("models")).map_or(0, |entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "model"))
                .count()
        })
    }

    /// Waits until `model` is in this computer's library, and returns it.
    async fn has_model(&self, model: ModelId) -> LibraryModel {
        let state = self.training(|state| state.model(model).is_some()).await;
        state.model(model).expect("just seen").clone()
    }
}

struct Team {
    _dir: tempfile::TempDir,
    net: Arc<Network>,
    mum: Trainer,
    dad: Trainer,
    kid: Trainer,
    outsider: Trainer,
    group: GroupId,
    /// The group as Mum's computer listed it once everyone had joined.
    roster: GroupState,
}

impl Team {
    /// Mum founds the group; Dad and Kid join as people of it. Nobody has
    /// turned "Help the group train" on.
    async fn start(n: u8) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let net = Arc::new(Network::default());
        let open = |n: u8| Trainer::open(&net, n, dir.path(), Arc::new(MemoryStore::default()));
        let (mum, dad, kid, outsider) = (open(n), open(n + 1), open(n + 2), open(n + 3));
        mum.send(GroupCommand::Create {
            name: "Family".into(),
            your_name: "Mum".into(),
            computer_name: "Laptop".into(),
        });
        let mut roster = mum.roster(|state| state.name == "Family").await;
        let group = roster.group;
        for (joiner, name, computer) in [(&dad, "Dad", "Desktop"), (&kid, "Kid", "Tablet")] {
            mum.send(GroupCommand::InvitePerson {
                name: name.into(),
                role: Role::Member,
            });
            let link = mum.link().await;
            joiner.send(GroupCommand::Join {
                link,
                computer_name: computer.into(),
            });
            roster = mum.roster(|state| state.admits(&joiner.device)).await;
        }
        dad.roster(|state| state.admits(&kid.device)).await;
        kid.roster(|state| state.admits(&dad.device)).await;
        Self {
            _dir: dir,
            net,
            mum,
            dad,
            kid,
            outsider,
            group,
            roster,
        }
    }

    /// Mum starts training with the group: the one thing that lets the group's
    /// computers be given pieces of her text.
    async fn mum_starts(&self) {
        self.mum
            .train(TrainingCommand::Start(Box::new(training_plan(true))));
        self.mum
            .training(|state| {
                state.phase == TrainingPhase::Running && state.sharing == Some(self.group)
            })
            .await;
    }

    /// Mum presses Stop: the model as it is goes to her library, shared with
    /// the group the run was trained with.
    async fn mum_finishes(&self) -> LibraryModel {
        self.mum.train(TrainingCommand::Stop);
        let done = self
            .mum
            .training(|state| state.phase == TrainingPhase::Complete && !state.models.is_empty())
            .await;
        assert_eq!(done.error, None);
        assert_eq!(done.sharing, None, "a finished run serves no more work");
        let model = done.models[0].clone();
        assert_eq!(model.card.origin, ModelOrigin::TrainedHere);
        assert_eq!(model.card.group, Some(self.group));
        model
    }

    async fn ask(&self, from: DeviceId, to: DeviceId, work: TrainingPeerRequest) -> PeerResponse {
        ask_training(&self.net, from, to, self.group, work)
            .await
            .expect("delivered")
    }

    /// One round given to Mum's run in `from`'s name, without `from`'s runtime
    /// taking part.
    async fn contribute(&self, from: DeviceId) {
        let assigned = self
            .ask(from, self.mum.device, TrainingPeerRequest::Work)
            .await;
        let PeerResponse::Training {
            work: TrainingPeerResponse::Assigned(task),
        } = assigned
        else {
            panic!("expected a work packet, got {assigned:?}");
        };
        let accepted = self
            .ask(
                from,
                self.mum.device,
                TrainingPeerRequest::Submit(Box::new(zero_result(&task))),
            )
            .await;
        assert!(
            matches!(
                accepted,
                PeerResponse::Training {
                    work: TrainingPeerResponse::Accepted(_)
                }
            ),
            "{accepted:?}"
        );
    }

    fn switch_off(&self, device: DeviceId) {
        self.net.off.lock().expect("off").insert(device);
    }

    fn switch_on(&self, device: DeviceId) {
        self.net.off.lock().expect("off").remove(&device);
    }
}

fn helping(state: &TrainingState) -> bool {
    state
        .helping
        .as_ref()
        .is_some_and(|help| help.phase.active())
}

fn rounds_given(state: &TrainingState) -> u32 {
    state
        .helping
        .as_ref()
        .map_or(0, |help| help.accepted_rounds)
}

/// The file a library entry points at is exactly the model its card names.
fn file_is(entry: &LibraryModel, model: ModelId) {
    let bytes = std::fs::read(&entry.saved_at).expect("the model file is on disk");
    assert_eq!(model_id(&bytes), model);
    assert_eq!(bytes.len() as u64, entry.card.bytes);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_permitted_computer_helps_by_itself_and_every_member_gets_the_model() {
    let team = Team::start(101).await;
    // Dad lets his computer help. Kid does not.
    team.dad.permit(true).await;
    team.mum_starts().await;

    // Nobody chose a computer: Dad's finds Mum's run and gives it rounds.
    let dad = team.dad.training(|state| rounds_given(state) >= 2).await;
    let help = dad.helping.as_ref().expect("helping");
    assert_eq!(help.peer.device, team.mum.device);
    assert_eq!(help.group, team.group);
    let run = help.run.as_ref().expect("what is being trained");
    assert_eq!(run.name, training_plan(true).name);
    assert_eq!(run.base, BaseModel::Starter);
    assert_eq!(
        dad.phase,
        TrainingPhase::Ready,
        "Dad trains nothing of his own"
    );

    // Mum's page shows who is helping and how much.
    let mum = team
        .mum
        .training(|state| {
            state.helped_rounds >= 2
                && state.group.as_ref().is_some_and(|group| {
                    group
                        .peers
                        .iter()
                        .any(|peer| peer.device == team.dad.device && peer.rounds >= 2)
                })
        })
        .await;
    assert_eq!(mum.phase, TrainingPhase::Running);
    assert_eq!(mum.error, None);
    let kid_row = mum
        .group
        .as_ref()
        .expect("group")
        .peers
        .iter()
        .find(|peer| peer.device == team.kid.device)
        .expect("kid is listed");
    assert_eq!((kid_row.rounds, kid_row.helping), (0, false));
    // The owner never runs work a helper sent; the helper did the learning.
    assert_eq!(team.mum.engine.learned(), 0);
    assert!(team.dad.engine.learned() >= 2);
    assert_eq!(team.kid.engine.learned(), 0, "Kid's switch is off");

    // The finished model goes to everyone in the group, switch or no switch.
    let model = team.mum_finishes().await;
    let id = model.card.id;
    assert!(model.card.rounds >= 2);
    file_is(&model, id);
    for member in [&team.dad, &team.kid] {
        let received = member.has_model(id).await;
        assert_eq!(received.card.id, id, "the same model on every computer");
        assert_eq!(
            received.card.origin,
            ModelOrigin::Received {
                from: team.mum.device
            }
        );
        assert_eq!(received.card.name, model.card.name);
        assert_eq!(received.card.trained_by, "Mum · Laptop");
        assert_eq!(received.card.group, Some(team.group));
        assert!(
            received.delivery.is_empty(),
            "only the owner tracks delivery"
        );
        file_is(&received, id);
        assert_eq!(member.model_files(), 1);
    }
    let delivered = team
        .mum
        .training(|state| {
            state
                .model(id)
                .is_some_and(|model| model.delivered() == (2, 2))
        })
        .await;
    let names: BTreeSet<&str> = delivered
        .model(id)
        .expect("listed")
        .delivery
        .iter()
        .map(|delivery| delivery.name.as_str())
        .collect();
    assert_eq!(names, BTreeSet::from(["Dad · Desktop", "Kid · Tablet"]));

    // A computer outside the group is told nothing and given nothing.
    for request in [
        TrainingPeerRequest::Offers { have: Vec::new() },
        TrainingPeerRequest::FetchModel {
            model: id,
            offset: 0,
        },
        TrainingPeerRequest::Received { model: id },
        TrainingPeerRequest::Announce { models: Vec::new() },
    ] {
        assert_eq!(
            refusal(
                team.ask(team.outsider.device, team.mum.device, request)
                    .await
            ),
            Refusal::NotAMember
        );
    }
    assert_eq!(team.outsider.model_files(), 0);
    // A received model is not passed on by the computer that received it.
    let offered = team
        .ask(
            team.kid.device,
            team.dad.device,
            TrainingPeerRequest::Offers { have: Vec::new() },
        )
        .await;
    assert!(
        matches!(
            &offered,
            PeerResponse::Training {
                work: TrainingPeerResponse::Offered(cards)
            } if cards.is_empty()
        ),
        "{offered:?}"
    );

    // With the run over, Dad's computer lets go of it by itself.
    let after = team.dad.training(|state| !helping(state)).await;
    assert!(rounds_given(&after) >= 2);
    assert_eq!(team.mum.engine.learned(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_member_that_was_away_gets_the_model_when_it_is_back_and_delivery_is_remembered() {
    let team = Team::start(111).await;
    team.mum_starts().await;
    team.contribute(team.dad.device).await;
    // Kid's computer is off when the model is finished.
    team.switch_off(team.kid.device);
    let model = team.mum_finishes().await;
    let id = model.card.id;
    assert_eq!(model.delivered(), (0, 2));

    team.dad.has_model(id).await;
    let partly = team
        .mum
        .training(|state| {
            state
                .model(id)
                .is_some_and(|model| model.delivered() == (1, 2))
        })
        .await;
    let waiting: Vec<&str> = partly
        .model(id)
        .expect("listed")
        .delivery
        .iter()
        .filter(|delivery| !delivery.delivered)
        .map(|delivery| delivery.name.as_str())
        .collect();
    assert_eq!(waiting, ["Kid · Tablet"]);
    assert_eq!(team.kid.model_files(), 0);

    // Back on, and the person opens Training: the model is there to fetch.
    team.switch_on(team.kid.device);
    team.kid.train(TrainingCommand::Load);
    let received = team.kid.has_model(id).await;
    assert_eq!(
        received.card.origin,
        ModelOrigin::Received {
            from: team.mum.device
        }
    );
    file_is(&received, id);
    team.mum
        .training(|state| {
            state
                .model(id)
                .is_some_and(|model| model.delivered() == (2, 2))
        })
        .await;

    // Who has it is kept on disk, not only in memory.
    assert_eq!(team.kid.model_files(), 1);
    let Team { _dir, net, mum, .. } = team;
    let mum = mum.restart(&net).await;
    let reopened = mum.has_model(id).await;
    assert_eq!(reopened.delivered(), (2, 2));
    assert_eq!(reopened.card.origin, ModelOrigin::TrainedHere);
}

/// A group computer whose model parts are changed on the way out.
struct Tamperer {
    real: Arc<dyn PeerHandler>,
    /// This computer gets parts with one bit changed.
    changed_for: DeviceId,
    /// This computer is told the model is longer than its card says.
    stretched_for: DeviceId,
}

#[async_trait::async_trait]
impl PeerHandler for Tamperer {
    async fn handle(&self, from: DeviceId, request: PeerRequest) -> PeerResponse {
        let mut response = self.real.handle(from, request).await;
        if let PeerResponse::Training {
            work: TrainingPeerResponse::ModelPart { total, bytes },
        } = &mut response
        {
            if from == self.changed_for {
                let last = bytes.0.len() - 1;
                bytes.0[last] ^= 1;
            } else if from == self.stretched_for {
                *total += 4;
            }
        }
        response
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_oversized_or_unknown_model_is_refused_and_never_stored() {
    let team = Team::start(121).await;
    team.mum_starts().await;
    team.contribute(team.dad.device).await;
    team.switch_off(team.dad.device);
    team.switch_off(team.kid.device);
    let model = team.mum_finishes().await;
    let id = model.card.id;

    // Mum's computer now answers with model parts that are not the model.
    let real = team
        .net
        .handlers
        .lock()
        .expect("handlers")
        .get(&team.mum.device)
        .cloned()
        .expect("mum is listening");
    team.net.handlers.lock().expect("handlers").insert(
        team.mum.device,
        Arc::new(Tamperer {
            real: real.clone(),
            changed_for: team.dad.device,
            stretched_for: team.kid.device,
        }),
    );
    team.switch_on(team.dad.device);
    team.switch_on(team.kid.device);
    for member in [&team.dad, &team.kid] {
        member.train(TrainingCommand::Load);
        let refused = member
            .training(|state| {
                state
                    .notice
                    .as_deref()
                    .is_some_and(|notice| notice.contains("did not pass its checks"))
            })
            .await;
        assert!(
            refused
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("Mum · Laptop")),
            "{:?}",
            refused.notice
        );
        assert!(refused.models.is_empty(), "{:?}", refused.models);
        assert_eq!(member.model_files(), 0);
    }

    // A card for a model nobody has: Kid asks for it, gets nothing, keeps nothing.
    let phantom_file = encode_model(
        &ModelHeader::fresh(BaseModel::Starter),
        &vec![0.5; BYTE_MODEL_PARAMETERS],
    )
    .expect("a model file");
    let phantom = ModelCard::from_facts(
        &inspect_model(&phantom_file).expect("facts"),
        "Phantom".into(),
        "Nobody".into(),
        ModelOrigin::TrainedHere,
        Some(team.group),
        1,
    );
    let thanked = team
        .ask(
            team.dad.device,
            team.kid.device,
            TrainingPeerRequest::Announce {
                models: vec![phantom.clone()],
            },
        )
        .await;
    assert!(
        matches!(
            thanked,
            PeerResponse::Training {
                work: TrainingPeerResponse::Thanks
            }
        ),
        "{thanked:?}"
    );
    // A card that claims an impossible size is refused as a whole message.
    let mut oversized = phantom.clone();
    oversized.bytes = MAX_MODEL_BYTES as u64 + 1;
    let response = team
        .ask(
            team.dad.device,
            team.kid.device,
            TrainingPeerRequest::Announce {
                models: vec![oversized],
            },
        )
        .await;
    assert!(rejected(&response), "{response:?}");
    // A card for another group is not this group's to fetch.
    let mut elsewhere = phantom;
    elsewhere.group = Some(GroupId([8; 32]));
    team.ask(
        team.dad.device,
        team.kid.device,
        TrainingPeerRequest::Announce {
            models: vec![elsewhere],
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(team.kid.model_files(), 0);
    assert_eq!(team.dad.model_files(), 0);
    // Nothing was delivered, and Mum's page does not say otherwise.
    team.mum.train(TrainingCommand::Load);
    let mum = team.mum.training(|state| state.model(id).is_some()).await;
    assert_eq!(mum.model(id).expect("listed").delivered(), (0, 2));

    // The model itself was fine: with honest answers Dad's computer takes it
    // the next time it starts.
    team.net
        .handlers
        .lock()
        .expect("handlers")
        .insert(team.mum.device, real);
    let Team { _dir, net, dad, .. } = team;
    let dad = dad.restart(&net).await;
    let received = dad.has_model(id).await;
    file_is(&received, id);
    assert_eq!(dad.model_files(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_helping_holds_until_the_permission_is_turned_off_and_on() {
    let team = Team::start(131).await;
    team.dad.permit(true).await;
    team.mum_starts().await;
    team.dad.training(|state| rounds_given(state) >= 1).await;

    // Dad presses Stop helping.
    team.dad.train(TrainingCommand::StopHelping);
    let stopped = team
        .dad
        .training(|state| {
            state
                .helping
                .as_ref()
                .is_some_and(|help| help.phase == HelpingPhase::Stopped)
        })
        .await;
    assert!(
        stopped
            .helping
            .as_ref()
            .and_then(|help| help.notice.as_deref())
            .is_some_and(|notice| notice.contains("Help the group train")),
        "{:?}",
        stopped.helping
    );
    // Longer than it takes to look for a run again: nothing more is asked of
    // Mum's computer and nothing more is learned on Dad's.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (assigned, learned) = (team.mum.engine.assigned(), team.dad.engine.learned());
    tokio::time::sleep(Duration::from_millis(3600)).await;
    assert_eq!(team.mum.engine.assigned(), assigned);
    assert_eq!(team.dad.engine.learned(), learned);
    assert!(team.dad.store.0.lock().expect("store").help_train);

    // Off and on again is Dad asking for help to resume.
    team.dad.permit(false).await;
    team.dad.permit(true).await;
    team.dad
        .training(|state| helping(state) && rounds_given(state) >= 1)
        .await;
    assert!(team.dad.engine.learned() > learned);

    // Dad starts training of his own: the help Sovra started gives way at once.
    team.dad
        .train(TrainingCommand::Start(Box::new(training_plan(false))));
    let own = team
        .dad
        .training(|state| state.phase == TrainingPhase::Running)
        .await;
    assert!(!helping(&own));
    assert_eq!(own.sharing, None);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let assigned = team.mum.engine.assigned();
    tokio::time::sleep(Duration::from_millis(3600)).await;
    assert_eq!(
        team.mum.engine.assigned(),
        assigned,
        "a computer training its own model helps nobody"
    );
    // Mum's run was never held up by any of it.
    team.mum.train(TrainingCommand::Load);
    let mum = team
        .mum
        .training(|state| state.phase == TrainingPhase::Running)
        .await;
    assert_eq!(mum.error, None);
    assert_eq!(team.mum.engine.learned(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_the_permission_nothing_helps_and_a_removed_member_gets_nothing() {
    let team = Team::start(141).await;
    team.mum_starts().await;
    // Nobody turned "Help the group train" on. Mum's run is shared, and still
    // no computer takes work from it.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(team.mum.engine.assigned(), 0);
    for member in [&team.dad, &team.kid, &team.outsider] {
        assert_eq!(member.engine.learned(), 0);
    }

    // The switch is the only thing that was missing.
    team.dad.permit(true).await;
    team.dad.training(|state| rounds_given(state) >= 1).await;
    // Turning it off stops the help without a Stop helping.
    team.dad.permit(false).await;
    let off = team.dad.training(|state| !helping(state)).await;
    assert!(
        off.helping
            .as_ref()
            .and_then(|help| help.notice.as_deref())
            .is_some_and(|notice| notice.contains("turned off")),
        "{:?}",
        off.helping
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let assigned = team.mum.engine.assigned();
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(team.mum.engine.assigned(), assigned);

    // Mum removes Dad from the group before the model is finished.
    let dad_person = team
        .roster
        .owner_of(&team.dad.device)
        .expect("member")
        .person;
    team.mum
        .send(GroupCommand::RemovePerson { person: dad_person });
    team.mum
        .roster(|state| !state.admits(&team.dad.device))
        .await;
    let model = team.mum_finishes().await;
    let id = model.card.id;
    // Kid, still a member, gets it. Only Kid is counted.
    team.kid.has_model(id).await;
    let delivered = team
        .mum
        .training(|state| {
            state
                .model(id)
                .is_some_and(|model| model.delivered() == (1, 1))
        })
        .await;
    assert_eq!(
        delivered.model(id).expect("listed").delivery[0].device,
        team.kid.device
    );
    // Dad's computer is refused, whatever it asks.
    for request in [
        TrainingPeerRequest::Offers { have: Vec::new() },
        TrainingPeerRequest::FetchModel {
            model: id,
            offset: 0,
        },
        TrainingPeerRequest::Status,
        TrainingPeerRequest::Work,
    ] {
        assert_eq!(
            refusal(team.ask(team.dad.device, team.mum.device, request).await),
            Refusal::NotAMember
        );
    }
    team.dad.train(TrainingCommand::Load);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(team.dad.model_files(), 0);
}
