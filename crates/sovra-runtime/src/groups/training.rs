//! Authenticated group access for training; no model work runs under group locks.

use sovra_core::training::{
    TrainingGroup, TrainingPeer, TrainingPeerRequest, TrainingPeerResponse,
};

use super::{
    DeviceId, GroupId, GroupKnowledge, GroupRuntime, Groups, PeerRequest, PeerResponse, Refusal,
};

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
                    })
            })
            .take(64)
            .collect();
        Some(TrainingGroup {
            id: roster.group,
            name: roster.name.clone(),
            this_device: me,
            peers,
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

    pub(crate) async fn request(
        &self,
        group: GroupId,
        peer: DeviceId,
        work: TrainingPeerRequest,
    ) -> Result<TrainingPeerResponse, String> {
        let groups = self.0.groups().ok_or("Groups is unavailable.")?;
        let address = {
            let inner = groups.inner.lock().await;
            let allowed = inner.roster.as_ref().is_some_and(|roster| {
                roster.group == group && roster.admits(&groups.me()) && roster.admits(&peer)
            });
            if !allowed {
                return Err("That computer is no longer in your group.".into());
            }
            Groups::address_of(&inner.record, peer)
        };
        let response = groups
            .ask(&address, PeerRequest::Training { group, work })
            .await?;
        if !self.permits(group, peer).await {
            return Err("That computer is no longer in your group.".into());
        }
        match response {
            PeerResponse::Training { work } => Ok(work),
            PeerResponse::Refused {
                reason: Refusal::NotTraining,
            } => Err("That computer is not helping the group train.".into()),
            _ => Err("That computer is not sharing training right now.".into()),
        }
    }

    pub(crate) async fn set_training_help(&self, on: bool) {
        if let Some(groups) = self.0.groups() {
            groups.set_training_help(on).await;
        }
    }

    pub(crate) async fn training_help(&self) -> bool {
        match self.0.groups() {
            Some(groups) => groups.training_help().await,
            None => false,
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
