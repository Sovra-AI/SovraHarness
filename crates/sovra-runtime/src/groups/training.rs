//! Authenticated group access for training; no model work runs under group locks.

use sovra_core::training::{
    MAX_TRAINING_PEERS, TrainingGroup, TrainingPeer, TrainingPeerRequest, TrainingPeerResponse,
};

use super::{
    DeviceId, Duration, GroupId, GroupKnowledge, GroupRuntime, Groups, PeerRequest, PeerResponse,
    Refusal,
};

/// Time allowed for a small question: is a run going, which models are offered.
pub(crate) const QUICK: Duration = Duration::from_secs(5);
/// Time allowed for a message that carries model values: a work packet, its
/// result, or a part of a finished model. The other computer may also have to
/// finish the round it is on and save before it answers.
pub(crate) const BULKY: Duration = Duration::from_secs(60);

/// What came back from a training question to another computer of the group.
#[derive(Debug)]
pub(crate) enum PeerReply {
    /// A checked answer.
    Answer(TrainingPeerResponse),
    /// The computer runs a Sovra that does not know this question. Ask it
    /// again much later, and show nothing alarming.
    TooOld,
    /// One of the two computers is not in the group, or the other refused.
    Refused,
    /// The answer broke the message bounds. Nothing in it is used.
    Garbled,
    /// No answer in time.
    Unreachable,
}

#[derive(Clone)]
pub(crate) struct GroupTraining(GroupKnowledge);

impl GroupRuntime {
    pub(crate) fn training(&self) -> GroupTraining {
        GroupTraining(self.knowledge.clone())
    }
}

impl GroupTraining {
    pub(crate) fn attach(&self, inbox: crate::training::PeerInbox) {
        *self.0.4.write().expect("training inbox lock") = Some(inbox);
    }

    pub(crate) async fn snapshot(&self) -> Option<TrainingGroup> {
        let groups = self.0.groups()?;
        let inner = groups.inner.lock().await;
        let roster = inner.roster.as_ref()?;
        let me = groups.me();
        if !roster.admits(&me) {
            return None;
        }
        let now = super::now().millis();
        let peers = roster
            .people
            .values()
            .flat_map(|person| {
                person
                    .devices
                    .iter()
                    .filter(|(id, _)| **id != me)
                    .map(|(id, device)| TrainingPeer {
                        device: *id,
                        name: format!("{} · {}", person.name, device.name),
                        online: inner.last_seen.get(id).is_some_and(|seen| {
                            now.saturating_sub(seen.millis()) < sovra_core::group::ONLINE_WINDOW_MS
                        }),
                        // The training actor fills these in for its own run.
                        helping: false,
                        rounds: 0,
                    })
            })
            .take(MAX_TRAINING_PEERS)
            .collect();
        Some(TrainingGroup {
            id: roster.group,
            name: roster.name.clone(),
            this_device: me,
            peers,
        })
    }

    /// "Person · Computer" for this computer, as its own group list has it.
    /// `None` outside a group.
    pub(crate) async fn identity(&self) -> Option<String> {
        let groups = self.0.groups()?;
        let inner = groups.inner.lock().await;
        let roster = inner.roster.as_ref()?;
        let me = groups.me();
        roster.people.values().find_map(|person| {
            person
                .devices
                .get(&me)
                .map(|device| format!("{} · {}", person.name, device.name))
        })
    }

    pub(crate) async fn permits(&self, group: GroupId, peer: DeviceId) -> bool {
        let Some(groups) = self.0.groups() else {
            return false;
        };
        let inner = groups.inner.lock().await;
        inner.roster.as_ref().is_some_and(|roster| {
            roster.group == group && roster.admits(&groups.me()) && roster.admits(&peer)
        })
    }

    /// Whether this computer's person has turned on "Help the group train".
    /// It lends this computer's processor to a run another computer started;
    /// it never opens this computer's own run. The number counts how often it
    /// was turned on since the app started.
    pub(crate) async fn help_permitted(&self) -> (bool, u64) {
        let Some(groups) = self.0.groups() else {
            return (false, 0);
        };
        let inner = groups.inner.lock().await;
        (inner.record.help_train, inner.help_turned_on)
    }

    /// Asks one computer of the group a training question. Both computers must
    /// be in `group` before the question leaves and still when the answer is
    /// back, and the answer's bounded parts are checked before it is returned.
    /// No group lock is held while the question is out.
    pub(crate) async fn ask(
        &self,
        group: GroupId,
        peer: DeviceId,
        work: TrainingPeerRequest,
        limit: Duration,
    ) -> PeerReply {
        let Some(groups) = self.0.groups() else {
            return PeerReply::Unreachable;
        };
        let address = {
            let inner = groups.inner.lock().await;
            let allowed = inner.roster.as_ref().is_some_and(|roster| {
                roster.group == group && roster.admits(&groups.me()) && roster.admits(&peer)
            });
            if !allowed {
                return PeerReply::Refused;
            }
            Groups::address_of(&inner.record, peer)
        };
        let Ok(response) = groups
            .ask_within(&address, PeerRequest::Training { group, work }, limit)
            .await
        else {
            return PeerReply::Unreachable;
        };
        if !self.permits(group, peer).await {
            return PeerReply::Refused;
        }
        match response {
            PeerResponse::Training { work } if work.validate().is_ok() => PeerReply::Answer(work),
            PeerResponse::Training { .. } => PeerReply::Garbled,
            // A computer on the first training release with its switch off.
            PeerResponse::Refused {
                reason: Refusal::NotTraining,
            } => PeerReply::Answer(TrainingPeerResponse::Unavailable),
            // It could not read the question: it does not know this message.
            PeerResponse::Refused {
                reason: Refusal::Malformed,
            } => PeerReply::TooOld,
            _ => PeerReply::Refused,
        }
    }
}

impl GroupKnowledge {
    pub(super) async fn training_request(
        &self,
        group: GroupId,
        from: DeviceId,
        work: TrainingPeerRequest,
    ) -> PeerResponse {
        let inbox = self.4.read().expect("training inbox lock").clone();
        let work = match inbox {
            Some(inbox) => inbox.request(group, from, work).await,
            None => TrainingPeerResponse::Unavailable,
        };
        PeerResponse::Training { work }
    }
}
