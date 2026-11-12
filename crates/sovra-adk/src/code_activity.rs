//! Source-only AG-UI previews from visible model output. This never parses or
//! publishes private reasoning, and preview limits do not truncate saved work.

use async_channel::Sender;
use sovra_core::artifacts::{ArtifactDraft, ArtifactKind, ArtifactSource, find_pieces};
use sovra_core::{AgUiEvent, Event, LiveActivity};

#[derive(Default)]
pub(crate) struct CodeActivity {
    author: String,
    text: String,
    previous: String,
    completed: Vec<ArtifactDraft>,
    overflowed: bool,
    project_saved: bool,
}

impl CodeActivity {
    fn collect_completed(&mut self, session_id: &str) {
        for piece in find_pieces(&self.text) {
            if !matches!(piece.kind, ArtifactKind::Code | ArtifactKind::Webpage)
                || !self.text[piece.span.clone()].trim_end().ends_with("```")
                || self
                    .completed
                    .iter()
                    .any(|draft| draft.content == piece.content)
            {
                continue;
            }
            self.completed.push(ArtifactDraft {
                title: if piece.kind == ArtifactKind::Webpage {
                    "Website".into()
                } else {
                    format!("{} code", piece.language)
                },
                kind: piece.kind,
                language: piece.language,
                content: piece.content,
                source: ArtifactSource {
                    session_id: session_id.to_owned(),
                    author: "Sovra".into(),
                    ..Default::default()
                },
            });
        }
    }

    pub(crate) async fn save_completed(
        mut self,
        session_id: &str,
        saver: &crate::ArtifactSaver,
        events: &Sender<Event>,
    ) {
        if self.project_saved {
            return;
        }
        self.collect_completed(session_id);
        for draft in self.completed {
            match saver.save(draft).await {
                Ok(artifact) => {
                    let _ = events
                        .send(Event::AgUi {
                            session_id: session_id.to_owned(),
                            event: AgUiEvent::ActivitySnapshot {
                                message_id: format!("code-preview-{session_id}"),
                                activity: LiveActivity::CodeWriting {
                                    language: artifact.language.clone(),
                                    content: artifact.content.chars().take(32_768).collect(),
                                    artifact_id: Some(artifact.id.clone()),
                                },
                            },
                        })
                        .await;

                    let _ = events
                        .send(Event::AgUiSurface {
                            session_id: session_id.to_owned(),
                            widget: sovra_core::AgUiWidget::ArtifactSaved {
                                artifact_id: artifact.id,
                                title: artifact.title,
                                artifact_kind: artifact.kind,
                            },
                        })
                        .await;
                }
                Err(message) => {
                    let _ = events
                        .send(Event::Artifacts(
                            sovra_core::artifacts::ArtifactEvent::Error(message),
                        ))
                        .await;
                }
            }
        }
    }

    pub(crate) async fn push(
        &mut self,
        session_id: &str,
        event: &adk_core::Event,
        events: &Sender<Event>,
    ) {
        if crate::artifacts::saved_artifact_widgets(event)
            .iter()
            .any(|widget| {
                matches!(
                    widget,
                    sovra_core::AgUiWidget::ArtifactSaved {
                        artifact_kind: ArtifactKind::Project,
                        ..
                    }
                )
            })
        {
            self.project_saved = true;
        }
        let (_, Some(text)) = crate::extract_output(event) else {
            return;
        };
        if self.author != event.author {
            self.collect_completed(session_id);
            self.author.clone_from(&event.author);
            self.text.clear();
            self.overflowed = false;
        }
        // Bound the accumulated visible answer as well as the displayed source.
        if self.overflowed {
            return;
        }
        if self.text.len().saturating_add(text.len()) > 512 * 1024 {
            // Never resume with later chunks: doing so could close a fence around
            // incomplete source and incorrectly announce it as a complete save.
            self.overflowed = true;
            return;
        }
        self.text.push_str(&text);
        let Some(piece) = find_pieces(&self.text)
            .into_iter()
            .rev()
            .find(|piece| matches!(piece.kind, ArtifactKind::Code | ArtifactKind::Webpage))
        else {
            return;
        };
        let preview: String = piece.content.chars().take(32_768).collect();
        if preview == self.previous {
            return;
        }
        self.previous.clone_from(&preview);
        let _ = events
            .send(Event::AgUi {
                session_id: session_id.to_owned(),
                event: AgUiEvent::ActivitySnapshot {
                    message_id: format!("code-preview-{session_id}"),
                    activity: LiveActivity::CodeWriting {
                        language: piece.language,
                        content: preview,
                        artifact_id: None,
                    },
                },
            })
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::Content;

    #[test]
    fn only_complete_source_blocks_are_kept_without_preview_truncation() {
        let source = "a".repeat(40_000);
        let mut activity = CodeActivity {
            text: format!("```html\n{source}\n```\n```rust\nunfinished"),
            ..Default::default()
        };
        activity.collect_completed("chat");
        activity.collect_completed("chat");
        assert_eq!(activity.completed.len(), 1);
        assert_eq!(activity.completed[0].content, source);
        assert_eq!(activity.completed[0].source.session_id, "chat");
    }

    #[test]
    fn completed_code_is_saved_and_storage_failure_is_visible() {
        futures::executor::block_on(async {
            for fail in [false, true] {
                let activity = CodeActivity {
                    text: "```html\n<h1>Garden</h1>\n```".into(),
                    ..Default::default()
                };
                let saver = crate::ArtifactSaver::from_fn(move |draft| async move {
                    assert_eq!(draft.content, "<h1>Garden</h1>");
                    if fail {
                        Err("Could not save".into())
                    } else {
                        Ok(sovra_core::artifacts::Artifact::from_draft(
                            "website".into(),
                            draft,
                            sovra_core::Timestamp(1),
                        ))
                    }
                });
                let (tx, rx) = async_channel::unbounded();
                activity.save_completed("chat", &saver, &tx).await;
                let mut event = rx.recv().await.unwrap();
                if !fail {
                    assert!(matches!(
                        event,
                        Event::AgUi {
                            event: AgUiEvent::ActivitySnapshot {
                                activity: LiveActivity::CodeWriting {
                                    artifact_id: Some(_),
                                    ..
                                },
                                ..
                            },
                            ..
                        }
                    ));
                    event = rx.recv().await.unwrap();
                }
                if fail {
                    assert!(matches!(
                        event,
                        Event::Artifacts(sovra_core::artifacts::ArtifactEvent::Error(_))
                    ));
                } else {
                    assert!(matches!(
                        event,
                        Event::AgUiSurface {
                            widget: sovra_core::AgUiWidget::ArtifactSaved { .. },
                            ..
                        }
                    ));
                }
            }
        });
    }

    #[test]
    fn an_oversized_chunk_cannot_be_dropped_then_saved_as_complete_code() {
        futures::executor::block_on(async {
            let mut activity = CodeActivity::default();
            let (tx, _rx) = async_channel::unbounded();
            let mut event = adk_core::Event::new("test");
            for text in [
                "```html\n<h1>".to_owned(),
                "a".repeat(512 * 1024),
                "</h1>\n```".to_owned(),
            ] {
                event.set_content(Content::new("model").with_text(&text));
                activity.push("chat", &event, &tx).await;
            }
            activity.collect_completed("chat");
            assert!(
                activity.completed.is_empty(),
                "never save source with a missing chunk"
            );
        });
    }

    #[test]
    fn source_snapshots_follow_partial_fences_and_ignore_tool_text() {
        futures::executor::block_on(async {
            let mut activity = CodeActivity::default();
            let (tx, rx) = async_channel::unbounded();
            let mut event = adk_core::Event::new("test");
            event.set_content(Content::new("tool").with_text("```html\nprivate tool result"));
            activity.push("chat", &event, &tx).await;
            assert!(rx.is_empty());
            event.set_content(Content::new("model").with_text("Here is the page:\n```ht"));
            activity.push("chat", &event, &tx).await;
            assert!(rx.is_empty());
            event.set_content(Content::new("model").with_text("ml\n<h1>Garden</h1>"));
            activity.push("chat", &event, &tx).await;
            let Event::AgUi {
                session_id,
                event:
                    AgUiEvent::ActivitySnapshot {
                        activity:
                            LiveActivity::CodeWriting {
                                language, content, ..
                            },
                        ..
                    },
            } = rx.recv().await.unwrap()
            else {
                panic!("source activity");
            };
            assert_eq!(session_id, "chat");
            assert_eq!(language, "html");
            assert_eq!(content, "<h1>Garden</h1>");
            event.set_content(Content::new("model").with_text("\n```"));
            activity.push("chat", &event, &tx).await;
            assert!(
                rx.is_empty(),
                "unchanged source does not emit another snapshot"
            );
        });
    }
}
