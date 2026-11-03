//! Bounded assembly of visible team replies; runtime supplies time and permission.

use std::collections::BTreeMap;

use crate::AgUiEvent;

use super::{
    EpisodeKind, MAX_EPISODE_BYTES, MemoryError, MemorySourceAdmission, MemorySourceAuthor,
    MemorySourceProvenance, MemorySourceScope, NewEpisode, Timestamp,
};

#[derive(Debug, Default)]
pub struct TeamMemoryCapture {
    run_id: Option<String>,
    speakers: BTreeMap<String, Author>,
    messages: BTreeMap<String, Reply>,
}

#[derive(Debug)]
struct Reply {
    speaker: Option<Author>,
    content: String,
    started_at: Timestamp,
}

#[derive(Debug, Clone)]
struct Author {
    member_id: String,
    name: String,
}

impl TeamMemoryCapture {
    /// Only the visible text protocol is evidence: activity, hidden reasoning and
    /// moderation controls are not user statements and do not enter the buffer.
    pub fn apply(
        &mut self,
        session: &str,
        event: &AgUiEvent,
        at: Timestamp,
    ) -> Result<Option<MemorySourceAdmission>, MemoryError> {
        let invalid = || {
            MemoryError::InvalidInput("Team reply capture has invalid or oversized data.".into())
        };
        let bounded = |id: &str| !id.trim().is_empty() && id.len() <= 256;
        match event {
            AgUiEvent::RunStarted { run_id } => {
                *self = Self::default();
                if !bounded(run_id) {
                    return Err(invalid());
                }
                self.run_id = Some(run_id.clone());
            }
            AgUiEvent::SubagentStarted {
                subagent_run_id,
                speaker,
                ..
            } => {
                if !bounded(subagent_run_id)
                    || !bounded(&speaker.member_id)
                    || !bounded(&speaker.name)
                    || self.speakers.len() >= crate::teams::MAX_REPLIES_PER_TURN
                {
                    return Err(invalid());
                }
                self.speakers.insert(
                    subagent_run_id.clone(),
                    Author {
                        member_id: speaker.member_id.clone(),
                        name: speaker.name.clone(),
                    },
                );
            }
            AgUiEvent::TextMessageStart {
                message_id,
                subagent_run_id,
            } => {
                let speaker = subagent_run_id
                    .as_ref()
                    .and_then(|id| self.speakers.get(id));
                if self.run_id.is_none()
                    || !bounded(message_id)
                    || self.messages.contains_key(message_id)
                    || self.messages.len() >= crate::teams::MAX_REPLIES_PER_TURN
                {
                    return Err(invalid());
                }
                if subagent_run_id.is_some() && speaker.is_none() {
                    return Err(invalid());
                }
                self.messages.insert(
                    message_id.clone(),
                    Reply {
                        speaker: speaker.cloned(),
                        content: String::new(),
                        started_at: at,
                    },
                );
            }
            AgUiEvent::TextMessageContent { message_id, delta } => {
                if let Some(reply) = self.messages.get_mut(message_id) {
                    if reply.content.len().saturating_add(delta.len()) > MAX_EPISODE_BYTES {
                        self.messages.remove(message_id);
                        return Err(invalid());
                    }
                    reply.content.push_str(delta);
                }
            }
            AgUiEvent::TextMessageEnd { message_id } => {
                let Some(reply) = self.messages.remove(message_id) else {
                    return Ok(None);
                };
                if reply.content.trim().is_empty() {
                    return Ok(None);
                }
                let Some(run_id) = &self.run_id else {
                    return Err(invalid());
                };
                let (speaker, provenance) = match reply.speaker {
                    Some(speaker) => (
                        Some(speaker.name),
                        MemorySourceProvenance::Team {
                            run_id: run_id.clone(),
                            member_id: speaker.member_id,
                        },
                    ),
                    None => (None, MemorySourceProvenance::Conversation),
                };
                let source = MemorySourceAdmission {
                    dependencies: None,
                    expected_generation: None,
                    key: message_id.clone(),
                    version: 1,
                    author: MemorySourceAuthor::Assistant { speaker },
                    scope: MemorySourceScope::Private,
                    provenance,
                    episode: NewEpisode {
                        session_id: session.into(),
                        kind: EpisodeKind::AgentMessage,
                        content: reply.content,
                        occurred_at: reply.started_at,
                    },
                };
                source.validate()?;
                return Ok(Some(source));
            }
            AgUiEvent::SubagentFinished {
                subagent_run_id, ..
            } => {
                self.speakers.remove(subagent_run_id);
            }
            AgUiEvent::RunFinished { .. } | AgUiEvent::RunError { .. } => {
                *self = Self::default();
            }
            AgUiEvent::ActivitySnapshot { .. } => {}
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Speaker;

    #[test]
    fn an_unnamed_visible_summary_does_not_invent_a_team_member() {
        let mut capture = TeamMemoryCapture::default();
        for event in [
            AgUiEvent::RunStarted {
                run_id: "research-run".into(),
            },
            AgUiEvent::TextMessageStart {
                message_id: "summary".into(),
                subagent_run_id: None,
            },
            AgUiEvent::TextMessageContent {
                message_id: "summary".into(),
                delta: "Visible summary.".into(),
            },
        ] {
            assert!(
                capture
                    .apply("chat", &event, Timestamp(1))
                    .unwrap()
                    .is_none()
            );
        }
        let source = capture
            .apply(
                "chat",
                &AgUiEvent::TextMessageEnd {
                    message_id: "summary".into(),
                },
                Timestamp(2),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            source.author,
            MemorySourceAuthor::Assistant { speaker: None }
        );
        assert_eq!(source.provenance, MemorySourceProvenance::Conversation);
        assert!(!source.needs_extraction());
    }

    fn start(capture: &mut TeamMemoryCapture, member: &str) {
        capture
            .apply(
                "chat",
                &AgUiEvent::SubagentStarted {
                    subagent_run_id: member.into(),
                    name: member.into(),
                    description: "helper".into(),
                    speaker: Speaker {
                        member_id: member.into(),
                        name: format!("Helper {member}"),
                        icon: String::new(),
                        role: "helper".into(),
                    },
                },
                Timestamp(1),
            )
            .unwrap();
        capture
            .apply(
                "chat",
                &AgUiEvent::TextMessageStart {
                    message_id: member.into(),
                    subagent_run_id: Some(member.into()),
                },
                Timestamp(2),
            )
            .unwrap();
    }

    #[test]
    fn interleaved_team_text_keeps_exact_authorship_identity_and_source_time() {
        let mut capture = TeamMemoryCapture::default();
        capture
            .apply(
                "chat",
                &AgUiEvent::RunStarted {
                    run_id: "run-1".into(),
                },
                Timestamp(1),
            )
            .unwrap();
        start(&mut capture, "a");
        start(&mut capture, "b");
        for (member, text) in [
            ("a", "I suggest 茶"),
            ("b", "A different plan."),
            ("a", "."),
        ] {
            capture
                .apply(
                    "chat",
                    &AgUiEvent::TextMessageContent {
                        message_id: member.into(),
                        delta: text.into(),
                    },
                    Timestamp(3),
                )
                .unwrap();
        }
        for (member, text) in [("b", "A different plan."), ("a", "I suggest 茶.")] {
            let source = capture
                .apply(
                    "chat",
                    &AgUiEvent::TextMessageEnd {
                        message_id: member.into(),
                    },
                    Timestamp(4),
                )
                .unwrap()
                .unwrap();
            assert_eq!(source.episode.content, text);
            assert_eq!(source.episode.occurred_at, Timestamp(2));
            assert_eq!(source.key, member);
            assert_eq!(
                source.author,
                MemorySourceAuthor::Assistant {
                    speaker: Some(format!("Helper {member}"))
                }
            );
            assert_eq!(
                source.provenance,
                MemorySourceProvenance::Team {
                    run_id: "run-1".into(),
                    member_id: member.into()
                }
            );
            assert!(!source.needs_extraction());
            assert!(
                capture
                    .apply(
                        "chat",
                        &AgUiEvent::TextMessageEnd {
                            message_id: member.into()
                        },
                        Timestamp(5)
                    )
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn invalid_oversized_empty_and_interrupted_replies_do_not_become_truncated_evidence() {
        let mut capture = TeamMemoryCapture::default();
        assert!(
            capture
                .apply(
                    "chat",
                    &AgUiEvent::TextMessageStart {
                        message_id: "orphan".into(),
                        subagent_run_id: None
                    },
                    Timestamp(0)
                )
                .is_err()
        );
        capture
            .apply(
                "chat",
                &AgUiEvent::RunStarted {
                    run_id: "run".into(),
                },
                Timestamp(0),
            )
            .unwrap();
        start(&mut capture, "large");
        assert!(
            capture
                .apply(
                    "chat",
                    &AgUiEvent::TextMessageContent {
                        message_id: "large".into(),
                        delta: "x".repeat(MAX_EPISODE_BYTES + 1)
                    },
                    Timestamp(3)
                )
                .is_err()
        );
        assert!(
            capture
                .apply(
                    "chat",
                    &AgUiEvent::TextMessageEnd {
                        message_id: "large".into()
                    },
                    Timestamp(4)
                )
                .unwrap()
                .is_none()
        );
        start(&mut capture, "empty");
        assert!(
            capture
                .apply(
                    "chat",
                    &AgUiEvent::TextMessageEnd {
                        message_id: "empty".into()
                    },
                    Timestamp(4)
                )
                .unwrap()
                .is_none()
        );
        start(&mut capture, "interrupted");
        capture
            .apply(
                "chat",
                &AgUiEvent::TextMessageContent {
                    message_id: "interrupted".into(),
                    delta: "partial".into(),
                },
                Timestamp(3),
            )
            .unwrap();
        capture
            .apply(
                "chat",
                &AgUiEvent::RunError {
                    message: "cancelled".into(),
                },
                Timestamp(4),
            )
            .unwrap();
        assert!(
            capture
                .apply(
                    "chat",
                    &AgUiEvent::TextMessageEnd {
                        message_id: "interrupted".into()
                    },
                    Timestamp(5)
                )
                .unwrap()
                .is_none()
        );
    }
}
