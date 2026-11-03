//! Saved file relationships remain readable and navigable at every supported size.
use super::knowledge_profile::{reveal, settle};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::{
    Command, Event, NavTab,
    file_index::{graph::*, *},
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

fn page() -> GraphPage {
    GraphPage {
        entities: vec![GraphEntity {
            id: "entity:orchid".into(),
            kind: EntityKind::Project,
            label: "Orchid garden project with an unusually long descriptive name".into(),
            evidence: vec![1],
            created_at: 1,
        }],
        relations: vec![GraphRelation {
            id: "relation:orchid".into(),
            subject: GraphEndpoint::Entity("entity:orchid".into()),
            object: GraphEndpoint::Source(1),
            relation: RelationKind::GroundedIn,
            statement: "The Orchid garden project is described in the shared project brief.".into(),
            evidence: vec![1],
            basis: EvidenceBasis {
                tier: EvidenceTier::ModelInference,
                confidence: None,
                activity_id: None,
                recorded_at: 1,
                observed_at: None,
                valid_time: None,
            },
            proposal_id: 1,
            origin: None,
            replaces: None,
            expired_at: None,
            sources_current: true,
        }],
        sources: vec![GraphSource {
            id: 1,
            reference: PassageRef {
                root_id: "root".into(),
                file_id: "file".into(),
                version_id: "version".into(),
                ordinal: 0,
            },
            path: "Garden planning and maintenance/project brief with a long readable name.txt"
                .into(),
            start_line: 1,
            end_line: 30,
            current: true,
        }],
        truncated: true,
        next: Some(GraphCursor {
            recorded_at: 1,
            id: "relation:orchid".into(),
        }),
    }
}
fn request(commands: &async_channel::Receiver<Command>) -> IndexCommand {
    std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| match command {
            Command::FileIndex(command) => Some(command),
            _ => None,
        })
        .expect("file knowledge request")
}
fn response(command: &IndexCommand, page: GraphPage) -> Event {
    let IndexCommand::BrowseKnowledge { request_id, .. } = command else {
        panic!("browse")
    };
    Event::FileIndex(IndexEvent::KnowledgeBrowsed {
        request_id: *request_id,
        result: Ok(page),
    })
}

#[gpui_kit::test]
fn saved_file_relationships_search_page_follow_subjects_and_read_evidence_at_every_size(
    cx: &mut TestAppContext,
) {
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(
                    sovra_core::models::InterfaceScale::from_percent(scale),
                    cx,
                );
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| {
                    let mut shell = Shell::new(0, window, cx);
                    shell.select_tab(NavTab::Knowledge, cx);
                    shell
                });
                Root::new(shell, window, cx)
            });
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.click("knowledge-files", cx);
                window.render_frame(cx);
                reveal(window, "knowledge-browse-saved", cx);
                window.click("knowledge-browse-saved", cx);
            })
            .unwrap();
            let browse = request(&commands);
            assert!(
                matches!(&browse,IndexCommand::BrowseKnowledge { query, .. } if *query == GraphQuery::default())
            );
            events.send_blocking(response(&browse, page())).unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "knowledge-saved-next", cx);
                window.click("knowledge-saved-next", cx);
            })
            .unwrap();
            let next = request(&commands);
            assert!(
                matches!(&next,IndexCommand::BrowseKnowledge { query, .. } if query.before == page().next)
            );
            events.send_blocking(response(&next, page())).unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("knowledge-saved-open", 0usize), cx);
                window.click(("knowledge-saved-open", 0usize), cx);
                window.render_frame(cx);
                reveal(window, "knowledge-relationship-status", cx);
                assert_eq!(
                    window.find("knowledge-relationship-status").label(),
                    Some("A helper’s interpretation · not verified")
                );
                reveal(window, ("knowledge-read-evidence", 0usize), cx);
                window.click(("knowledge-read-evidence", 0usize), cx);
            })
            .unwrap();
            let IndexCommand::KnowledgeEvidence { request_id, id } = request(&commands) else {
                panic!("read evidence")
            };
            assert_eq!(id, 1);
            events
                .send_blocking(Event::FileIndex(IndexEvent::KnowledgeEvidence {
                    request_id,
                    result: Err(IndexError("Source temporarily unavailable".into())),
                }))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "knowledge-evidence-error", cx);
                reveal(window, "knowledge-evidence-retry", cx);
                window.click("knowledge-evidence-retry", cx);
            })
            .unwrap();
            let IndexCommand::KnowledgeEvidence { request_id, .. } = request(&commands) else {
                panic!()
            };
            let source = page().sources.remove(0);
            let text = "The garden project uses locally grown plants. 茶 ".repeat(25);
            events
                .send_blocking(Event::FileIndex(IndexEvent::KnowledgeEvidence {
                    request_id,
                    result: Ok(Box::new(GraphEvidence {
                        passage: IndexPassage {
                            reference: source.reference.clone(),
                            path: source.path.clone(),
                            text: text.clone(),
                            start_line: 1,
                            end_line: 30,
                            mime_type: "text/plain".into(),
                            note: Some("Only the saved passage is shown.".into()),
                            observed_at: 1,
                            modified_at: None,
                        },
                        source,
                    })),
                }))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "knowledge-evidence-text", cx);
                assert_eq!(
                    window.find("knowledge-evidence-text").label(),
                    Some(text.as_str())
                );
                reveal(window, "knowledge-evidence-back", cx);
                window.click("knowledge-evidence-back", cx);
                window.render_frame(cx);
                reveal(window, ("knowledge-follow-subject", 0usize), cx);
                window.click(("knowledge-follow-subject", 0usize), cx);
            })
            .unwrap();
            let follow = request(&commands);
            assert!(
                matches!(&follow,IndexCommand::BrowseKnowledge { query, .. } if query.entity_id.as_deref()==Some("entity:orchid") && query.before.is_none())
            );
            events.send_blocking(response(&follow, page())).unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "knowledge-saved-stale", cx);
                window.click("knowledge-saved-stale", cx);
            })
            .unwrap();
            let stale = request(&commands);
            assert!(
                matches!(&stale,IndexCommand::BrowseKnowledge { query, .. } if query.include_stale)
            );
            let mut old = page();
            old.relations[0].sources_current = false;
            events.send_blocking(response(&stale, old)).unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "knowledge-saved-history", cx);
                window.click("knowledge-saved-history", cx);
            })
            .unwrap();
            let history = request(&commands);
            assert!(
                matches!(&history, IndexCommand::BrowseKnowledge { query, .. } if query.include_superseded && query.include_stale)
            );
            let mut old = page();
            old.relations[0].sources_current = false;
            old.relations[0].expired_at = Some(2);
            events.send_blocking(response(&history, old)).unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("knowledge-saved-open", 0usize), cx);
                window.click(("knowledge-saved-open", 0usize), cx);
                window.render_frame(cx);
                reveal(window, "knowledge-relationship-status", cx);
                assert_eq!(
                    window.find("knowledge-relationship-status").label(),
                    Some("A helper’s interpretation · not verified · Replaced relationship · Outdated source")
                );
                reveal(window, "knowledge-relationship-back", cx);
                window.click("knowledge-relationship-back", cx);
                window.render_frame(cx);
                reveal(window, "knowledge-saved-back", cx);
                window.click("knowledge-saved-back", cx);
            })
            .unwrap();
            let back = request(&commands);
            assert!(
                matches!(&back, IndexCommand::BrowseKnowledge { query, .. } if !query.include_superseded && query.include_stale)
            );
            let IndexCommand::BrowseKnowledge { request_id, .. } = back else {
                panic!()
            };
            events
                .send_blocking(Event::FileIndex(IndexEvent::KnowledgeBrowsed {
                    request_id,
                    result: Err(IndexError(
                        "The saved view is temporarily unavailable".into(),
                    )),
                }))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "knowledge-saved-error", cx);
                reveal(window, "knowledge-saved-retry", cx);
                window.click("knowledge-saved-retry", cx);
            })
            .unwrap();
            events
                .send_blocking(response(&request(&commands), page()))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "knowledge-saved-search", cx);
                window.click("knowledge-saved-search", cx);
                window.input("missing", cx);
                window.press("enter", cx);
            })
            .unwrap();
            settle(cx);
            let search = request(&commands);
            assert!(
                matches!(&search,IndexCommand::BrowseKnowledge { query, .. } if query.text.as_deref()==Some("missing") && query.entity_id.is_none() && query.before.is_none())
            );
            events
                .send_blocking(response(
                    &search,
                    GraphPage {
                        entities: vec![],
                        relations: vec![],
                        sources: vec![],
                        next: None,
                        truncated: false,
                    },
                ))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "knowledge-saved-empty", cx);
                reveal(window, "knowledge-back-files", cx);
                window.click("knowledge-back-files", cx);
                window.render_frame(cx);
                reveal(window, "knowledge-browse-saved", cx);
                window.click("knowledge-browse-saved", cx);
            })
            .unwrap();
            events
                .send_blocking(response(
                    &request(&commands),
                    GraphPage {
                        entities: vec![],
                        relations: vec![],
                        sources: vec![],
                        next: None,
                        truncated: false,
                    },
                ))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "knowledge-find-helper", cx);
                window.click("knowledge-find-helper", cx);
                window.render_frame(cx);
                assert!(window.try_find("sovra-skills-view").is_some());
                assert!(
                    window.find(("skill-card", 0usize)).label().is_some_and(
                        |label| label.contains("Connect people & topics in your files")
                    )
                );
            })
            .unwrap();
            cx.update_window(window.into(), |_, window, _| window.remove_window())
                .unwrap();
        }
    }
}
