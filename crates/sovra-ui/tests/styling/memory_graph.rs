//! The saved-memory picture and the local repeat check stay readable at every size.

#![allow(
    clippy::unwrap_used,
    reason = "headless UI fixtures fail loudly when the scripted runtime is gone"
)]

use super::knowledge_profile::{request, respond, reveal, settle};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ElementId, TestAppContext, Window, px, size};
use sovra_core::{
    Command, Fact, FactId, MemoryAction, MemoryCommand, MemoryFactChat, MemoryPage,
    MemoryReviewResult, NavTab, Timestamp, models::InterfaceScale,
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

fn saved(id: &str, content: &str, names: &[&str], at: i64) -> Fact {
    Fact {
        id: FactId(id.into()),
        content: content.into(),
        entities: names.iter().map(|name| (*name).to_owned()).collect(),
        source: None,
        replaces: None,
        extraction: None,
        created_at: Timestamp(at),
        valid_at: None,
        invalid_at: None,
        expired_at: None,
        stopped_at: None,
    }
}

fn linked_page() -> MemoryPage {
    MemoryPage {
        facts: vec![
            saved("fact:old", "Sam likes mint tea.", &["Sam"], 1),
            saved("fact:school", "Sam likes mint tea at school.", &["Sam"], 2),
            saved("fact:home", "Sam keeps mint tea at home.", &["Sam"], 3),
            saved("fact:new", "sam   likes mint tea!", &["Sam"], 4),
            saved("fact:close", "Sam enjoys mint tea.", &["Sam"], 5),
            saved("fact:gate", "The school gate opens at three.", &[], 6),
            saved("fact:you", "You prefer quiet mornings.", &["you"], 7),
            saved("fact:user", "You prefer quiet evenings.", &["user"], 8),
        ],
        chats: vec![
            MemoryFactChat {
                fact: FactId("fact:school".into()),
                session: "chat-school-a".into(),
                title: "School pickup".into(),
            },
            MemoryFactChat {
                fact: FactId("fact:home".into()),
                session: "chat-school-b".into(),
                title: "School pickup".into(),
            },
            MemoryFactChat {
                fact: FactId("fact:you".into()),
                session: "memory-review".into(),
                title: "memory-review".into(),
            },
        ],
        ..Default::default()
    }
}

fn persisted_page() -> MemoryPage {
    let mut page = linked_page();
    for fact in &mut page.facts {
        if fact.id.0 == "fact:new" || fact.id.0 == "fact:close" {
            fact.replaces = Some(FactId("fact:old".into()));
        }
    }
    page
}

fn chained_page() -> MemoryPage {
    let mut facts = Vec::new();
    for index in 0..30 {
        let mut fact = saved(
            &format!("fact:chain-{index}"),
            &format!("Detail {index} about Sam."),
            &["Sam"],
            index,
        );
        if index > 0 {
            fact.replaces = Some(FactId(format!("fact:chain-{}", index - 1)));
        }
        facts.push(fact);
    }
    MemoryPage {
        facts,
        ..Default::default()
    }
}

fn labels(window: &Window, id: fn(usize) -> ElementId) -> Vec<String> {
    (0..40)
        .filter_map(|index| {
            window
                .try_find(id(index))
                .and_then(|item| item.label().map(str::to_owned))
        })
        .collect()
}

fn node_labels(window: &Window) -> Vec<String> {
    labels(window, |index| ("memory-graph-node", index).into())
}

fn link_labels(window: &Window) -> Vec<String> {
    labels(window, |index| ("memory-graph-link", index).into())
}

fn near_labels(window: &Window) -> Vec<String> {
    labels(window, |index| ("memory-duplicate-near", index).into())
}

fn take_memory(commands: &async_channel::Receiver<Command>) -> Vec<MemoryCommand> {
    std::iter::from_fn(|| commands.try_recv().ok())
        .filter_map(|command| match command {
            Command::Memory(command) => Some(command),
            _ => None,
        })
        .collect()
}

fn reload(
    window: &mut Window,
    commands: &async_channel::Receiver<Command>,
    events: &async_channel::Sender<sovra_core::Event>,
    page: MemoryPage,
    cx: &mut gpui_kit::App,
) {
    reveal(window, "memory-search", cx);
    window.click("memory-search", cx);
    let load = request(commands);
    assert!(matches!(load.action, MemoryAction::List(_)));
    respond(events, load.request_id, MemoryReviewResult::Page(page));
}

#[gpui_kit::test]
fn memory_graph_is_readable_at_every_size(cx: &mut TestAppContext) {
    for (case, (width, height, scale)) in
        [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)]
            .into_iter()
            .enumerate()
    {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let show_omitted = case == 0 && mode == ThemeMode::Dark;
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                Theme::change(mode, None, cx);
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
            assert!(matches!(load.action, MemoryAction::List(_)));
            respond(
                &events,
                load.request_id,
                MemoryReviewResult::Page(MemoryPage::default()),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-graph-empty", cx);
                let empty = window.find("memory-graph-empty");
                assert!(empty.label().unwrap().contains("Nothing is saved yet"));
                reload(
                    window,
                    &commands,
                    &events,
                    MemoryPage {
                        facts: vec![saved("fact:only", "Sam likes mint tea.", &["Sam"], 1)],
                        ..Default::default()
                    },
                    cx,
                );
            })
            .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-graph-single", cx);
                assert!(
                    window
                        .find("memory-graph-single")
                        .label()
                        .unwrap()
                        .contains("One saved detail")
                );
                reveal(window, ("memory-graph-node", 0usize), cx);
                window.click(("memory-graph-node", 0usize), cx);
                window.render_frame(cx);
                reveal(window, "memory-graph-reading", cx);
                assert_eq!(
                    window.find("memory-graph-reading").label(),
                    Some("Sam likes mint tea.")
                );
                reload(window, &commands, &events, linked_page(), cx);
            })
            .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-graph-canvas", cx);
                assert_eq!(
                    window.find("memory-graph-canvas").label(),
                    Some("Picture of saved links")
                );
                let nodes = node_labels(window);
                assert_eq!(
                    nodes
                        .iter()
                        .filter(|label| label.as_str() == "Chat School pickup")
                        .count(),
                    2
                );
                assert!(nodes.iter().all(|label| !label.contains("memory-review")));
                assert!(nodes.iter().all(|label| !label.contains("chat-school")));
                assert!(nodes.iter().any(|label| label.contains("quiet mornings")));
                let links = link_labels(window);
                assert!(
                    links
                        .iter()
                        .any(|label| label.contains("Also mentions Sam"))
                );
                assert!(
                    links
                        .iter()
                        .any(|label| label.contains("Saved from this chat"))
                );
                assert!(
                    links
                        .iter()
                        .all(|label| !label.to_lowercase().contains("you"))
                );
                assert!(links.iter().all(|label| !label.contains("school gate")));
                let school = nodes
                    .iter()
                    .position(|label| label == "Sam likes mint tea at school.")
                    .unwrap();
                reveal(window, ("memory-graph-node", school), cx);
                window.click(("memory-graph-node", school), cx);
                window.render_frame(cx);
                reveal(window, "memory-graph-reading", cx);
                assert_eq!(
                    window.find("memory-graph-reading").label(),
                    Some("Sam likes mint tea at school.")
                );
                let chat = node_labels(window)
                    .iter()
                    .position(|label| label == "Chat School pickup")
                    .unwrap();
                reveal(window, ("memory-graph-node", chat), cx);
                window.click(("memory-graph-node", chat), cx);
                window.render_frame(cx);
                reveal(window, "memory-graph-reading", cx);
                assert_eq!(
                    window.find("memory-graph-reading").label(),
                    Some("These details were saved from the chat School pickup.")
                );
                assert!(window.try_find("memory-duplicate-exact").is_none());
                reveal(window, "memory-duplicate-check", cx);
                window.click("memory-duplicate-check", cx);
            })
            .unwrap();
            let sent = take_memory(&commands);
            assert_ne!(sent.len(), 0);
            let link = sent
                .into_iter()
                .next()
                .expect("the repeat check stores the exact pair");
            let MemoryAction::KeepSameDetails(links) = link.action else {
                panic!("the repeat check stores the exact pair");
            };
            assert!(
                links
                    .iter()
                    .any(|pair| pair.newer.0 == "fact:new" && pair.older.0 == "fact:old")
            );
            respond(&events, link.request_id, MemoryReviewResult::VersionLinked);
            assert_eq!(take_memory(&commands).len(), 0);
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-duplicate-exact", cx);
                let exact = window.find("memory-duplicate-exact");
                let exact = exact.label().unwrap();
                assert!(exact.contains("Both stay"));
                assert!(exact.contains("earlier version"));
                assert!(exact.contains("Sam likes mint tea."));
                assert!(exact.contains("sam   likes mint tea!"));
                assert!(!exact.contains("school gate"));
                reveal(window, ("memory-duplicate-near", 0usize), cx);
                let near = near_labels(window);
                assert!(
                    near.iter()
                        .any(|label| label.contains("Sam enjoys mint tea."))
                );
                assert!(near.iter().all(|label| label.contains("Nothing is joined")));
                assert!(near.iter().all(|label| !label.contains("school gate")));
                let before = link_labels(window)
                    .iter()
                    .filter(|label| label.contains("Earlier version"))
                    .count();
                let confirm = near
                    .iter()
                    .position(|label| label.contains("Sam enjoys mint tea."))
                    .unwrap();
                reveal(window, ("memory-duplicate-confirm", confirm), cx);
                window.click(("memory-duplicate-confirm", confirm), cx);
                window.render_frame(cx);
                let after = link_labels(window)
                    .iter()
                    .filter(|label| label.contains("Earlier version"))
                    .count();
                assert!(after > before);
                assert!(
                    near_labels(window)
                        .iter()
                        .all(|label| !label.contains("Sam enjoys mint tea."))
                );
                let nodes = node_labels(window);
                assert!(
                    nodes
                        .iter()
                        .any(|label| label.contains("Sam enjoys mint tea."))
                );
                assert!(
                    nodes
                        .iter()
                        .any(|label| label.contains("Sam likes mint tea."))
                );
            })
            .unwrap();
            let sent = take_memory(&commands);
            assert_ne!(sent.len(), 0);
            let confirm_link = sent
                .into_iter()
                .next()
                .expect("Treat as the same detail stores the earlier-version link");
            let MemoryAction::KeepSameDetails(links) = confirm_link.action else {
                panic!("Treat as the same detail stores the earlier-version link");
            };
            assert!(
                links
                    .iter()
                    .any(|pair| pair.newer.0 == "fact:close" && pair.older.0 == "fact:old")
            );
            respond(
                &events,
                confirm_link.request_id,
                MemoryReviewResult::VersionLinked,
            );
            assert_eq!(take_memory(&commands).len(), 0);
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reload(window, &commands, &events, persisted_page(), cx);
            })
            .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                let earlier = link_labels(window)
                    .iter()
                    .filter(|label| label.contains("Earlier version"))
                    .count();
                assert!(earlier >= 2);
                reveal(window, "memory-duplicate-check", cx);
                window.click("memory-duplicate-check", cx);
                window.render_frame(cx);
                assert!(
                    near_labels(window)
                        .iter()
                        .all(|label| !label.contains("Sam enjoys mint tea."))
                );
                assert_eq!(take_memory(&commands).len(), 0);
                if show_omitted {
                    reload(window, &commands, &events, chained_page(), cx);
                }
            })
            .unwrap();
            if show_omitted {
                settle(cx);
                cx.update_window(window.into(), |_, window, cx| {
                    reveal(window, "memory-graph-more", cx);
                    assert!(
                        window
                            .find("memory-graph-more")
                            .label()
                            .unwrap()
                            .contains("More links exist")
                    );
                })
                .unwrap();
            }
        }
    }
}
