//! Bounded group inbox and one owned, cancellable learner interval at a time.

use std::sync::atomic::{AtomicBool, Ordering};

use sovra_core::{
    group::{DeviceId, GroupId},
    training::{
        HelpingPhase, TrainingError, TrainingHelp, TrainingPeerRequest, TrainingPeerResponse,
        TrainingProgress,
    },
};

use super::{Actor, Arc, Duration, Receiver, Sender, TrainingEngine, TrainingPhase, encode};

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
        let (sender, receiver) = async_channel::bounded(8);
        (Self(sender), receiver)
    }

    pub(crate) async fn request(
        &self,
        group: GroupId,
        from: DeviceId,
        work: TrainingPeerRequest,
    ) -> TrainingPeerResponse {
        let (reply, receive) = tokio::sync::oneshot::channel();
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
        tokio::time::timeout(Duration::from_secs(15), receive)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(TrainingPeerResponse::Unavailable)
    }
}

pub(super) enum HelpOutcome {
    Accepted(TrainingProgress),
    Waiting(String),
    Stopped(String),
    Cancelled,
}

impl Actor {
    pub(super) async fn receive_peer(&mut self, incoming: PeerWork) {
        let response = self
            .peer_work(incoming.group, incoming.from, incoming.work)
            .await;
        let _ = incoming.reply.send(response);
    }

    async fn peer_work(
        &mut self,
        group: GroupId,
        from: DeviceId,
        work: TrainingPeerRequest,
    ) -> TrainingPeerResponse {
        if self.state.sharing != Some(group)
            || self.state.phase != TrainingPhase::Running
            || !self.groups.training_help().await
            || !self.groups.permits(group, from).await
        {
            return TrainingPeerResponse::Unavailable;
        }
        let Some(mut session) = self.session.take() else {
            return TrainingPeerResponse::Unavailable;
        };
        let now = self.now_ms();
        let merged = matches!(&work, TrainingPeerRequest::Submit(_));
        let operation = tokio::task::spawn_blocking(move || {
            let result = match work {
                TrainingPeerRequest::Work => session
                    .assign(from, now)
                    .map(|task| TrainingPeerResponse::Assigned(Box::new(task))),
                TrainingPeerRequest::Submit(result) => session
                    .merge(*result, from, now)
                    .map(|()| TrainingPeerResponse::Accepted(session.progress())),
            };
            (session, result)
        })
        .await;
        let (session, response) = match operation {
            Ok((session, Ok(response))) => (session, response),
            Ok((session, Err(TrainingError::InvalidWork))) => {
                self.session = Some(session);
                return TrainingPeerResponse::Rejected;
            }
            Ok((_, Err(error))) => {
                self.fail(error.to_string());
                self.publish().await;
                return TrainingPeerResponse::Rejected;
            }
            Err(_) => {
                self.fail("The training worker stopped. Resume from the last saved step.".into());
                self.publish().await;
                return TrainingPeerResponse::Rejected;
            }
        };
        let saved = match encode(session).await {
            Ok((session, bytes)) => self.save(session, bytes).await,
            Err(error) => Err(error),
        };
        if let Err(error) = saved {
            self.fail(error);
            self.publish().await;
            return TrainingPeerResponse::Rejected;
        }
        if merged {
            self.state.helped_rounds = self.state.helped_rounds.saturating_add(1);
        }
        self.publish().await;
        // Membership can change while a disk write is in progress. Never deliver
        // more selected text/model weights after revocation has been observed.
        if !self.groups.training_help().await || !self.groups.permits(group, from).await {
            return TrainingPeerResponse::Unavailable;
        }
        response
    }

    pub(super) async fn start_help(&mut self, device: DeviceId) -> Result<(), String> {
        if self.state.busy() {
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
        self.state.sharing = None;
        self.state.helping = Some(TrainingHelp {
            group: group.id,
            peer,
            phase: HelpingPhase::Waiting,
            accepted_rounds: 0,
            progress: None,
            notice: None,
        });
        self.state.group = Some(group);
        self.state.error = None;
        self.help_cancel = Arc::new(AtomicBool::new(false));
        self.help_after = 0;
        Ok(())
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
    let fetched = groups.request(group, peer, TrainingPeerRequest::Work).await;
    if cancelled.load(Ordering::SeqCst) {
        return HelpOutcome::Cancelled;
    }
    if !groups.permits(group, peer).await {
        return HelpOutcome::Stopped(
            "Helping stopped because your group membership changed.".into(),
        );
    }
    let task = match fetched {
        Ok(TrainingPeerResponse::Assigned(task)) => *task,
        Ok(_) => {
            return HelpOutcome::Waiting(
                "Waiting for that computer to share a running training job.".into(),
            );
        }
        Err(_) => return HelpOutcome::Waiting(
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
        .request(group, peer, TrainingPeerRequest::Submit(Box::new(result)))
        .await
    {
        Ok(TrainingPeerResponse::Accepted(progress)) => HelpOutcome::Accepted(progress),
        _ => HelpOutcome::Waiting(
            "The update was not acknowledged. Sovra will ask for fresh work.".into(),
        ),
    }
}
