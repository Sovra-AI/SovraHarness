//! Opt-in native raster evidence using synthetic protocol events, never user data.
//! A dedicated main keeps `AppKit` initialization on the process main thread.

#[path = "../src/assets.rs"]
mod assets;

use std::{path::Path, sync::Arc, time::Duration};

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, ElementId, HeadlessAppContext, ScrollDelta, point, px, size,
};
use sovra_core::{
    Command, Episode, EpisodeId, EpisodeKind, Event, Fact, FactId, MemoryAction, MemoryDetail,
    MemoryEvent, MemoryJob, MemoryJobState, MemoryPage, MemoryReviewEvent, MemoryReviewResult,
    MemoryStatus, NavTab, Timestamp,
    file_index::{IndexEvent, IndexProgress, IndexStatus, IndexedRoot, ScanOptions},
    models::{Appearance, InterfaceScale, ModelEvent, ModelPreferences},
};

fn settle(cx: &HeadlessAppContext) {
    cx.advance_clock(Duration::from_millis(16));
    cx.run_until_parked();
}

fn detail() -> MemoryDetail {
    let source = Episode {
        id: EpisodeId("episode:synthetic-tea".into()),
        session_id: "Synthetic demonstration conversation".into(),
        kind: EpisodeKind::UserMessage,
        content: "Synthetic example: I prefer jasmine tea with breakfast.".into(),
        occurred_at: Timestamp(1_798_718_400_000),
    };
    MemoryDetail {
        neighborhood: sovra_core::MemoryNeighborhood::default(),
        fact: Fact {
            id: FactId("fact:synthetic-tea".into()),
            content: "Synthetic example: I prefer jasmine tea with breakfast.".into(),
            entities: vec![],
            source: Some(source.id.clone()),
            replaces: None,
            extraction: None,
            created_at: source.occurred_at,
            valid_at: Some(source.occurred_at),
            invalid_at: None,
            expired_at: None,
            stopped_at: None,
        },
        source: Some(source),
    }
}

fn respond(
    commands: &async_channel::Receiver<Command>,
    events: &async_channel::Sender<Event>,
) -> gpui_kit::Result<()> {
    while let Ok(command) = commands.try_recv() {
        if let Command::Memory(command) = command {
            let result = match command.action {
                MemoryAction::List(_) => MemoryReviewResult::Page(MemoryPage {
                    facts: vec![detail().fact],
                    next: None,
                    overview: None,
                    chats: Vec::new(),
                }),
                MemoryAction::ListSources(_) => {
                    MemoryReviewResult::Sources(sovra_core::MemorySourcePage::default())
                }
                MemoryAction::ListSubjects(_) => {
                    MemoryReviewResult::Subjects(sovra_core::MemorySubjectPage::default())
                }
                MemoryAction::Inspect(_) => MemoryReviewResult::Detail(Box::new(detail())),
                MemoryAction::InspectSource(_) => {
                    let MemoryDetail { source, .. } = detail();
                    MemoryReviewResult::Source(sovra_core::MemorySourceRecord {
                        episode: source.expect("synthetic detail includes its source"),
                        metadata: sovra_core::MemorySourceMetadata::default(),
                    })
                }
                MemoryAction::Correct(_)
                | MemoryAction::Remember(_)
                | MemoryAction::RetryFailed
                | MemoryAction::StopUsing(_)
                | MemoryAction::Remove(_)
                | MemoryAction::StopSource(_)
                | MemoryAction::KeepSameDetails(_)
                | MemoryAction::ConfigureSemantic(_) => {
                    panic!("visual proof must not request mutations")
                }
            };
            events.send_blocking(Event::Memory(MemoryEvent::Review(MemoryReviewEvent {
                request_id: command.request_id,
                result: Ok(result),
            })))?;
        }
    }
    Ok(())
}

fn reveal(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    id: impl Into<ElementId>,
) -> gpui_kit::Result<()> {
    let id = id.into();
    cx.update_window(window, |_, window, cx| {
        for _ in 0..100 {
            window.render_frame(cx);
            let target = window.find(id.clone());
            let viewport = window.find("sovra-knowledge-view").bounds();
            let bounds = target.bounds();
            let fits = bounds.size.height <= viewport.size.height;
            if target.visible()
                && bounds.top() >= viewport.top()
                && if fits {
                    bounds.bottom() <= viewport.bottom()
                } else {
                    bounds.top() <= viewport.top() + px(1.)
                }
            {
                return;
            }
            let offset = if bounds.top() < viewport.top() || !fits {
                viewport.top() - bounds.top()
            } else {
                viewport.bottom() - bounds.bottom()
            };
            window.scroll(
                "sovra-knowledge-view",
                ScrollDelta::Pixels(point(px(0.), offset)),
                cx,
            );
        }
        panic!("synthetic screenshot target {id:?} must be reachable");
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
    assert!(image.width() > 0 && image.height() > 0);
    image.save(path)?;
    Ok(())
}

/// Run explicitly with `SOVRA_KNOWLEDGE_VISUAL_OUTPUT` pointing to a new directory.
#[expect(
    clippy::disallowed_methods,
    reason = "opt-in visual evidence reads only its explicit artifact output directory"
)]
fn main() -> gpui_kit::Result<()> {
    let output = std::path::PathBuf::from(std::env::var("SOVRA_KNOWLEDGE_VISUAL_OUTPUT")?);
    std::fs::create_dir(&output)?;
    std::fs::write(
        output.join("README.txt"),
        "Synthetic UI evidence rendered by native GPUI/Metal. These screenshots use scripted loopback events and fictional details, not a real model or the user's saved data. Runtime capture, retrieval and persistence are verified separately by integration tests and live evaluation.\n",
    )?;
    let text_system = gpui_kit::platform::current_platform(true).text_system();
    for (name, width, height, scale) in [
        ("minimum", 640., 420., 100),
        ("standard", 1280., 820., 100),
        ("wide", 2560., 1440., 100),
        ("double-text", 640., 420., 200),
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
            events.send_blocking(Event::FileIndex(IndexEvent::Roots(vec![IndexedRoot {
                id: "synthetic-folder".into(),
                path: "/Synthetic example/Recipes".into(),
                status: IndexStatus::Ready,
                progress: IndexProgress {
                    visited: 3,
                    indexed: 3,
                    ..Default::default()
                },
                issues: vec![],
                completed_at: Some(1),
                auto_refresh: true,
                scan_options: ScanOptions::default(),
            }])))?;
            capture(
                &mut cx,
                window,
                &output.join(format!("{name}-{theme}-files.png")),
            )?;
            cx.update_window(window, |_, window, cx| {
                window.click("knowledge-memories", cx);
            })?;
            respond(&commands, &events)?;
            events.send_blocking(Event::Memory(MemoryEvent::Status(MemoryStatus::Ready)))?;
            events.send_blocking(Event::Memory(MemoryEvent::Processed(MemoryJob {
                episode_id: EpisodeId("episode:synthetic-tea".into()),
                state: MemoryJobState::Complete,
                attempts: 1,
                error: None,
                fact_ids: vec![detail().fact.id],
                candidates: vec![],
                extraction_model: Some("synthetic-visual-fixture".into()),
            })))?;
            settle(&cx);
            respond(&commands, &events)?;
            capture(
                &mut cx,
                window,
                &output.join(format!("{name}-{theme}-memories.png")),
            )?;
            reveal(&mut cx, window, "memory-capture-result")?;
            capture(
                &mut cx,
                window,
                &output.join(format!("{name}-{theme}-saved-detail.png")),
            )?;
            reveal(&mut cx, window, ("memory-open", 0usize))?;
            cx.update_window(window, |_, window, cx| {
                window.click(("memory-open", 0usize), cx);
            })?;
            respond(&commands, &events)?;
            settle(&cx);
            reveal(&mut cx, window, "memory-source")?;
            capture(
                &mut cx,
                window,
                &output.join(format!("{name}-{theme}-source.png")),
            )?;
        }
    }
    Ok(())
}
