//! Opt-in screenshots of the knowledge map using fictional details, never user data.
//! Run with `SOVRA_KNOWLEDGE_MAP_OUTPUT` pointing to a new directory.

#[path = "../src/assets.rs"]
mod assets;

use std::{path::Path, sync::Arc, time::Duration};

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, ElementId, HeadlessAppContext, ScrollDelta, point, px, size,
};
use sovra_core::{
    Command, Event, Fact, FactId, MemoryAction, MemoryEvent, MemoryFactChat, MemoryPage,
    MemoryReviewEvent, MemoryReviewResult, MemoryStatus, NavTab, Timestamp,
    models::{Appearance, InterfaceScale, ModelEvent, ModelPreferences},
};

fn settle(cx: &HeadlessAppContext) {
    for _ in 0..4 {
        cx.advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
}

fn saved(id: &str, content: &str, names: &[&str], source: Option<&str>, at: i64) -> Fact {
    Fact {
        id: FactId(id.into()),
        content: content.into(),
        entities: names.iter().map(|name| (*name).to_owned()).collect(),
        source: source.map(|source| sovra_core::EpisodeId(source.into())),
        replaces: None,
        extraction: None,
        created_at: Timestamp(1_798_718_400_000 + at * 86_400_000),
        valid_at: None,
        invalid_at: None,
        expired_at: None,
        stopped_at: None,
    }
}

fn page() -> MemoryPage {
    let mut facts = vec![
        saved(
            "fact:maya-sister",
            "Maya is my younger sister.",
            &["Maya"],
            Some("m1"),
            1,
        ),
        saved(
            "fact:maya-bday",
            "Maya's birthday is on 14 March.",
            &["Maya"],
            Some("m1"),
            2,
        ),
        saved(
            "fact:maya-oslo",
            "Maya moved to Oslo for work.",
            &["Maya", "Oslo"],
            None,
            3,
        ),
        saved(
            "fact:oslo-trip",
            "Planning a trip to Oslo in May.",
            &["Oslo"],
            Some("m2"),
            4,
        ),
        saved(
            "fact:flights",
            "Prefers morning flights.",
            &["Oslo"],
            Some("m2"),
            5,
        ),
        saved(
            "fact:garden-tomato",
            "Growing three kinds of tomato this year.",
            &["Garden"],
            Some("m3"),
            6,
        ),
        saved(
            "fact:garden-basil",
            "Basil goes next to the tomatoes.",
            &["Garden"],
            Some("m3"),
            7,
        ),
        saved(
            "fact:garden-water",
            "Waters the garden before work.",
            &["Garden"],
            None,
            8,
        ),
        saved(
            "fact:atlas-launch",
            "Project Atlas launches in June.",
            &["Project Atlas"],
            Some("m4"),
            9,
        ),
        saved(
            "fact:atlas-lead",
            "Leads the design team on Project Atlas.",
            &["Project Atlas"],
            Some("m4"),
            10,
        ),
        saved(
            "fact:atlas-review",
            "Atlas review every Thursday.",
            &["Project Atlas"],
            None,
            11,
        ),
        saved(
            "fact:tea",
            "Likes jasmine tea with breakfast.",
            &[],
            None,
            12,
        ),
        saved(
            "fact:tea-old",
            "Likes green tea with breakfast.",
            &[],
            None,
            0,
        ),
    ];
    facts[11].replaces = Some(FactId("fact:tea-old".into()));
    let chat = |fact: &str, session: &str, title: &str| MemoryFactChat {
        fact: FactId(fact.into()),
        session: session.into(),
        title: title.into(),
    };
    MemoryPage {
        facts,
        chats: vec![
            chat("fact:maya-oslo", "chat-family", "Family catch-up"),
            chat("fact:maya-bday", "chat-family", "Family catch-up"),
            chat("fact:oslo-trip", "chat-travel", "Spring travel"),
            chat("fact:flights", "chat-travel", "Spring travel"),
            chat("fact:atlas-launch", "chat-work", "Atlas planning"),
            chat("fact:atlas-review", "chat-work", "Atlas planning"),
        ],
        ..Default::default()
    }
}

fn respond(
    commands: &async_channel::Receiver<Command>,
    events: &async_channel::Sender<Event>,
) -> gpui_kit::Result<()> {
    while let Ok(command) = commands.try_recv() {
        if let Command::Memory(command) = command {
            let result = match command.action {
                MemoryAction::List(_) => MemoryReviewResult::Page(page()),
                MemoryAction::ListSources(_) => {
                    MemoryReviewResult::Sources(sovra_core::MemorySourcePage::default())
                }
                MemoryAction::ListSubjects(_) => {
                    MemoryReviewResult::Subjects(sovra_core::MemorySubjectPage::default())
                }
                other => panic!("map screenshots must not request {other:?}"),
            };
            events.send_blocking(Event::Memory(MemoryEvent::Review(MemoryReviewEvent {
                request_id: command.request_id,
                result: Ok(result),
            })))?;
        }
    }
    Ok(())
}

/// Scrolls the knowledge view until the map's top is at the top of the viewport.
fn reveal(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    id: impl Into<ElementId>,
) -> gpui_kit::Result<()> {
    let id = id.into();
    cx.update_window(window, |_, window, cx| {
        for _ in 0..100 {
            window.render_frame(cx);
            let target = window.find(id.clone()).bounds();
            let viewport = window.find("sovra-knowledge-view").bounds();
            let offset = viewport.top() - target.top() + px(12.);
            if offset.abs() < px(2.) {
                return;
            }
            window.scroll(
                "sovra-knowledge-view",
                ScrollDelta::Pixels(point(px(0.), offset)),
                cx,
            );
            let moved = window.find(id.clone()).bounds();
            if (moved.top() - target.top()).abs() < px(1.) {
                return;
            }
        }
    })?;
    Ok(())
}

fn capture(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    path: &Path,
) -> gpui_kit::Result<()> {
    settle(cx);
    cx.update_window(window, |_, window, cx| window.render_frame(cx))?;
    let image = cx.capture_screenshot(window)?;
    image.save(path)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "opt-in visual evidence reads only its explicit artifact output directory"
)]
fn main() -> gpui_kit::Result<()> {
    let output = std::path::PathBuf::from(std::env::var("SOVRA_KNOWLEDGE_MAP_OUTPUT")?);
    std::fs::create_dir(&output)?;
    let text_system = gpui_kit::platform::current_platform(true).text_system();
    for (name, width, height, scale) in [
        ("standard", 1280., 820., 100),
        ("wide", 2560., 1440., 100),
        ("minimum", 640., 420., 100),
    ] {
        for (theme, mode) in [("dark", ThemeMode::Dark), ("light", ThemeMode::Light)] {
            let mut cx = HeadlessAppContext::with_platform(
                text_system.clone(),
                Arc::new(assets::Assets),
                gpui_kit::platform::current_headless_renderer,
            );
            let (client, commands, events) = sovra_runtime::Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                sovra_ui::design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| {
                    let mut shell = sovra_ui::Shell::new(0, window, cx);
                    shell.select_tab(NavTab::Knowledge, cx);
                    shell
                });
                cx.new(|cx| Root::new(view, window, cx))
            })?;
            let window = window.into();
            events.send_blocking(Event::Models(ModelEvent::Loaded {
                connections: vec![],
                preferences: ModelPreferences {
                    appearance: if mode.is_dark() {
                        Appearance::Dark
                    } else {
                        Appearance::Light
                    },
                    interface_scale: InterfaceScale::from_percent(scale),
                    ..Default::default()
                },
            }))?;
            settle(&cx);
            cx.update_window(window, |_, window, cx| {
                window.click("knowledge-memories", cx);
            })?;
            events.send_blocking(Event::Memory(MemoryEvent::Status(MemoryStatus::Ready)))?;
            settle(&cx);
            respond(&commands, &events)?;
            settle(&cx);
            respond(&commands, &events)?;
            reveal(&mut cx, window, "memory-graph")?;
            capture(
                &mut cx,
                window,
                &output.join(format!("{name}-{theme}-map.png")),
            )?;
            // Point at the best-linked detail, then select it.
            cx.update_window(window, |_, window, cx| {
                let target = (0..40_usize)
                    .find_map(|index| {
                        let id: ElementId = ("memory-graph-node", index).into();
                        let found = window.try_find(id)?;
                        found
                            .label()
                            .is_some_and(|label| label.contains("Oslo for work"))
                            .then(|| found.bounds().center())
                    })
                    .expect("the Oslo detail is on the map");
                window.pointer_move(target, None, cx);
            })?;
            capture(
                &mut cx,
                window,
                &output.join(format!("{name}-{theme}-hover.png")),
            )?;
            cx.update_window(window, |_, window, cx| {
                let target = (0..40_usize)
                    .find_map(|index| {
                        let id: ElementId = ("memory-graph-node", index).into();
                        let found = window.try_find(id)?;
                        found
                            .label()
                            .is_some_and(|label| label.contains("Project Atlas launches"))
                            .then(|| found.bounds().center())
                    })
                    .expect("the Atlas detail is on the map");
                window.pointer_down(target, gpui_kit::MouseButton::Left, cx);
                window.pointer_up(target, gpui_kit::MouseButton::Left, cx);
            })?;
            capture(
                &mut cx,
                window,
                &output.join(format!("{name}-{theme}-selected.png")),
            )?;
        }
    }
    Ok(())
}
