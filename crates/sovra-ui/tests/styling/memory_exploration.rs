//! Follow actual saved links and filter subjects without losing the return path.

use super::knowledge_profile::{request, respond, reveal, settle};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::{
    Episode, EpisodeId, EpisodeKind, Fact, FactId, MemoryAction, MemoryConnection,
    MemoryConnectionReason, MemoryDetail, MemoryNeighborhood, MemoryPage, MemoryReviewResult,
    NavTab, Timestamp,
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

fn detail(name: &str) -> MemoryDetail {
    MemoryDetail {
        fact: Fact {
            id: FactId(format!("fact:{name}")),
            content: format!("Mira keeps the {name} project notes."),
            entities: vec!["Mira".into(), name.into()],
            source: Some(EpisodeId(format!("episode:{name}"))),
            replaces: None,
            extraction: None,
            created_at: Timestamp(0),
            valid_at: Some(Timestamp(0)),
            invalid_at: None,
            expired_at: None,
            stopped_at: None,
        },
        source: Some(Episode {
            id: EpisodeId(format!("episode:{name}")),
            session_id: "fixture".into(),
            kind: EpisodeKind::UserMessage,
            content: "Mira keeps the project notes.".into(),
            occurred_at: Timestamp(0),
        }),
        neighborhood: MemoryNeighborhood::default(),
    }
}

#[gpui_kit::test]
fn connected_knowledge_is_readable_navigable_and_filterable_at_every_size(cx: &mut TestAppContext) {
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
            let load = request(&commands);
            let mut cedar = detail("Cedar");
            let birch = detail("Birch");
            let source_text = format!(
                "{}Final source sentence.",
                "Mira keeps the project notes. ".repeat(30)
            );
            cedar.source.as_mut().unwrap().content = source_text.clone();
            cedar.neighborhood.connections.push(MemoryConnection {
                fact: birch.fact.clone(),
                reason: MemoryConnectionReason::SharedSubjects(vec!["Mira".into()]),
            });
            respond(
                &events,
                load.request_id,
                MemoryReviewResult::Page(MemoryPage {
                    facts: vec![cedar.fact.clone()],
                    ..Default::default()
                }),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("memory-card-text", 0usize), cx);
                assert_eq!(
                    window.find(("memory-card-text", 0usize)).label(),
                    Some("Mira keeps the Cedar project notes.")
                );
                reveal(window, ("memory-open", 0usize), cx);
                window.click(("memory-open", 0usize), cx);
            })
            .unwrap();
            let open = request(&commands);
            assert_eq!(open.action, MemoryAction::Inspect(cedar.fact.id.clone()));
            respond(
                &events,
                open.request_id,
                MemoryReviewResult::Detail(Box::new(cedar.clone())),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-learned-date", cx);
                assert_eq!(
                    window.find("memory-learned-date").label(),
                    Some("Saved 1 January 1970 · dates shown in UTC")
                );
                window.render_frame(cx);
                let source_bounds = window.find("memory-source").bounds();
                let pane_bounds = window.find("sovra-knowledge-view").bounds();
                assert!(
                    source_bounds.left() >= pane_bounds.left()
                        && source_bounds.right() <= pane_bounds.right()
                );
                assert!(window.find("memory-source").label().unwrap().ends_with('…'));
                reveal(window, "memory-expand-source", cx);
                window.click("memory-expand-source", cx);
                window.render_frame(cx);
                assert_eq!(
                    window.find("memory-source").label(),
                    Some(source_text.as_str())
                );
                reveal(window, "memory-expand-source", cx);
                window.click("memory-expand-source", cx);
                window.render_frame(cx);
                reveal(window, ("memory-connection-reason", 0usize), cx);
                assert_eq!(
                    window.find(("memory-connection-reason", 0usize)).label(),
                    Some("Also mentions Mira")
                );
                reveal(window, ("memory-connection-open", 0usize), cx);
                window.click(("memory-connection-open", 0usize), cx);
            })
            .unwrap();
            let open = request(&commands);
            assert_eq!(open.action, MemoryAction::Inspect(birch.fact.id.clone()));
            respond(
                &events,
                open.request_id,
                MemoryReviewResult::Detail(Box::new(birch.clone())),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-supporting-records", cx);
                window.click("memory-supporting-records", cx);
            })
            .unwrap();
            let supporting = request(&commands);
            assert!(
                matches!(&supporting.action, MemoryAction::ListSources(query) if query.fact.as_ref() == Some(&birch.fact.id) && query.search.is_empty())
            );
            respond(
                &events,
                supporting.request_id,
                super::memory_sources::source_page(false, true),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                assert!(window.try_find("conversation-memory-status").is_none());
                reveal(window, "memory-supported-detail", cx);
                assert_eq!(
                    window.find("memory-supported-detail").label(),
                    Some(birch.fact.content.as_str())
                );
                reveal(window, "memory-sources-next", cx);
                window.click("memory-sources-next", cx);
            })
            .unwrap();
            let older = request(&commands);
            assert!(
                matches!(&older.action, MemoryAction::ListSources(query) if query.fact.as_ref() == Some(&birch.fact.id) && query.before.is_some())
            );
            respond(
                &events,
                older.request_id,
                super::memory_sources::source_page(false, false),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-sources-input", cx);
                window.click("memory-sources-input", cx);
                window.input("needle", cx);
                window.press("enter", cx);
            })
            .unwrap();
            settle(cx);
            let search = request(&commands);
            assert!(
                matches!(&search.action, MemoryAction::ListSources(query) if query.fact.as_ref() == Some(&birch.fact.id) && query.before.is_none() && query.search == "needle")
            );
            respond(
                &events,
                search.request_id,
                super::memory_sources::source_page(false, false),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-sources-stopped", cx);
                window.click("memory-sources-stopped", cx);
            })
            .unwrap();
            let stopped = request(&commands);
            assert!(
                matches!(&stopped.action, MemoryAction::ListSources(query) if query.fact.as_ref() == Some(&birch.fact.id) && query.include_stopped && query.search == "needle")
            );
            respond(
                &events,
                stopped.request_id,
                super::memory_sources::source_page(true, false),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("memory-source-open", 0usize), cx);
                window.click(("memory-source-open", 0usize), cx);
            })
            .unwrap();
            let source = request(&commands);
            assert_eq!(
                source.action,
                MemoryAction::InspectSource(super::memory_sources::record(true).episode.id)
            );
            respond(
                &events,
                source.request_id,
                MemoryReviewResult::Source(super::memory_sources::record(true)),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-close-source", cx);
                window.click("memory-close-source", cx);
            })
            .unwrap();
            let records = request(&commands);
            assert!(
                matches!(&records.action, MemoryAction::ListSources(query) if query.fact.as_ref() == Some(&birch.fact.id) && query.include_stopped && query.search == "needle")
            );
            respond(
                &events,
                records.request_id,
                super::memory_sources::source_page(true, false),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-sources-back-detail", cx);
                window.click("memory-sources-back-detail", cx);
            })
            .unwrap();
            let back_detail = request(&commands);
            assert_eq!(
                back_detail.action,
                MemoryAction::Inspect(birch.fact.id.clone())
            );
            respond(
                &events,
                back_detail.request_id,
                MemoryReviewResult::Detail(Box::new(birch)),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-trail-back", cx);
                window.click("memory-trail-back", cx);
            })
            .unwrap();
            let back = request(&commands);
            assert_eq!(back.action, MemoryAction::Inspect(cedar.fact.id.clone()));
            respond(
                &events,
                back.request_id,
                MemoryReviewResult::Detail(Box::new(cedar)),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("memory-detail-subject", 0usize), cx);
                window.click(("memory-detail-subject", 0usize), cx);
            })
            .unwrap();
            let filter = request(&commands);
            let MemoryAction::List(query) = filter.action else {
                panic!("subject filter")
            };
            assert_eq!(query.subject.as_deref(), Some("Mira"));
            assert!(query.before.is_none());
            respond(
                &events,
                filter.request_id,
                MemoryReviewResult::Page(MemoryPage::default()),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(
                    window
                        .find("memory-empty")
                        .label()
                        .unwrap()
                        .starts_with("No saved details match this subject")
                );
                reveal(window, "memory-clear-subject", cx);
                window.click("memory-clear-subject", cx);
            })
            .unwrap();
            let clear = request(&commands);
            let MemoryAction::List(query) = clear.action else {
                panic!("clear filter")
            };
            assert_eq!(query.subject, None);
            let overview = sovra_core::MemoryOverview {
                subjects: vec![
                    sovra_core::MemorySubject {
                        name: "Mira".into(),
                        details: 2,
                    },
                    sovra_core::MemorySubject {
                        name: "A long saved project name ".repeat(8),
                        details: 1,
                    },
                ],
                recent_activity: vec![sovra_core::MemoryActivity {
                    episode_id: EpisodeId("episode:empty-check".into()),
                    session_id: "fixture".into(),
                    created_at: Timestamp(0),
                    state: sovra_core::MemoryJobState::Complete,
                    saved: 0,
                    error: None,
                }],
                ..Default::default()
            };
            respond(
                &events,
                clear.request_id,
                MemoryReviewResult::Page(MemoryPage {
                    overview: Some(overview.clone()),
                    ..Default::default()
                }),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("memory-subject", 1usize), cx);
                reveal(window, ("memory-subject", 0usize), cx);
                assert_eq!(
                    window.find(("memory-subject", 0usize)).label(),
                    Some("Mira · 2")
                );
                window.click(("memory-subject", 0usize), cx);
            })
            .unwrap();
            let filter = request(&commands);
            assert!(
                matches!(filter.action, MemoryAction::List(query) if query.subject.as_deref() == Some("Mira"))
            );
            respond(
                &events,
                filter.request_id,
                MemoryReviewResult::Page(MemoryPage {
                    overview: Some(overview),
                    ..Default::default()
                }),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-activity-toggle", cx);
                window.click("memory-activity-toggle", cx);
                window.render_frame(cx);
                reveal(window, ("memory-activity-source", 0usize), cx);
                window.click(("memory-activity-source", 0usize), cx);
            })
            .unwrap();
            let source = request(&commands);
            assert_eq!(
                source.action,
                MemoryAction::InspectSource(EpisodeId("episode:empty-check".into()))
            );
            let mut original = detail("Cedar").source.unwrap();
            original.id = EpisodeId("episode:empty-check".into());
            original.content = "Evidence remains readable when no details were extracted.".into();
            respond(
                &events,
                source.request_id,
                MemoryReviewResult::Source(sovra_core::MemorySourceRecord {
                    episode: original,
                    metadata: sovra_core::MemorySourceMetadata::default(),
                }),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                let source_bounds = window.find("memory-source").bounds();
                let pane_bounds = window.find("sovra-knowledge-view").bounds();
                assert!(
                    source_bounds.left() >= pane_bounds.left()
                        && source_bounds.right() <= pane_bounds.right()
                );
                assert_eq!(
                    window.find("memory-source").label(),
                    Some("Evidence remains readable when no details were extracted.")
                );
                assert!(window.try_find("memory-profile").is_none());
                assert!(window.try_find("memory-back-chat").is_none());
                reveal(window, "memory-close-source", cx);
                window.click("memory-close-source", cx);
            })
            .unwrap();
            assert!(matches!(request(&commands).action, MemoryAction::List(_)));
        }
    }
}
