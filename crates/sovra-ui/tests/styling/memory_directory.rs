//! Search and page through the complete subject directory using native controls.

use super::knowledge_profile::{request, respond, reveal, settle};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::{
    MemoryAction, MemoryPage, MemoryReviewResult, MemorySubject, MemorySubjectPage,
    MemorySubjectQuery, NavTab,
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

fn page(names: &[&str], next: Option<&str>) -> MemoryReviewResult {
    MemoryReviewResult::Subjects(MemorySubjectPage {
        subjects: names
            .iter()
            .map(|name| MemorySubject {
                name: (*name).into(),
                details: 2,
            })
            .collect(),
        next: next.map(str::to_owned),
    })
}

#[gpui_kit::test]
fn subject_directory_search_paging_and_selection_fit_every_layout(cx: &mut TestAppContext) {
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
                reveal(window, "memory-browse-subjects", cx);
                window.click("memory-browse-subjects", cx);
            })
            .unwrap();
            let open = request(&commands);
            assert_eq!(
                open.action,
                MemoryAction::ListSubjects(MemorySubjectQuery::default())
            );
            let long_name = "Zebra project with a readable long name ".repeat(6);
            respond(
                &events,
                open.request_id,
                page(&["Mira", &long_name], Some(&long_name)),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("memory-subject-name", 1usize), cx);
                assert_eq!(
                    window.find(("memory-subject-name", 1usize)).label(),
                    Some(long_name.as_str())
                );
                reveal(window, "memory-subjects-next", cx);
                window.click("memory-subjects-next", cx);
            })
            .unwrap();
            let more = request(&commands);
            assert!(
                matches!(more.action, MemoryAction::ListSubjects(query) if query.after.as_deref() == Some(long_name.as_str()))
            );
            respond(&events, more.request_id, page(&["茶"], None));
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(
                    window.find("conversation-memory-status").bounds().top()
                        >= window.find("sovra-knowledge-view").bounds().top(),
                    "a newly opened page starts at the top"
                );
                assert!(window.try_find("memory-subjects-next").is_none());
                reveal(window, "memory-subjects-input", cx);
                window.click("memory-subjects-input", cx);
                window.input("Mira", cx);
                window.press("enter", cx);
            })
            .unwrap();
            settle(cx);
            let search = request(&commands);
            assert!(
                matches!(search.action, MemoryAction::ListSubjects(query) if query.search == "Mira" && query.after.is_none())
            );
            respond(&events, search.request_id, page(&["Mira"], None));
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("memory-subject-open", 0usize), cx);
                window.click(("memory-subject-open", 0usize), cx);
            })
            .unwrap();
            let select = request(&commands);
            assert!(
                matches!(select.action, MemoryAction::List(query) if query.subject.as_deref() == Some("Mira") && query.search.is_empty())
            );
            respond(
                &events,
                select.request_id,
                MemoryReviewResult::Page(MemoryPage::default()),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-search-input", cx);
                window.click("memory-search-input", cx);
                window.input("notes", cx);
                window.press("enter", cx);
            })
            .unwrap();
            settle(cx);
            let search = request(&commands);
            assert!(
                matches!(search.action, MemoryAction::List(query) if query.search == "notes" && query.subject.as_deref() == Some("Mira"))
            );
            respond(
                &events,
                search.request_id,
                MemoryReviewResult::Page(MemoryPage::default()),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-browse-subjects", cx);
                window.click("memory-browse-subjects", cx);
            })
            .unwrap();
            let open = request(&commands);
            events
                .send_blocking(sovra_core::Event::Memory(sovra_core::MemoryEvent::Review(
                    sovra_core::MemoryReviewEvent {
                        request_id: open.request_id,
                        result: Err("Fixture storage failure".into()),
                    },
                )))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-subjects-error", cx);
                assert!(window.try_find("memory-subjects-empty").is_none());
                reveal(window, "memory-subjects-search", cx);
                window.click("memory-subjects-search", cx);
            })
            .unwrap();
            let retry = request(&commands);
            respond(&events, retry.request_id, page(&[], None));
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-subjects-empty", cx);
                assert!(
                    window
                        .find("memory-subjects-empty")
                        .label()
                        .unwrap()
                        .starts_with("No subjects have been identified")
                );
                reveal(window, "memory-subjects-close", cx);
                window.click("memory-subjects-close", cx);
            })
            .unwrap();
            assert!(matches!(request(&commands).action, MemoryAction::List(_)));
        }
    }
}
