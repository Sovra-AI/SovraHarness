//! Giving finished models to the group, and receiving the group's.
//!
//! A model trained with the group, or one the person chose to share, is
//! offered to every computer of that group: each is told at once, and whoever
//! was away finds it the next time it asks around. Receiving is the same for
//! every member, whatever its "Help the group train" switch says.
//!
//! The rules a received model must pass, in order:
//!
//! - Bytes only ever arrive as the answer to this computer's own request. An
//!   announcement or an offer is a hint and carries a card, never a model.
//! - The sender is in this computer's current group, checked before and after
//!   every request, and the card names that same group.
//! - The card is valid before anything is fetched: bounded names, a supported
//!   shape, and exactly the size that shape implies.
//! - One model is fetched at a time, in parts of at most 2 MiB, each checked
//!   against the card. The whole file must hash to the card's id.
//! - The engine checks the file and must agree with the card.
//! - There is room: received models have their own limit, and nothing is ever
//!   removed to make room.
//!
//! Only then is it written, as a model "from" the sender as this computer's
//! own group list names it. A model is never run as a program.

use std::sync::Arc;

use futures::StreamExt as _;
use sovra_core::{
    group::{DeviceId, GroupId},
    training::{
        LibraryModel, MAX_MODEL_PART_BYTES, MAX_TRAINED_BY_CHARS, ModelAssembly, ModelCard,
        ModelId, TrainingEngine, TrainingPeerRequest, TrainingPeerResponse, held, model_part,
        offers, room_for_received,
    },
};
use tokio::task::JoinError;

use super::{Actor, auto::TOO_OLD_MS, library::unix_ms, storage};
use crate::{
    groups::{BULKY, GroupTraining, PeerReply, QUICK},
    private_storage::write_private,
};

/// Group computers asked at the same time, for any question sent to several.
pub(super) const FAN_OUT: usize = 4;
/// How often computers heard from lately are asked what they offer.
const POLL_EVERY_MS: u64 = 30_000;
/// Every this many asks, computers not heard from lately are asked as well.
const POLL_ALL_EVERY: u32 = 4;
/// Outside a group there is nobody to ask; look again shortly in case one is
/// joined or still loading.
const NO_GROUP_RETRY_MS: u64 = 2_000;
/// Offers kept waiting for their turn to be fetched.
const MAX_QUEUED_OFFERS: usize = 64;
/// Small sends that may be on their way at once.
const MAX_CHORES: usize = 256;
/// How long a served model file stays in memory after its last part left.
const SERVED_KEEP_MS: u64 = 30_000;
/// A model that could not be fetched right now is tried again after this.
const RETRY_LATER_MS: u64 = 60_000;
/// A model that failed its checks is left alone this long, doubling each time.
const RETRY_BAD_MS: u64 = 5 * 60_000;
const RETRY_MAX_MS: u64 = 60 * 60_000;
/// Models remembered as declined before old entries are dropped.
const MAX_DECLINED: usize = 256;

const SOMEONE: &str = "A computer in your group";

/// A model a group computer says it shares, waiting to be fetched.
pub(super) struct Offer {
    group: GroupId,
    peer: DeviceId,
    card: ModelCard,
}

/// A model that was not taken, and when it may be tried again.
pub(super) struct Declined {
    failures: u32,
    until: u64,
}

pub(super) struct Served {
    model: ModelId,
    file: Arc<Vec<u8>>,
    used_ms: u64,
}

pub(super) struct Polled {
    group: GroupId,
    offers: Vec<(DeviceId, ModelCard)>,
    too_old: Vec<DeviceId>,
}

enum Failure {
    /// The card, a part or the assembled file failed a check.
    Bad,
    /// Nothing wrong was seen; the model just could not be had right now.
    Later,
}

pub(super) struct Fetched {
    group: GroupId,
    peer: DeviceId,
    card: ModelCard,
    outcome: Result<Vec<u8>, Failure>,
}

/// Asks each computer which finished models it shares that are not here yet,
/// a few at a time.
async fn poll(
    groups: GroupTraining,
    group: GroupId,
    peers: Vec<DeviceId>,
    have: Vec<ModelId>,
) -> Polled {
    let mut answers = futures::stream::iter(peers)
        .map(|peer| {
            let groups = groups.clone();
            let have = have.clone();
            async move {
                let reply = groups
                    .ask(group, peer, TrainingPeerRequest::Offers { have }, QUICK)
                    .await;
                (peer, reply)
            }
        })
        .buffer_unordered(FAN_OUT);
    let mut polled = Polled {
        group,
        offers: Vec::new(),
        too_old: Vec::new(),
    };
    while let Some((peer, reply)) = answers.next().await {
        match reply {
            PeerReply::Answer(TrainingPeerResponse::Offered(cards)) => {
                polled.offers.extend(
                    cards
                        .into_iter()
                        .filter(|card| card.group == Some(group))
                        .map(|card| (peer, card)),
                );
            }
            PeerReply::TooOld => polled.too_old.push(peer),
            PeerReply::Answer(_)
            | PeerReply::Refused
            | PeerReply::Garbled
            | PeerReply::Unreachable => {}
        }
    }
    polled
}

/// Fetches one model part by part and has the engine check the whole file.
async fn fetch(
    groups: GroupTraining,
    engine: Arc<dyn TrainingEngine>,
    group: GroupId,
    peer: DeviceId,
    mut assembly: ModelAssembly,
) -> Fetched {
    let card = assembly.card().clone();
    // A sender cannot stretch a fetch out with tiny parts.
    let most_parts = card.bytes / MAX_MODEL_PART_BYTES as u64 + 8;
    let mut parts = 0_u64;
    let outcome = loop {
        parts += 1;
        if parts > most_parts {
            break Err(Failure::Bad);
        }
        match groups.ask(group, peer, assembly.request(), BULKY).await {
            PeerReply::Answer(TrainingPeerResponse::ModelPart { total, bytes }) => {
                match assembly.accept(total, &bytes) {
                    Ok(Some(file)) => break Ok(file),
                    Ok(None) => {}
                    Err(_) => break Err(Failure::Bad),
                }
            }
            PeerReply::Garbled => break Err(Failure::Bad),
            // Not on offer there (any more), busy, away, or no longer in the group.
            PeerReply::Answer(_)
            | PeerReply::TooOld
            | PeerReply::Refused
            | PeerReply::Unreachable => break Err(Failure::Later),
        }
    };
    let outcome = match outcome {
        Ok(file) => {
            let expected = card.clone();
            tokio::task::spawn_blocking(move || match engine.inspect(&file) {
                Ok(facts) if expected.agrees_with(&facts) => Ok(file),
                Ok(_) | Err(_) => Err(Failure::Bad),
            })
            .await
            .unwrap_or(Err(Failure::Bad))
        }
        Err(failure) => Err(failure),
    };
    Fetched {
        group,
        peer,
        card,
        outcome,
    }
}

impl Actor {
    // ---- This computer's models, for the group -------------------------

    /// Answers a group computer asking about finished models. None of this
    /// needs the run, so it is answered whether or not training is going.
    pub(super) async fn serve_delivery(
        &mut self,
        group: GroupId,
        from: DeviceId,
        work: TrainingPeerRequest,
    ) -> TrainingPeerResponse {
        if self.config.is_none() || !self.groups.permits(group, from).await {
            return TrainingPeerResponse::Unavailable;
        }
        match work {
            TrainingPeerRequest::Offers { have } => {
                // What it already holds counts as delivered, so a lost
                // "received" is made up for the next time it asks.
                self.note_delivered(group, from, &have).await;
                TrainingPeerResponse::Offered(offers(&self.state.models, group, &have))
            }
            TrainingPeerRequest::FetchModel { model, offset } => {
                self.serve_part(group, from, model, offset).await
            }
            TrainingPeerRequest::Received { model } => {
                if !self.offers_model(model, group) {
                    return TrainingPeerResponse::Unavailable;
                }
                self.note_delivered(group, from, &[model]).await;
                TrainingPeerResponse::Thanks
            }
            TrainingPeerRequest::Announce { models } => {
                // Only a hint. This computer fetches what it wants itself.
                for card in models {
                    self.note_offer(group, from, card);
                }
                TrainingPeerResponse::Thanks
            }
            TrainingPeerRequest::Work
            | TrainingPeerRequest::Submit(_)
            | TrainingPeerRequest::Status => TrainingPeerResponse::Unavailable,
        }
    }

    /// Whether `model` was trained here and is shared with exactly `group`.
    fn offers_model(&self, model: ModelId, group: GroupId) -> bool {
        self.state
            .model(model)
            .is_some_and(|entry| entry.offered_to(group))
    }

    async fn serve_part(
        &mut self,
        group: GroupId,
        from: DeviceId,
        model: ModelId,
        offset: u64,
    ) -> TrainingPeerResponse {
        if !self.offers_model(model, group) {
            return TrainingPeerResponse::Unavailable;
        }
        let now = self.now_ms();
        let file = match &mut self.served {
            Some(served) if served.model == model => {
                served.used_ms = now;
                served.file.clone()
            }
            Some(_) | None => {
                let Some(root) = self.root() else {
                    return TrainingPeerResponse::Unavailable;
                };
                // The reader checks the file is still the model its name says.
                let read =
                    tokio::task::spawn_blocking(move || storage::read_model(&root, model)).await;
                let Ok(Ok(bytes)) = read else {
                    tracing::warn!(model = %model.to_hex(), "a shared model's file could not be read");
                    return TrainingPeerResponse::Unavailable;
                };
                let file = Arc::new(bytes);
                self.served = Some(Served {
                    model,
                    file: file.clone(),
                    used_ms: now,
                });
                file
            }
        };
        // Membership can change while the file is being read.
        if !self.groups.permits(group, from).await || !self.offers_model(model, group) {
            return TrainingPeerResponse::Unavailable;
        }
        model_part(&file, offset)
    }

    /// Records that `from` has these models, for the ones shared with `group`
    /// from here, and saves that beside each model.
    async fn note_delivered(&mut self, group: GroupId, from: DeviceId, models: &[ModelId]) {
        let pending: Vec<LibraryModel> = self
            .state
            .models
            .iter()
            .filter(|entry| {
                entry.offered_to(group)
                    && models.contains(&entry.card.id)
                    && !entry
                        .delivery
                        .iter()
                        .any(|delivery| delivery.device == from && delivery.delivered)
            })
            .cloned()
            .collect();
        if pending.is_empty() {
            return;
        }
        let devices = self.group_devices(group).await;
        for mut entry in pending {
            if !entry
                .delivery
                .iter()
                .any(|delivery| delivery.device == from)
            {
                entry.sync_delivery(&devices);
            }
            if entry.mark_delivered(from) {
                self.replace_model(entry).await;
            }
        }
        self.unpublished = true;
    }

    /// Puts a changed library entry in the list and its card on disk.
    async fn replace_model(&mut self, entry: LibraryModel) {
        let Some(root) = self.root() else {
            return;
        };
        let id = entry.card.id;
        if let Err(error) = write_private(
            &storage::models_directory(&root),
            &storage::card_filename(id),
            &entry,
        )
        .await
        {
            // The list still shows it; the next time that computer asks, the
            // card is written again.
            tracing::warn!(model = %id.to_hex(), %error, "a model's delivery record could not be saved");
        }
        if let Some(listed) = self
            .state
            .models
            .iter_mut()
            .find(|listed| listed.card.id == id)
        {
            *listed = entry;
        }
    }

    /// Keeps "On N of M computers" in line with who is in the group now, for
    /// every model shared with it from here. Returns whether anything changed.
    pub(super) async fn sync_deliveries(&mut self) -> bool {
        let Some(group) = self.groups.snapshot().await else {
            return false;
        };
        let devices: Vec<(DeviceId, String)> = group
            .peers
            .iter()
            .map(|peer| (peer.device, peer.name.clone()))
            .collect();
        let changed: Vec<LibraryModel> = self
            .state
            .models
            .iter()
            .filter(|entry| entry.offered_to(group.id))
            .filter_map(|entry| {
                let mut synced = entry.clone();
                synced.sync_delivery(&devices);
                (synced != *entry).then_some(synced)
            })
            .collect();
        let any = !changed.is_empty();
        for entry in changed {
            self.replace_model(entry).await;
        }
        any
    }

    /// Tells every computer of the group that does not have `model` yet that
    /// it is there to fetch: each on its own small owned task, a few at a
    /// time, with no group lock held. A computer that is away finds the model
    /// later, when it asks around.
    pub(super) async fn announce(&mut self, model: ModelId) {
        let Some(group) = self.groups.snapshot().await else {
            return;
        };
        let Some(entry) = self
            .state
            .model(model)
            .filter(|entry| entry.offered_to(group.id))
        else {
            return;
        };
        let card = entry.card.offered_to(group.id);
        let now = self.now_ms();
        let waiting: Vec<DeviceId> = group
            .peers
            .iter()
            .map(|peer| peer.device)
            .filter(|device| {
                !entry
                    .delivery
                    .iter()
                    .any(|delivery| delivery.device == *device && delivery.delivered)
                    && self.too_old.get(device).is_none_or(|until| *until <= now)
            })
            .collect();
        for device in waiting {
            self.chore(
                group.id,
                device,
                TrainingPeerRequest::Announce {
                    models: vec![card.clone()],
                },
            );
        }
    }

    /// Sends one small message whose answer is not needed, on an owned task.
    fn chore(&mut self, group: GroupId, peer: DeviceId, work: TrainingPeerRequest) {
        if self.chores.len() >= MAX_CHORES {
            // The next ask-around makes up for a message that was not sent.
            return;
        }
        let groups = self.groups.clone();
        let gate = self.chore_gate.clone();
        self.chores.spawn(async move {
            let Ok(_turn) = gate.acquire().await else {
                return;
            };
            let _ = groups.ask(group, peer, work, QUICK).await;
        });
    }

    // ---- The group's models, for this computer -------------------------

    /// Ask around on the next tick, including computers not heard from lately.
    pub(super) fn poll_soon(&mut self) {
        self.poll_after = 0;
        self.poll_round = 0;
    }

    /// Starts the next fetch and the next ask-around when they are due.
    /// Returns whether the state changed.
    pub(super) async fn tick_delivery(&mut self) -> bool {
        if self.config.is_none() {
            return false;
        }
        let now = self.now_ms();
        if self
            .served
            .as_ref()
            .is_some_and(|served| now.saturating_sub(served.used_ms) > SERVED_KEEP_MS)
        {
            self.served = None;
        }
        let changed = self.fetch_next().await;
        if self.poll_task.is_none() && now >= self.poll_after {
            self.start_poll(now).await;
        }
        changed
    }

    async fn start_poll(&mut self, now: u64) {
        let Some(group) = self.groups.snapshot().await else {
            self.poll_after = now.saturating_add(NO_GROUP_RETRY_MS);
            return;
        };
        let everyone = self.poll_round.is_multiple_of(POLL_ALL_EVERY);
        self.poll_round = self.poll_round.wrapping_add(1);
        self.poll_after = now.saturating_add(POLL_EVERY_MS);
        self.too_old.retain(|_, until| *until > now);
        let peers: Vec<DeviceId> = group
            .peers
            .iter()
            .filter(|peer| (everyone || peer.online) && !self.too_old.contains_key(&peer.device))
            .map(|peer| peer.device)
            .collect();
        if peers.is_empty() {
            return;
        }
        let have = held(&self.state.models);
        let groups = self.groups.clone();
        self.poll_task =
            Some(tokio::runtime::Handle::current().spawn(poll(groups, group.id, peers, have)));
    }

    pub(super) async fn finish_poll(&mut self, result: Result<Polled, JoinError>) -> bool {
        let Ok(Polled {
            group,
            offers,
            too_old,
        }) = result
        else {
            return false;
        };
        let until = self.now_ms().saturating_add(TOO_OLD_MS);
        for device in too_old {
            self.too_old.insert(device, until);
        }
        for (peer, card) in offers {
            self.note_offer(group, peer, card);
        }
        let fetching = self.fetch_next().await;
        self.sync_deliveries().await || fetching
    }

    /// Remembers a model a group computer says it shares. Nothing is fetched
    /// here, and nothing about the card is trusted yet.
    fn note_offer(&mut self, group: GroupId, peer: DeviceId, card: ModelCard) {
        if card.group != Some(group)
            || self.offered.len() >= MAX_QUEUED_OFFERS
            || self.offered.iter().any(|offer| offer.card.id == card.id)
        {
            return;
        }
        self.offered.push_back(Offer { group, peer, card });
    }

    /// Starts fetching the next offered model, if none is being fetched.
    async fn fetch_next(&mut self) -> bool {
        let mut changed = false;
        while self.fetch_task.is_none()
            && let Some(offer) = self.offered.pop_front()
        {
            changed |= self.start_fetch(offer).await;
        }
        changed
    }

    async fn start_fetch(&mut self, offer: Offer) -> bool {
        let Offer { group, peer, card } = offer;
        let id = card.id;
        let now = self.now_ms();
        if self.state.model(id).is_some() {
            // Already here. Say so, in case the first "received" was lost.
            self.chore(group, peer, TrainingPeerRequest::Received { model: id });
            return false;
        }
        if self
            .declined
            .get(&id)
            .is_some_and(|declined| now < declined.until)
        {
            return false;
        }
        let Some(engine) = self.engine() else {
            return false;
        };
        if !self.groups.permits(group, peer).await {
            return false;
        }
        let who = self.peer_name(group, peer).await;
        let bytes = card.bytes;
        let Ok(assembly) = ModelAssembly::new(card, group) else {
            self.decline(id, RETRY_BAD_MS);
            self.state.notice = Some(failed_checks(&who));
            return true;
        };
        if !room_for_received(&self.state.models, bytes) {
            self.decline(id, RETRY_MAX_MS);
            self.state.notice = Some(no_room(&assembly.card().name));
            return true;
        }
        let groups = self.groups.clone();
        self.fetch_task = Some(
            tokio::runtime::Handle::current().spawn(fetch(groups, engine, group, peer, assembly)),
        );
        false
    }

    /// The fetch is back. A model that passed every check is stored as
    /// received from that computer; anything else is left out and said so.
    pub(super) async fn finish_fetch(&mut self, result: Result<Fetched, JoinError>) {
        let Ok(Fetched {
            group,
            peer,
            card,
            outcome,
        }) = result
        else {
            return;
        };
        let id = card.id;
        let who = self.peer_name(group, peer).await;
        let file = match outcome {
            Ok(file) => file,
            Err(Failure::Later) => {
                self.decline(id, RETRY_LATER_MS);
                return;
            }
            Err(Failure::Bad) => {
                self.decline(id, RETRY_BAD_MS);
                self.state.notice = Some(failed_checks(&who));
                return;
            }
        };
        // The sender may have left the group while the model was on its way.
        if !self.groups.permits(group, peer).await {
            return;
        }
        if !room_for_received(&self.state.models, card.bytes) {
            self.decline(id, RETRY_MAX_MS);
            self.state.notice = Some(no_room(&card.name));
            return;
        }
        let mut received = card.received_from(peer);
        // Who it is from is this computer's own word, not the sender's.
        received.trained_by = who.clone();
        received.created_unix_ms = received.created_unix_ms.min(unix_ms());
        if received.validate().is_err() {
            self.decline(id, RETRY_BAD_MS);
            self.state.notice = Some(failed_checks(&who));
            return;
        }
        let name = received.name.clone();
        match self.store_model(received, file).await {
            Ok(()) => {
                self.declined.remove(&id);
                self.state.notice = Some(format!("{name} from {who} is now in Your models."));
                self.chore(group, peer, TrainingPeerRequest::Received { model: id });
            }
            Err(error) => {
                self.decline(id, RETRY_LATER_MS);
                self.state.error = Some(format!(
                    "A model from your group could not be saved: {error}"
                ));
            }
        }
    }

    /// "Person · Computer" for a group computer as this computer's own group
    /// list has it.
    async fn peer_name(&mut self, group: GroupId, peer: DeviceId) -> String {
        self.group_devices(group)
            .await
            .into_iter()
            .find(|(device, _)| *device == peer)
            .map_or_else(
                || SOMEONE.to_owned(),
                |(_, name)| {
                    name.chars()
                        .filter(|c| !c.is_control())
                        .take(MAX_TRAINED_BY_CHARS)
                        .collect()
                },
            )
    }

    /// Leaves a model alone for a while. Each time it fails again, longer.
    fn decline(&mut self, model: ModelId, wait_ms: u64) {
        let now = self.now_ms();
        if self.declined.len() >= MAX_DECLINED {
            self.declined.retain(|_, declined| declined.until > now);
        }
        if self.declined.len() >= MAX_DECLINED {
            self.declined.pop_first();
        }
        let declined = self.declined.entry(model).or_insert(Declined {
            failures: 0,
            until: 0,
        });
        let wait = wait_ms
            .saturating_mul(1 << declined.failures.min(4))
            .min(RETRY_MAX_MS);
        declined.failures = declined.failures.saturating_add(1);
        declined.until = now.saturating_add(wait);
    }
}

fn failed_checks(who: &str) -> String {
    format!("A model offered by {who} did not pass its checks, so it was not kept.")
}

fn no_room(name: &str) -> String {
    format!(
        "Your models has no room for more models from your group, so {name} was not fetched. Nothing was removed."
    )
}
