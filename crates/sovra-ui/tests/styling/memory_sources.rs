//! Browse original evidence, inspect provenance, and follow confirmed withdrawal.

use super::knowledge_profile::{request, respond, reveal, settle};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::{
    Episode, EpisodeId, EpisodeKind, MemoryAction, MemoryPage, MemoryReviewResult,
    MemorySourceCursor, MemorySourceMetadata, MemorySourceOrigin, MemorySourcePage,
    MemorySourceQuery, MemorySourceRecord, MemorySourceScope, MemorySourceSummary, NavTab,
    Timestamp,
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

pub(super) fn record(stopped: bool) -> MemorySourceRecord {
    MemorySourceRecord {
        episode: Episode {
            id: EpisodeId("episode:original".into()),
            session_id: "fixture".into(),
            kind: EpisodeKind::UserMessage,
            content: format!(
                "{} The original needle is here.",
                "A readable original sentence. 茶 ".repeat(30)
            ),
            occurred_at: Timestamp(0),
        },
        metadata: MemorySourceMetadata {
            origin: MemorySourceOrigin::UserStatement,
            scope: MemorySourceScope::Private,
            version: Some(1),
            revoked_at: stopped.then_some(Timestamp(1)),
            processing: Some(sovra_core::MemoryJobState::Complete),
            detail_count: Some(1),
            ..Default::default()
        },
    }
}
pub(super) fn source_page(stopped: bool, more: bool) -> MemoryReviewResult {
    let record = record(stopped);
    MemoryReviewResult::Sources(MemorySourcePage {
        sources: vec![MemorySourceSummary {
            id: record.episode.id.clone(),
            kind: EpisodeKind::UserMessage,
            occurred_at: Timestamp(0),
            excerpt: "A saved original sentence. 茶".into(),
            shortened: true,
            metadata: record.metadata,
        }],
        next: more.then_some(MemorySourceCursor {
            id: record.episode.id,
            occurred_at: Timestamp(0),
        }),
    })
}

#[gpui_kit::test]
fn original_source_browser_search_history_and_links_fit_every_layout(cx: &mut TestAppContext) {
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
            respond(
                &events,
                load.request_id,
                MemoryReviewResult::Page(MemoryPage::default()),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-browse-sources", cx);
                window.click("memory-browse-sources", cx);
            })
            .unwrap();
            let browse = request(&commands);
            assert_eq!(
                browse.action,
                MemoryAction::ListSources(MemorySourceQuery::default())
            );
            respond(&events, browse.request_id, source_page(false, true));
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("memory-source-state", 0usize), cx);
                assert_eq!(
                    window.find(("memory-source-state", 0usize)).label(),
                    Some("Evidence saved · check finished")
                );
                reveal(window, "memory-sources-next", cx);
                window.click("memory-sources-next", cx);
            })
            .unwrap();
            let older = request(&commands);
            assert!(
                matches!(older.action, MemoryAction::ListSources(query) if query.before.is_some())
            );
            respond(&events, older.request_id, source_page(false, false));
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
                matches!(search.action, MemoryAction::ListSources(query) if query.search == "needle" && query.before.is_none())
            );
            respond(&events, search.request_id, source_page(false, false));
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("memory-source-open", 0usize), cx);
                window.click(("memory-source-open", 0usize), cx);
            })
            .unwrap();
            let open = request(&commands);
            assert_eq!(
                open.action,
                MemoryAction::InspectSource(record(false).episode.id)
            );
            respond(
                &events,
                open.request_id,
                MemoryReviewResult::Source(record(false)),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                assert!(window.try_find("conversation-memory-status").is_none());
                reveal(window, "memory-source-origin", cx);
                assert_eq!(
                    window.find("memory-source-origin").label(),
                    Some("A statement from you")
                );
                reveal(window, "memory-source", cx);
                assert_eq!(
                    window.find("memory-source").label(),
                    Some(record(false).episode.content.as_str())
                );
                reveal(window, "memory-stop-source", cx);
                window.click("memory-stop-source", cx);
            })
            .unwrap();
            assert!(
                !std::iter::from_fn(|| commands.try_recv().ok())
                    .any(|command| matches!(command, sovra_core::Command::Memory(_)))
            );
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-confirm-stop-source", cx);
                window.click("memory-confirm-stop-source", cx);
            })
            .unwrap();
            let stop = request(&commands);
            assert_eq!(
                stop.action,
                MemoryAction::StopSource(record(false).episode.id)
            );
            respond(
                &events,
                stop.request_id,
                MemoryReviewResult::SourceStopped(record(false).episode.id),
            );
            settle(cx);
            let refresh = request(&commands);
            assert!(
                matches!(refresh.action, MemoryAction::ListSources(query) if query.search == "needle" && !query.include_stopped)
            );
            respond(
                &events,
                refresh.request_id,
                MemoryReviewResult::Sources(MemorySourcePage::default()),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-sources-empty", cx);
                reveal(window, "memory-sources-stopped", cx);
                window.click("memory-sources-stopped", cx);
            })
            .unwrap();
            let history = request(&commands);
            assert!(
                matches!(history.action, MemoryAction::ListSources(query) if query.include_stopped && query.search == "needle")
            );
            respond(&events, history.request_id, source_page(true, false));
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("memory-source-open", 0usize), cx);
                window.click(("memory-source-open", 0usize), cx);
            })
            .unwrap();
            let open = request(&commands);
            respond(
                &events,
                open.request_id,
                MemoryReviewResult::Source(record(true)),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-source-status", cx);
                assert_eq!(
                    window.find("memory-source-status").label(),
                    Some("Stopped using · retained for review")
                );
                reveal(window, "memory-stop-source", cx);
                assert_eq!(window.find("memory-stop-source").disabled(), Some(true));
                reveal(window, "memory-source-details", cx);
                window.click("memory-source-details", cx);
            })
            .unwrap();
            let linked = request(&commands);
            assert!(
                matches!(linked.action, MemoryAction::List(query) if query.source == Some(record(false).episode.id) && query.include_history && query.subject.is_none())
            );
            respond(
                &events,
                linked.request_id,
                MemoryReviewResult::Page(MemoryPage::default()),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-clear-source", cx);
                window.click("memory-clear-source", cx);
            })
            .unwrap();
            assert!(
                matches!(request(&commands).action, MemoryAction::List(query) if query.source.is_none())
            );
        }
    }
}

#[gpui_kit::test]
fn named_team_source_is_readable_at_minimum_wide_and_double_size(cx: &mut TestAppContext) {
    for (mode, width, height, scale) in [
        (ThemeMode::Dark, 640., 420., 100),
        (ThemeMode::Light, 640., 420., 100),
        (ThemeMode::Dark, 2560., 1440., 100),
        (ThemeMode::Light, 2560., 1440., 100),
        (ThemeMode::Dark, 640., 420., 200),
        (ThemeMode::Light, 640., 420., 200),
    ] {
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
        respond(
            &events,
            load.request_id,
            MemoryReviewResult::Page(MemoryPage::default()),
        );
        settle(cx);
        cx.update_window(window.into(), |_, window, cx| {
            reveal(window, "memory-browse-sources", cx);
            window.click("memory-browse-sources", cx);
        })
        .unwrap();
        let mut source = record(false);
        source.episode.kind = EpisodeKind::AgentMessage;
        source.metadata.origin = MemorySourceOrigin::AssistantOutput;
        source.metadata.processing = None;
        source.metadata.detail_count = Some(0);
        source.metadata.attribution = Some(sovra_core::MemorySourceAttribution {
            producer_key: "team-reply".into(),
            author: sovra_core::MemorySourceAuthor::Assistant {
                speaker: Some("Priya — planning and technical writing helper".into()),
            },
            provenance: sovra_core::MemorySourceProvenance::Team {
                run_id: "run".into(),
                member_id: "priya".into(),
            },
        });
        let browse = request(&commands);
        respond(
            &events,
            browse.request_id,
            MemoryReviewResult::Sources(MemorySourcePage {
                sources: vec![MemorySourceSummary {
                    id: source.episode.id.clone(),
                    kind: source.episode.kind,
                    occurred_at: source.episode.occurred_at,
                    excerpt: "A fictional reply.".into(),
                    shortened: false,
                    metadata: source.metadata.clone(),
                }],
                next: None,
            }),
        );
        settle(cx);
        cx.update_window(window.into(), |_, window, cx| {
            reveal(window, ("memory-source-open", 0usize), cx);
            window.click(("memory-source-open", 0usize), cx);
        })
        .unwrap();
        let inspect = request(&commands);
        respond(
            &events,
            inspect.request_id,
            MemoryReviewResult::Source(source),
        );
        settle(cx);
        cx.update_window(window.into(), |_, window, cx| {
            reveal(window, "memory-source-origin", cx);
            assert_eq!(
                window.find("memory-source-origin").label(),
                Some("A reply from Priya — planning and technical writing helper")
            );
            reveal(window, "memory-stop-source", cx);
        })
        .unwrap();
    }
}
