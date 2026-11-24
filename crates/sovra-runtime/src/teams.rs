//! The user's team: the roster (seeded on first use, saved in the memory store)
//! and team turns, which run on the selected model and stream AG-UI events.

use std::sync::Arc;

use async_channel::Sender;
use sovra_adk::{AdkHarness, TeamTurn};
use sovra_core::teams::{self, TeamCommand, TeamEvent, TeamMember, TeamRoom, TranscriptLine};
use sovra_core::{AgUiEvent, Event, MemoryStore};
use tokio::sync::Mutex;

use crate::ActiveRun;

/// One team turn the command loop is ready to start.
pub(crate) struct TeamRunRequest {
    pub session_id: String,
    pub prompt: String,
    pub history: Vec<TranscriptLine>,
    pub room: TeamRoom,
    pub timeout: Option<std::time::Duration>,
}

#[derive(Clone)]
pub(crate) struct TeamRuntime {
    store: Option<Arc<dyn MemoryStore>>,
    /// `None` until loaded; then the source of truth for team turns.
    members: Arc<Mutex<Option<Vec<TeamMember>>>>,
    events: Sender<Event>,
}

impl TeamRuntime {
    pub(crate) fn new(store: Option<Arc<dyn MemoryStore>>, events: Sender<Event>) -> Self {
        Self {
            store,
            members: Arc::new(Mutex::new(None)),
            events,
        }
    }

    pub(crate) fn with_events(&self, events: Sender<Event>) -> Self {
        Self {
            events,
            ..self.clone()
        }
    }

    /// Roster changes, off the command loop. `Run` is started by [`Self::spawn_run`].
    pub(crate) fn handle(&self, handle: &tokio::runtime::Handle, command: TeamCommand) {
        let runtime = self.clone();
        drop(handle.spawn(async move {
            let event = runtime.apply(command).await;
            let _ = runtime.events.send(Event::Teams(event)).await;
        }));
    }

    async fn apply(&self, command: TeamCommand) -> TeamEvent {
        let mut guard = self.load().await;
        let members = guard.get_or_insert_with(teams::default_team);
        let event = match command {
            TeamCommand::Load => {
                return TeamEvent::Loaded {
                    members: members.clone(),
                };
            }
            TeamCommand::Run { .. } => {
                return TeamEvent::Error("Team turns are started separately.".into());
            }
            TeamCommand::SaveMember(member) => {
                if member.name.trim().is_empty() {
                    return TeamEvent::Error("Give your helper a name.".into());
                }
                teams::upsert(members, member.clone());
                TeamEvent::MemberSaved(member)
            }
            TeamCommand::RemoveMember { id } => match members.iter().position(|m| m.id == id) {
                Some(index) if members[index].builtin => {
                    members[index].active = false;
                    TeamEvent::MemberSaved(members[index].clone())
                }
                Some(index) => {
                    members.remove(index);
                    TeamEvent::MemberRemoved { id }
                }
                None => return TeamEvent::Error("That helper is no longer on your team.".into()),
            },
            TeamCommand::ResetMember { id } => match teams::builtin_member(&id) {
                Some(original) => {
                    teams::upsert(members, original.clone());
                    TeamEvent::MemberSaved(original)
                }
                None => return TeamEvent::Error("Only Sovra's own helpers can be reset.".into()),
            },
        };
        if let Some(store) = &self.store
            && let Err(error) = store.save_team(members).await
        {
            tracing::warn!(%error, "failed to save team");
        }
        event
    }

    async fn load(&self) -> tokio::sync::MutexGuard<'_, Option<Vec<TeamMember>>> {
        let mut members = self.members.lock().await;
        if members.is_none()
            && let Some(store) = &self.store
        {
            match store.load_team().await {
                Ok(Some(saved)) => {
                    let before = saved.len();
                    let merged = teams::merge_shipped(saved);
                    let added = merged.len() != before;
                    *members = Some(merged);
                    if added
                        && let Some(current) = members.as_ref()
                        && let Err(error) = store.save_team(current).await
                    {
                        tracing::warn!(%error, "failed to save team");
                    }
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "failed to load team"),
            }
        }
        members
    }

    /// Starts a team turn on `harness`. `timeout` bounds the whole turn.
    pub(crate) fn spawn_run(
        &self,
        handle: &tokio::runtime::Handle,
        harness: AdkHarness,
        request: TeamRunRequest,
    ) -> ActiveRun {
        let TeamRunRequest {
            session_id,
            prompt,
            history,
            room,
            timeout,
        } = request;
        let runtime = self.clone();
        let slot = Arc::new(std::sync::RwLock::new(Some(harness.clone())));
        let interactive = crate::model_priority::interactive();
        let task = handle.spawn(async move {
            let _interactive = interactive;
            let members = runtime
                .load()
                .await
                .get_or_insert_with(teams::default_team)
                .clone();
            let turn = TeamTurn {
                run_id: format!("team-{}", crate::remembering::source_key()),
                session_id: session_id.clone(),
                prompt,
                history,
                members,
                room,
            };
            match timeout {
                // A long team turn is fine while members keep replying; stop only
                // when nobody has said anything for the whole timeout.
                Some(quiet) => {
                    let (relay, relayed) = crate::quiet_deadline::relay();
                    let run = harness.run_team(&turn, &relay);
                    // Team members have no terminal, so nobody is ever waiting on an approval card.
                    if crate::quiet_deadline::forward(run, &relayed, &runtime.events, quiet, || {
                        false
                    })
                    .await
                    .is_err()
                    {
                        let _ = runtime
                            .events
                            .send(Event::AgUi {
                                session_id,
                                event: AgUiEvent::RunError {
                                    message: "Your team stopped responding. Try again.".into(),
                                },
                            })
                            .await;
                    }
                }
                None => harness.run_team(&turn, &runtime.events).await,
            }
        });
        ActiveRun {
            harness: slot,
            task,
        }
    }
}
