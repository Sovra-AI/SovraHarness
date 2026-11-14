//! The group's side of training: serving work packets for a run this
//! computer's person started with the group, and helping another computer's run
//! with one owned, cancellable packet at a time.
//!
//! Nothing here is opened by a saved setting. Work is served only while
//! `state.sharing` names the asking computer's group, and that is set by one
//! thing: an explicit start with the group in this app session.

use std::sync::atomic::{AtomicBool, Ordering};

use sovra_core::{
    group::{DeviceId, GroupId},
    training::{
        BaseModel, HelpingPhase, RunSummary, TrainingBase, TrainingEngine, TrainingError,
        TrainingGroup, TrainingHelp, TrainingPeer, TrainingPeerRequest, TrainingPeerResponse,
        TrainingPhase, TrainingProgress,
    },
};

use super::{Actor, Arc, Duration, Receiver, Sender};
use crate::groups::{BULKY, PeerReply};

/// Requests the group may have queued for the actor at once. A full queue
/// answers "unavailable", which every caller treats as "ask again later".
const INBOX: usize = 64;
/// Requests that may wait for the run to come back from a slice.
const MAX_WAITING: usize = 64;
/// Packets a helper may take in a row without returning one. Each packet costs
/// the owner a full save, so a computer that only ever asks is made to wait.
const MAX_UNANSWERED: u32 = 3;
const UNANSWERED_COOL_DOWN_MS: u64 = 30_000;
/// How long a group computer's request may wait for this computer's answer.
/// The asking side allows a little longer for the trip itself.
const SERVE_WITHIN: Duration = Duration::from_secs(15);
const SERVE_WORK_WITHIN: Duration = Duration::from_secs(50);

#[derive(Clone)]
pub(crate) struct PeerInbox(Sender<PeerWork>);

pub(super) struct PeerWork {
    group: GroupId,
    from: DeviceId,
    work: TrainingPeerRequest,
    reply: tokio::sync::oneshot::Sender<TrainingPeerResponse>,
}

impl PeerInbox {
    pub(super) fn new() -> (Self, Receiver<PeerWork>) {
        let (sender, receiver) = async_channel::bounded(INBOX);
        (Self(sender), receiver)
    }

    pub(crate) async fn request(
        &self,
        group: GroupId,
        from: DeviceId,
        work: TrainingPeerRequest,
    ) -> TrainingPeerResponse {
        let (reply, receive) = tokio::sync::oneshot::channel();
        let limit = match &work {
            // A packet or a result may have to wait for the round in progress
            // and a save. Everything else is answered straight away.
            TrainingPeerRequest::Work | TrainingPeerRequest::Submit(_) => SERVE_WORK_WITHIN,
            TrainingPeerRequest::Status
            | TrainingPeerRequest::Offers { .. }
            | TrainingPeerRequest::FetchModel { .. }
            | TrainingPeerRequest::Received { .. }
            | TrainingPeerRequest::Announce { .. } => SERVE_WITHIN,
        };
        if self
            .0
            .try_send(PeerWork {
                group,
                from,
                work,
                reply,
            })
            .is_err()
        {
            return TrainingPeerResponse::Unavailable;
        }
        tokio::time::timeout(limit, receive)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(TrainingPeerResponse::Unavailable)
    }
}

pub(super) enum HelpOutcome {
    Accepted(TrainingProgress),
    /// The other computer had no work to give or did not answer.
    Away(String),
    /// Something on this side held the packet up; ask again shortly.
    Waiting(String),
    Stopped(String),
    Cancelled,
}

impl Actor {
    pub(super) async fn receive_peer(&mut self, incoming: PeerWork) {
        let PeerWork {
            group,
            from,
            work,
            reply,
        } = incoming;
        if work.validate().is_err() {
            let _ = reply.send(TrainingPeerResponse::Rejected);
            return;
        }
        let response = match &work {
            TrainingPeerRequest::Work | TrainingPeerRequest::Submit(_) => {
                if !self.serves(group, from).await {
                    TrainingPeerResponse::Unavailable
                } else if self.session.is_some() {
                    self.peer_work(group, from, work).await
                } else if self.slice.is_some() && self.waiting.len() < MAX_WAITING {
                    // The run is out training. Ask for it back after the round
                    // it is on, and answer then.
                    self.halt.store(true, Ordering::SeqCst);
                    self.waiting.push_back(PeerWork {
                        group,
                        from,
                        work,
                        reply,
                    });
                    return;
                } else {
                    TrainingPeerResponse::Unavailable
                }
            }
            TrainingPeerRequest::Status => self.status(group, from).await,
            // Finished models need no run and no switch: every member of
            // the group may ask for what was shared with it.
            TrainingPeerRequest::Offers { .. }
            | TrainingPeerRequest::FetchModel { .. }
            | TrainingPeerRequest::Received { .. }
            | TrainingPeerRequest::Announce { .. } => self.serve_delivery(group, from, work).await,
        };
        let _ = reply.send(response);
    }

    /// Whether `from` may be given work of the current run: the run was started
    /// with exactly this group in this app session, it is running, and both
    /// computers are still in the group.
    async fn serves(&mut self, group: GroupId, from: DeviceId) -> bool {
        self.state.sharing == Some(group)
            && self.state.phase == TrainingPhase::Running
            && self.groups.permits(group, from).await
    }

    /// Tells a group computer what is being trained, so it can offer to help.
    async fn status(&mut self, group: GroupId, from: DeviceId) -> TrainingPeerResponse {
        if !self.serves(group, from).await {
            return TrainingPeerResponse::Unavailable;
        }
        match self.run_summary() {
            Some(summary) if summary.validate().is_ok() => TrainingPeerResponse::Running(summary),
            Some(_) | None => TrainingPeerResponse::Unavailable,
        }
    }

    fn run_summary(&self) -> Option<RunSummary> {
        let plan = self.state.plan.as_ref()?;
        let base = match plan.base {
            TrainingBase::Fresh(model) => model,
            TrainingBase::Continue(model) => self.state.model(model).map_or(
                if plan.architecture.is_table() {
                    BaseModel::Starter
                } else {
                    BaseModel::Small
                },
                |entry| entry.card.base,
            ),
        };
        Some(RunSummary {
            name: plan.name.clone(),
            base,
            architecture: plan.architecture,
            rounds: self.state.progress.rounds,
            total_rounds: self.state.progress.total_rounds,
        })
    }

    /// Answers the requests that waited for the run to come back.
    pub(super) async fn serve_waiting(&mut self) {
        while let Some(waiting) = self.waiting.pop_front() {
            let PeerWork {
                group,
                from,
                work,
                reply,
            } = waiting;
            let response = if self.session.is_some() && self.serves(group, from).await {
                self.peer_work(group, from, work).await
            } else {
                TrainingPeerResponse::Unavailable
            };
            let _ = reply.send(response);
        }
    }

    /// Hands out one work packet, or merges one result. The run must be here.
    /// Whatever a helper sends, the owner's run carries on: a bad result is
    /// refused and nothing else changes.
    async fn peer_work(
        &mut self,
        group: GroupId,
        from: DeviceId,
        work: TrainingPeerRequest,
    ) -> TrainingPeerResponse {
        let now = self.now_ms();
        let merged = matches!(&work, TrainingPeerRequest::Submit(_));
        if !merged && let Some((taken, until)) = self.unanswered.get(&from).copied() {
            if taken >= MAX_UNANSWERED && now < until {
                return TrainingPeerResponse::Unavailable;
            }
            if taken >= MAX_UNANSWERED {
                self.unanswered.remove(&from);
            }
        }
        let Some(mut session) = self.session.take() else {
            return TrainingPeerResponse::Unavailable;
        };
        let operation = tokio::task::spawn_blocking(move || {
            let result = match work {
                TrainingPeerRequest::Work => session
                    .assign(from, now)
                    .map(|task| TrainingPeerResponse::Assigned(Box::new(task))),
                TrainingPeerRequest::Submit(result) => session
                    .merge(*result, from, now)
                    .map(|()| TrainingPeerResponse::Accepted(session.progress())),
                TrainingPeerRequest::Status
                | TrainingPeerRequest::Offers { .. }
                | TrainingPeerRequest::FetchModel { .. }
                | TrainingPeerRequest::Received { .. }
                | TrainingPeerRequest::Announce { .. } => Err(TrainingError::InvalidWork),
            };
            (session, result)
        })
        .await;
        let response = match operation {
            Ok((session, Ok(response))) => {
                self.session = Some(session);
                response
            }
            Ok((session, Err(error))) => {
                self.session = Some(session);
                if error != TrainingError::InvalidWork {
                    tracing::warn!(%error, "a group training request could not be served");
                }
                return TrainingPeerResponse::Rejected;
            }
            Err(_) => {
                // The copy in memory is gone. Carry on from the saved one.
                self.recover().await;
                self.publish().await;
                return TrainingPeerResponse::Rejected;
            }
        };
        // The lease counter is on disk before a packet leaves, and a result is
        // on disk before it is acknowledged.
        if let Err(error) = self.save_now().await {
            self.fail(error);
            self.publish().await;
            return TrainingPeerResponse::Rejected;
        }
        if merged {
            self.state.helped_rounds = self.state.helped_rounds.saturating_add(1);
            let rounds = self.peer_rounds.entry(from).or_default();
            *rounds = rounds.saturating_add(1);
            self.leases.remove(&from);
            self.unanswered.remove(&from);
            if let Some(point) = self.session.as_ref().and_then(|session| session.metrics()) {
                self.state.metrics.push(point);
            }
            self.observe(0);
            if self.state.progress.is_complete() {
                self.finish(None).await;
            } else if self.stalled() {
                self.finish(Some(
                    "Training finished early because the check stopped improving.".into(),
                ))
                .await;
            }
        } else {
            self.leases.insert(from, now);
            let unanswered = self.unanswered.entry(from).or_default();
            unanswered.0 = unanswered.0.saturating_add(1);
            unanswered.1 = now.saturating_add(UNANSWERED_COOL_DOWN_MS);
        }
        self.apply_peer_status();
        self.publish().await;
        // Membership can change while a disk write is in progress. A result
        // that was merged is acknowledged to a computer still in the group;
        // text and weights leave only while the run is still shared.
        let allowed = if merged {
            self.groups.permits(group, from).await
        } else {
            self.serves(group, from).await
        };
        if allowed {
            response
        } else {
            TrainingPeerResponse::Unavailable
        }
    }

    /// The copy of the run in memory was lost to a crashed step that was not
    /// this run's own. Reopen the saved one and keep going.
    async fn recover(&mut self) {
        let was_running = self.state.phase == TrainingPhase::Running;
        self.leases.clear();
        match self.load().await {
            Ok(()) => {
                if was_running && self.state.phase == TrainingPhase::Paused {
                    self.state.phase = TrainingPhase::Running;
                }
                self.state.notice = Some(
                    "A helper's update could not be used. Training carried on from the last saved step."
                        .into(),
                );
            }
            Err(error) => self.fail(error),
        }
    }

    /// Stops the group helping with this run. The run carries on here.
    pub(super) fn drop_sharing(&mut self) {
        self.state.sharing = None;
    }

    /// Takes back every packet that is out, so those stretches of text can be
    /// given out again. Needs the run here; otherwise the next sweep does it.
    pub(super) fn release_helpers(&mut self) {
        if let Some(session) = &mut self.session {
            for device in std::mem::take(&mut self.leases).into_keys() {
                session.abandon(device);
            }
        }
        self.apply_peer_status();
    }

    /// Takes back packets from helpers that left the group, took too long, or
    /// hold one for a run that is no longer shared.
    pub(super) async fn sweep_helpers(&mut self) {
        if self.leases.is_empty() || self.session.is_none() {
            return;
        }
        let lease_ms = self.state.plan.as_ref().map_or(60_000, |plan| {
            u64::from(plan.settings.lease_seconds).saturating_mul(1000)
        });
        let now = self.now_ms();
        let mut gone = Vec::new();
        for (device, taken) in &self.leases {
            let expired = now.saturating_sub(*taken) > lease_ms;
            let allowed = match self.state.sharing {
                Some(group) => self.groups.permits(group, *device).await,
                None => false,
            };
            if expired || !allowed {
                gone.push(*device);
            }
        }
        if gone.is_empty() {
            return;
        }
        if let Some(session) = &mut self.session {
            for device in gone {
                session.abandon(device);
                self.leases.remove(&device);
            }
        }
        self.apply_peer_status();
        self.unpublished = true;
    }

    /// Leaving or changing group ends sharing at once. Nothing but a new start
    /// with the group turns it back on.
    pub(super) async fn sync_sharing(&mut self) {
        let Some(shared) = self.state.sharing else {
            return;
        };
        match self.groups.snapshot().await {
            Some(group) if group.id == shared => {
                // Keep who is online current while the group is helping.
                let before = self.state.group.take();
                self.state.group = Some(group);
                self.apply_peer_status();
                if self.state.group != before {
                    self.unpublished = true;
                }
            }
            Some(_) | None => {
                self.drop_sharing();
                self.publish().await;
            }
        }
    }

    pub(super) async fn refresh_group(&mut self) {
        self.state.group = self.groups.snapshot().await;
        self.apply_peer_status();
    }

    /// Writes what this run knows about each helper into the group list.
    pub(super) fn apply_peer_status(&mut self) {
        if let Some(group) = &mut self.state.group {
            for peer in &mut group.peers {
                peer.helping = self.leases.contains_key(&peer.device);
                peer.rounds = self.peer_rounds.get(&peer.device).copied().unwrap_or(0);
            }
        }
    }

    pub(super) async fn start_help(&mut self, device: DeviceId) -> Result<(), String> {
        if self.state.busy() || self.slice.is_some() {
            self.state.notice = Some(
                if self.state.phase.busy() {
                    "This computer is training its own model. Pause or stop that before helping."
                } else {
                    "This computer is already helping. Stop helping before choosing another computer."
                }
                .into(),
            );
            return Ok(());
        }
        let group = self
            .groups
            .snapshot()
            .await
            .ok_or("Create or join a group first.")?;
        let peer = group
            .peers
            .iter()
            .find(|peer| peer.device == device)
            .cloned()
            .ok_or("Choose a computer in your group.")?;
        // The person chose this computer themselves: no permission is needed.
        self.begin_help(group, peer, None, false);
        Ok(())
    }

    /// Starts helping `peer`'s run. `automatic` says Sovra chose it under the
    /// person's "Help the group train" permission.
    pub(super) fn begin_help(
        &mut self,
        group: TrainingGroup,
        peer: TrainingPeer,
        run: Option<RunSummary>,
        automatic: bool,
    ) {
        self.drop_sharing();
        self.state.helping = Some(TrainingHelp {
            group: group.id,
            peer,
            phase: HelpingPhase::Waiting,
            accepted_rounds: 0,
            progress: None,
            run,
            notice: None,
        });
        self.state.group = Some(group);
        self.apply_peer_status();
        self.help_cancel = Arc::new(AtomicBool::new(false));
        self.help_after = 0;
        self.auto.automatic = automatic;
        self.auto.misses = 0;
    }

    pub(super) fn stop_help(&mut self) {
        if let Some(help) = &mut self.state.helping
            && help.phase.active()
        {
            self.help_cancel.store(true, Ordering::SeqCst);
            help.phase = if self.help_task.is_some() {
                HelpingPhase::Stopping
            } else {
                HelpingPhase::Stopped
            };
        }
    }

    pub(super) async fn tick_help(&mut self) -> bool {
        if let Some(help) = &self.state.helping
            && help.phase.active()
            && help.phase != HelpingPhase::Stopping
            && !self.groups.permits(help.group, help.peer.device).await
        {
            self.stop_help();
            if let Some(help) = &mut self.state.helping {
                help.notice = Some("Helping stopped because your group membership changed.".into());
            }
            return true;
        }
        if self.help_task.is_some() || self.now_ms() < self.help_after {
            return false;
        }
        let Some(help) = &mut self.state.helping else {
            return false;
        };
        if !matches!(help.phase, HelpingPhase::Working | HelpingPhase::Waiting) {
            return false;
        }
        let Some(group) = &self.state.group else {
            return false;
        };
        let Some(config) = &self.config else {
            return false;
        };
        help.phase = HelpingPhase::Working;
        let (group_id, peer, worker) = (help.group, help.peer.device, group.this_device);
        let engine = config.engine.clone();
        let groups = self.groups.clone();
        let cancelled = self.help_cancel.clone();
        self.help_task = Some(tokio::runtime::Handle::current().spawn(async move {
            help_once(engine, groups, group_id, peer, worker, cancelled).await
        }));
        true
    }

    pub(super) fn finish_help(&mut self, result: Result<HelpOutcome, tokio::task::JoinError>) {
        self.help_task = None;
        self.help_after = self.now_ms().saturating_add(2000);
        let Some(help) = &mut self.state.helping else {
            return;
        };
        let stopped = self.help_cancel.load(Ordering::SeqCst);
        // Why helping was stopped is already on the page; the packet that was
        // still out does not get to replace it.
        let said = help.notice.clone();
        let mut missed = false;
        match result {
            Ok(HelpOutcome::Accepted(progress)) => {
                help.accepted_rounds = help.accepted_rounds.saturating_add(1);
                help.phase = if progress.is_complete() {
                    HelpingPhase::Finished
                } else {
                    HelpingPhase::Waiting
                };
                help.progress = Some(progress);
                help.notice = None;
                self.help_after = 0;
                self.auto.misses = 0;
            }
            Ok(HelpOutcome::Away(notice)) => {
                help.phase = HelpingPhase::Waiting;
                help.notice = Some(notice);
                missed = true;
            }
            Ok(HelpOutcome::Waiting(notice)) => {
                help.phase = HelpingPhase::Waiting;
                help.notice = Some(notice);
            }
            Ok(HelpOutcome::Stopped(notice)) => {
                help.phase = HelpingPhase::Stopped;
                help.notice = Some(notice);
            }
            Ok(HelpOutcome::Cancelled) => help.phase = HelpingPhase::Stopped,
            Err(_) => {
                help.phase = HelpingPhase::Stopped;
                help.notice = Some("The helper stopped. Choose Help again to retry.".into());
            }
        }
        if stopped {
            help.phase = HelpingPhase::Stopped;
            help.notice = said;
        } else if missed {
            self.missed_help();
        }
    }
}

async fn help_once(
    engine: Arc<dyn TrainingEngine>,
    groups: crate::groups::GroupTraining,
    group: GroupId,
    peer: DeviceId,
    worker: DeviceId,
    cancelled: Arc<AtomicBool>,
) -> HelpOutcome {
    let fetched = groups
        .ask(group, peer, TrainingPeerRequest::Work, BULKY)
        .await;
    if cancelled.load(Ordering::SeqCst) {
        return HelpOutcome::Cancelled;
    }
    if !groups.permits(group, peer).await {
        return HelpOutcome::Stopped(
            "Helping stopped because your group membership changed.".into(),
        );
    }
    let task = match fetched {
        PeerReply::Answer(TrainingPeerResponse::Assigned(task)) => *task,
        PeerReply::Answer(TrainingPeerResponse::Rejected) => {
            return HelpOutcome::Waiting(
                "That computer did not accept the request. Sovra will try again.".into(),
            );
        }
        PeerReply::Answer(_) | PeerReply::TooOld => {
            return HelpOutcome::Away(
                "Waiting for that computer to share a running training job.".into(),
            );
        }
        PeerReply::Refused | PeerReply::Garbled | PeerReply::Unreachable => return HelpOutcome::Away(
            "That computer is unavailable. Sovra will try again; you can stop helping at any time."
                .into(),
        ),
    };
    if task.parameters(worker).is_err() {
        return HelpOutcome::Waiting(
            "That computer sent an unsupported training assignment.".into(),
        );
    }
    // A chat may have started while the assignment was crossing the network.
    if crate::model_priority::busy() {
        return HelpOutcome::Waiting("Helping will continue when your chat finishes.".into());
    }
    // Always await the bounded CPU task, even after Stop. No unowned worker remains.
    let trained = tokio::task::spawn_blocking(move || engine.work(task, worker)).await;
    if cancelled.load(Ordering::SeqCst) {
        return HelpOutcome::Cancelled;
    }
    let Ok(Ok(result)) = trained else {
        return HelpOutcome::Waiting(
            "This computer could not finish the short training step.".into(),
        );
    };
    match groups
        .ask(
            group,
            peer,
            TrainingPeerRequest::Submit(Box::new(result)),
            BULKY,
        )
        .await
    {
        PeerReply::Answer(TrainingPeerResponse::Accepted(progress)) => {
            HelpOutcome::Accepted(progress)
        }
        // A result that arrived too late or was refused is simply not used.
        // The next packet starts from the model as it is now.
        PeerReply::Answer(_)
        | PeerReply::TooOld
        | PeerReply::Refused
        | PeerReply::Garbled
        | PeerReply::Unreachable => HelpOutcome::Waiting(
            "The update was not acknowledged. Sovra will ask for fresh work.".into(),
        ),
    }
}
