//! Helping the group's training without being asked each time.
//!
//! When this computer's person has turned on "Help the group train", Sovra
//! looks for a run a group computer started with the group and lends this
//! computer's processor to it. All of these must hold, and are looked at again
//! on every tick:
//!
//! - the permission is on;
//! - this computer is not training its own run (nor holding a paused one it
//!   shares with the group), not already helping, not writing a model trial,
//!   and no chat turn is active;
//! - the person has not pressed Stop helping since the app started (turning
//!   the permission off and on again clears that);
//! - this computer is in a group, and the other computer is in it too;
//! - that computer answered, in this app session, that it is running training
//!   its group may help with.
//!
//! Helping sends only numbers worked out from the packet the owner sent. None
//! of this computer's own text or files ever leaves it this way.

use futures::StreamExt as _;
use sovra_core::{
    group::DeviceId,
    training::{
        HelpingPhase, RunSummary, TrainingGroup, TrainingPeer, TrainingPeerRequest,
        TrainingPeerResponse,
    },
};
use tokio::task::JoinError;

use super::{Actor, delivery::FAN_OUT};
use crate::groups::{GroupTraining, PeerReply, QUICK};

/// How often the group is asked whether anyone is training with it.
const SCOUT_EVERY_MS: u64 = 3_000;
/// Every this many looks, computers not heard from lately are asked as well.
/// An unreachable computer costs a full wait, so it is not asked every time.
const SCOUT_ALL_EVERY: u32 = 10;
/// A computer on an older Sovra is left alone this long.
pub(super) const TOO_OLD_MS: u64 = 10 * 60 * 1000;
/// Times in a row a run helped automatically may have nothing to give before
/// this computer lets go of it and looks again.
const MAX_MISSES: u32 = 3;

/// What the actor remembers about helping by itself.
#[derive(Debug, Default)]
pub(super) struct AutoHelp {
    /// The help under way was started by Sovra under the person's "Help the
    /// group train" permission, not by the person choosing a computer.
    pub(super) automatic: bool,
    /// The person pressed Stop helping. Nothing is helped automatically again
    /// until the app restarts or the permission is turned off and on.
    pub(super) latched: bool,
    /// The permission as it was at the last look.
    pub(super) permitted: bool,
    /// How often the permission had been turned on at the last look.
    pub(super) turned_on: u64,
    /// Times in a row the run being helped automatically had nothing to give.
    pub(super) misses: u32,
}

/// A group computer that said it is training with the group.
pub(super) struct Sighting {
    group: TrainingGroup,
    peer: TrainingPeer,
    run: RunSummary,
}

pub(super) struct Scouted {
    found: Option<Sighting>,
    too_old: Vec<DeviceId>,
}

/// Asks each candidate whether it is running training the group may help with,
/// a few at a time, and stops at the first that is.
async fn scout(
    groups: GroupTraining,
    group: TrainingGroup,
    candidates: Vec<TrainingPeer>,
) -> Scouted {
    let id = group.id;
    let mut answers = futures::stream::iter(candidates)
        .map(|peer| {
            let groups = groups.clone();
            async move {
                let reply = groups
                    .ask(id, peer.device, TrainingPeerRequest::Status, QUICK)
                    .await;
                (peer, reply)
            }
        })
        .buffer_unordered(FAN_OUT);
    let mut too_old = Vec::new();
    let mut found = None;
    while let Some((peer, reply)) = answers.next().await {
        match reply {
            PeerReply::Answer(TrainingPeerResponse::Running(run)) => {
                found = Some((peer, run));
                break;
            }
            PeerReply::TooOld => too_old.push(peer.device),
            PeerReply::Answer(_)
            | PeerReply::Refused
            | PeerReply::Garbled
            | PeerReply::Unreachable => {}
        }
    }
    drop(answers);
    Scouted {
        found: found.map(|(peer, run)| Sighting { group, peer, run }),
        too_old,
    }
}

impl Actor {
    /// Whether this computer is free to lend its processor right now.
    fn free_to_help(&self) -> bool {
        self.config.is_some()
            && self.auto.permitted
            && !self.auto.latched
            && !self.state.busy()
            // A paused run that is shared with the group keeps that choice;
            // helping someone else would end it.
            && self.state.sharing.is_none()
            && self.slice.is_none()
            && self.help_task.is_none()
            && self.trial_task.is_none()
    }

    fn helping_automatically(&self) -> bool {
        self.auto.automatic
            && self
                .state
                .helping
                .as_ref()
                .is_some_and(|help| help.phase.active())
    }

    /// Looks at the permission and, when everything allows it, starts looking
    /// for a run to help. Returns whether the state changed.
    pub(super) async fn tick_auto(&mut self, idle: bool) -> bool {
        if self.config.is_none() {
            return false;
        }
        let (permitted, turned_on) = self.groups.help_permitted().await;
        if turned_on != self.auto.turned_on {
            // Turned on (again): that is the person asking for help to resume.
            self.auto.turned_on = turned_on;
            self.auto.latched = false;
            self.scout_after = 0;
            self.scout_round = 0;
        }
        self.auto.permitted = permitted;
        if !permitted {
            if !self.helping_automatically() {
                return false;
            }
            self.stop_help();
            if let Some(help) = &mut self.state.helping {
                help.notice =
                    Some("Helping stopped because Help the group train was turned off.".into());
            }
            return true;
        }
        let now = self.now_ms();
        if !idle || !self.free_to_help() || self.scout_task.is_some() || now < self.scout_after {
            return false;
        }
        let Some(group) = self.groups.snapshot().await else {
            return false;
        };
        self.too_old.retain(|_, until| *until > now);
        let everyone = self.scout_round.is_multiple_of(SCOUT_ALL_EVERY);
        self.scout_round = self.scout_round.wrapping_add(1);
        self.scout_after = now.saturating_add(SCOUT_EVERY_MS);
        let candidates: Vec<TrainingPeer> = group
            .peers
            .iter()
            .filter(|peer| (everyone || peer.online) && !self.too_old.contains_key(&peer.device))
            .cloned()
            .collect();
        if candidates.is_empty() {
            return false;
        }
        let groups = self.groups.clone();
        self.scout_task =
            Some(tokio::runtime::Handle::current().spawn(scout(groups, group, candidates)));
        false
    }

    /// The look is back. Everything is checked again before help starts,
    /// because the person may have done something while it was out.
    pub(super) async fn finish_scout(&mut self, result: Result<Scouted, JoinError>) -> bool {
        let Ok(Scouted { found, too_old }) = result else {
            return false;
        };
        let until = self.now_ms().saturating_add(TOO_OLD_MS);
        for device in too_old {
            self.too_old.insert(device, until);
        }
        let Some(Sighting { group, peer, run }) = found else {
            return false;
        };
        if !self.free_to_help()
            || crate::model_priority::busy()
            || !self.groups.help_permitted().await.0
            || !self.groups.permits(group.id, peer.device).await
        {
            return false;
        }
        self.begin_help(group, peer, Some(run), true);
        self.tick_help().await;
        true
    }

    /// The person pressed Stop helping: stop, and do not start again unasked.
    pub(super) fn stop_helping(&mut self) {
        self.auto.latched = true;
        if let Some(task) = self.scout_task.take() {
            // Nothing is being helped yet; the look is simply given up.
            task.abort();
        }
        let automatic = self.helping_automatically();
        self.stop_help();
        if automatic && let Some(help) = &mut self.state.helping {
            help.notice = Some(
                "Helping is off until you restart Sovra, or turn Help the group train off and on again in Groups."
                    .into(),
            );
        }
    }

    /// This computer's person starts or resumes their own training: help that
    /// Sovra started by itself gives way at once. Help the person chose stays,
    /// and the caller refuses instead.
    pub(super) fn yield_auto_help(&mut self) {
        if !self.helping_automatically() {
            return;
        }
        self.stop_help();
        if let Some(help) = &mut self.state.helping {
            help.phase = HelpingPhase::Stopped;
            help.notice =
                Some("Helping stopped because this computer started its own training.".into());
        }
    }

    /// Whether the person's own training has to wait for help to be stopped:
    /// only help the person chose themselves holds it up.
    pub(super) fn held_by_help(&self) -> bool {
        self.state
            .helping
            .as_ref()
            .is_some_and(|help| help.phase.active())
            && !self.auto.automatic
    }

    /// The run being helped automatically had nothing to give. After a few
    /// times in a row it is let go, so another run can be found.
    pub(super) fn missed_help(&mut self) {
        if !self.auto.automatic {
            return;
        }
        self.auto.misses = self.auto.misses.saturating_add(1);
        if self.auto.misses < MAX_MISSES {
            return;
        }
        self.auto.misses = 0;
        self.scout_after = 0;
        if let Some(help) = &mut self.state.helping {
            help.phase = if help
                .progress
                .as_ref()
                .is_some_and(sovra_core::training::TrainingProgress::is_complete)
            {
                HelpingPhase::Finished
            } else {
                HelpingPhase::Stopped
            };
            help.notice = Some("That training is not running with the group any more.".into());
        }
    }
}
