//! Knowledge and the other workspace pages fit the pane, the text size, and a wide window.

#![allow(
    clippy::unwrap_used,
    reason = "headless UI fixtures fail loudly when the scripted runtime is gone"
)]

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    App, AppContext as _, ElementId, Pixels, ScrollDelta, TestAppContext, Window, point, px, size,
};
use sovra_core::{
    Command, Episode, EpisodeId, EpisodeKind, Fact, FactId, MemoryAction, MemoryCommand,
    MemoryPage, MemoryReviewResult, MemorySourceMetadata, MemorySourceRecord, NavTab, Timestamp,
    models::InterfaceScale,
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

use super::knowledge_profile::{request, respond, settle};

fn frames(window: &mut Window, cx: &mut App) {
    for _ in 0..4 {
        window.render_frame(cx);
    }
}

fn fact(id: &str, content: &str) -> Fact {
    Fact {
        id: FactId(id.into()),
        content: content.into(),
        entities: vec!["Sam".into()],
        source: None,
        replaces: None,
        extraction: None,
        created_at: Timestamp(1),
        valid_at: None,
        invalid_at: None,
        expired_at: None,
        stopped_at: None,
    }
}

fn memory_page() -> MemoryPage {
    MemoryPage {
        facts: vec![
            fact(
                "fact:tea",
                "Sam likes mint tea and asks for it after school on quiet afternoons.",
            ),
            fact(
                "fact:home",
                "Sam keeps mint tea at home, next to the blue tin on the second shelf.",
            ),
        ],
        ..Default::default()
    }
}

fn take_memory(commands: &async_channel::Receiver<Command>) -> Vec<MemoryCommand> {
    std::iter::from_fn(|| commands.try_recv().ok())
        .filter_map(|command| match command {
            Command::Memory(command) => Some(command),
            _ => None,
        })
        .collect()
}

fn inside_window(bounds: gpui_kit::Bounds<Pixels>, window: Pixels, label: &str) {
    assert!(
        bounds.left() >= px(0.) && bounds.right() <= window + px(1.),
        "{label} overflows horizontally: {bounds:?}"
    );
}

fn reveal(window: &mut Window, scroller: &'static str, id: impl Into<ElementId>, cx: &mut App) {
    let id = id.into();
    for _ in 0..80 {
        window.render_frame(cx);
        let item = window.find(id.clone());
        let bounds = item.bounds();
        let viewport = window.find(scroller).bounds();
        assert!(
            bounds.left() >= viewport.left() - px(1.)
                && bounds.right() <= viewport.right() + px(1.),
            "{id:?} overflows {scroller}: {bounds:?} vs {viewport:?}"
        );
        let contained = bounds.top() >= viewport.top() - px(1.)
            && bounds.bottom() <= viewport.bottom() + px(1.);
        let taller = bounds.size.height > viewport.size.height;
        let on_screen = bounds.top() < viewport.bottom() && bounds.bottom() > viewport.top();
        if item.visible() && (contained || (taller && on_screen)) {
            return;
        }
        let delta = if bounds.top() < viewport.top() {
            viewport.top() - bounds.top() + px(1.)
        } else {
            viewport.bottom() - bounds.bottom() - px(1.)
        };
        window.scroll(scroller, ScrollDelta::Pixels(point(px(0.), delta)), cx);
    }
    panic!("{id:?} must scroll into {scroller}");
}

fn assert_centred(
    column: gpui_kit::Bounds<Pixels>,
    container: gpui_kit::Bounds<Pixels>,
    label: &str,
) {
    let left = column.left() - container.left();
    let right = container.right() - column.right();
    assert!(
        (left - right).abs() < px(80.),
        "{label} is not centred: {left:?} vs {right:?}"
    );
}

struct KnowledgeSample {
    column: Pixels,
    canvas: Pixels,
    control: Pixels,
}

#[gpui_kit::test]
fn knowledge_sections_follow_the_pane(cx: &mut TestAppContext) {
    let mut narrow = None;
    let mut wide = None;
    let mut large_type = None;
    for (width, height, scale) in [(640., 420., 100u16), (2560., 1440., 100), (640., 420., 200)] {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                Theme::change(mode, None, cx);
            });
            let mut shell_entity = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| {
                    let mut shell = Shell::new(0, window, cx);
                    shell.select_tab(NavTab::Knowledge, cx);
                    shell
                });
                shell_entity = Some(shell.clone());
                Root::new(shell, window, cx)
            });
            let shell = shell_entity.unwrap();
            let load = request(&commands);
            assert!(matches!(load.action, MemoryAction::List(_)));
            respond(
                &events,
                load.request_id,
                MemoryReviewResult::Page(memory_page()),
            );
            settle(cx);
            let sample = cx
                .update_window(window.into(), |_, window, cx| {
                    frames(window, cx);
                    let window_px = px(width);
                    let page = window.find("sovra-knowledge-view").bounds();
                    inside_window(page, window_px, "knowledge");
                    assert!(page.top() >= px(0.) && page.bottom() <= px(height));
                    for id in [
                        "knowledge-memories",
                        "knowledge-people",
                        "knowledge-files",
                        "knowledge-help",
                    ] {
                        inside_window(window.find(id).bounds(), window_px, id);
                    }
                    let tokens = design::tokens(cx);
                    let rem = window.rem_size();
                    let page_cap = tokens.layout.page_width.to_pixels(rem);
                    let reading_cap = tokens.layout.reading_width.to_pixels(rem);
                    let column = window.find("knowledge-column").bounds();
                    assert!(
                        column.size.width <= page_cap + px(2.),
                        "memories column wider than the page cap: {column:?} > {page_cap:?}"
                    );
                    assert_centred(column, page, "memories");
                    reveal(window, "sovra-knowledge-view", "memory-search", cx);
                    reveal(window, "sovra-knowledge-view", "memory-graph-canvas", cx);
                    reveal(window, "sovra-knowledge-view", "memory-add", cx);
                    let canvas = window.find("memory-graph-canvas").bounds();
                    let browser = window.find("memory-browser").bounds();
                    let class = design::WidthClass::of(browser.size.width, window);
                    let inspector = window.find("memory-graph-inspector").bounds();
                    if class.fits_detail_pane() {
                        assert!(
                            inspector.left() + px(1.) >= canvas.right(),
                            "detail stays beside the picture: {inspector:?} {canvas:?}"
                        );
                    } else {
                        assert!(
                            inspector.top() + px(1.) >= canvas.bottom(),
                            "detail stacks under the picture: {inspector:?} {canvas:?}"
                        );
                    }
                    let expected = if class.fits_detail_pane() {
                        tokens.graph.height
                    } else {
                        tokens.graph.compact_height
                    };
                    let expected = expected.to_pixels(rem);
                    assert!(
                        (canvas.size.height - expected).abs() < px(2.),
                        "picture height {canvas:?} != {expected:?} for {class:?}"
                    );
                    assert!(canvas.left() >= column.left() - px(1.));
                    assert!(canvas.right() <= column.right() + px(1.));
                    let slack = tokens.space.medium.to_pixels(rem) * 8.;
                    if class.fits_detail_pane() {
                        assert!(
                            column.size.width - canvas.size.width > slack,
                            "picture should leave room for the detail column"
                        );
                    } else {
                        assert!(
                            column.size.width - canvas.size.width < slack,
                            "picture should track the column: {canvas:?} vs {column:?}"
                        );
                    }
                    let window_class = design::WidthClass::of_window(window);
                    if window_class.fits_side_pane() {
                        let floor = tokens.layout.content_min_width.to_pixels(rem);
                        assert!(
                            page.size.width >= floor,
                            "knowledge pane squeezed: {page:?} < {floor:?}"
                        );
                    } else {
                        assert!(window.try_find("sovra-inspector").is_none());
                    }
                    if scale == 200 {
                        assert!((rem - px(32.)).abs() < px(0.5));
                    }

                    window.click("knowledge-people", cx);
                    frames(window, cx);
                    let people = window.find("knowledge-column").bounds();
                    assert!(people.size.width <= reading_cap + px(2.));
                    assert_centred(
                        people,
                        window.find("sovra-knowledge-view").bounds(),
                        "people",
                    );
                    // With no one synced yet the page explains how people
                    // arrive; the message box only appears in an opened row.
                    reveal(window, "sovra-knowledge-view", "people-empty", cx);
                    inside_window(
                        window.find("people-empty").bounds(),
                        window_px,
                        "people empty note",
                    );

                    window.click("knowledge-files", cx);
                    frames(window, cx);
                    let files = window.find("knowledge-column").bounds();
                    assert!(files.size.width <= reading_cap + px(2.));
                    assert_centred(files, window.find("sovra-knowledge-view").bounds(), "files");
                    reveal(window, "sovra-knowledge-view", "index-folder", cx);

                    window.click("knowledge-help", cx);
                    frames(window, cx);
                    let guide = window.find("knowledge-column").bounds();
                    assert!(guide.size.width <= reading_cap + px(2.));
                    assert_centred(guide, window.find("sovra-knowledge-view").bounds(), "guide");
                    reveal(window, "sovra-knowledge-view", "knowledge-times-toggle", cx);
                    window.click("knowledge-times-toggle", cx);
                    frames(window, cx);
                    reveal(window, "sovra-knowledge-view", "knowledge-times", cx);

                    shell.update(cx, |shell, cx| {
                        shell.open_memory_source(
                            MemoryAction::InspectSource(EpisodeId("episode:school".into())),
                            cx,
                        );
                    });
                    KnowledgeSample {
                        column: column.size.width,
                        canvas: canvas.size.width,
                        control: window.find("knowledge-memories").bounds().size.height,
                    }
                })
                .unwrap();
            let inspect = take_memory(&commands)
                .into_iter()
                .find(|command| matches!(command.action, MemoryAction::InspectSource(_)))
                .expect("opening a source from chat inspects that record");
            respond(
                &events,
                inspect.request_id,
                MemoryReviewResult::Source(MemorySourceRecord {
                    episode: Episode {
                        id: EpisodeId("episode:school".into()),
                        session_id: "chat-school".into(),
                        kind: EpisodeKind::UserMessage,
                        content: "Sam mentioned mint tea after school. ".repeat(40),
                        occurred_at: Timestamp(2),
                    },
                    metadata: MemorySourceMetadata::default(),
                }),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                frames(window, cx);
                let window_px = px(width);
                let page = window.find("sovra-memory-source-view").bounds();
                inside_window(page, window_px, "source");
                assert!(page.bottom() <= px(height) + px(1.));
                let column = window.find("knowledge-column").bounds();
                let cap = design::tokens(cx)
                    .layout
                    .reading_width
                    .to_pixels(window.rem_size());
                assert!(column.size.width <= cap + px(2.));
                assert_centred(column, page, "source");
                reveal(window, "sovra-memory-source-view", "memory-back-chat", cx);
                reveal(window, "sovra-memory-source-view", "memory-source", cx);
                inside_window(window.find("memory-back-chat").bounds(), window_px, "back");
            })
            .unwrap();
            if mode == ThemeMode::Dark {
                match (width, scale) {
                    (640., 100) => narrow = Some(sample),
                    (2560., 100) => wide = Some(sample),
                    (640., 200) => large_type = Some(sample),
                    _ => {}
                }
            }
        }
    }
    let narrow = narrow.unwrap();
    let wide = wide.unwrap();
    let large_type = large_type.unwrap();
    assert!(
        narrow.column + px(32.) < wide.column,
        "narrow column should shrink below the wide one: {:?} vs {:?}",
        narrow.column,
        wide.column
    );
    assert!(
        narrow.canvas + px(32.) < wide.canvas,
        "the picture should widen with the column"
    );
    assert!(
        large_type.control > narrow.control * 1.6,
        "200% text should enlarge controls: {:?} vs {:?}",
        large_type.control,
        narrow.control
    );
}

struct PageFit {
    window_px: Pixels,
    height: f32,
    wide: bool,
}

struct CappedColumn {
    id: &'static str,
    cap: fn(&design::SovraTokens) -> gpui_kit::Rems,
}

fn check_page(
    window: &mut Window,
    cx: &mut App,
    fit: &PageFit,
    page: &'static str,
    control: &'static str,
    column: Option<CappedColumn>,
) {
    frames(window, cx);
    let page_bounds = window.find(page).bounds();
    inside_window(page_bounds, fit.window_px, page);
    assert!(page_bounds.top() >= px(0.) && page_bounds.bottom() <= px(fit.height) + px(1.));
    let control_bounds = window.find(control).bounds();
    inside_window(control_bounds, fit.window_px, control);
    if fit.wide {
        let Some(column) = column else {
            return;
        };
        let tokens = design::tokens(cx);
        let cap = (column.cap)(&tokens).to_pixels(window.rem_size());
        let column_bounds = window.find(column.id).bounds();
        assert!(
            column_bounds.size.width <= cap + px(2.),
            "{} is full-bleed: {column_bounds:?} > {cap:?}",
            column.id
        );
        assert_centred(column_bounds, page_bounds, column.id);
    }
}

#[gpui_kit::test]
fn workspace_pages_follow_the_pane(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100u16), (640., 420., 200), (2560., 1440., 100)] {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let (client, _commands, _events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                Theme::change(mode, None, cx);
            });
            let mut shell_entity = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| Shell::new(0, window, cx));
                shell_entity = Some(shell.clone());
                Root::new(shell, window, cx)
            });
            let shell = shell_entity.unwrap();
            let fit = PageFit {
                window_px: px(width),
                height,
                wide: width >= 2560.,
            };
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Chat, cx));
                frames(window, cx);
                let main = window.find("sovra-chat-main").bounds();
                inside_window(main, fit.window_px, "chat");
                let composer = window.find("chat-composer-input").bounds();
                inside_window(composer, fit.window_px, "composer");
                inside_window(
                    window.find("composer-send-btn").bounds(),
                    fit.window_px,
                    "send",
                );
                if fit.wide {
                    let cap = design::tokens(cx)
                        .chat
                        .column_width
                        .to_pixels(window.rem_size());
                    assert!(composer.size.width <= cap + px(2.));
                    assert_centred(composer, main, "chat");
                }

                shell.update(cx, |shell, cx| shell.select_tab(NavTab::MyAis, cx));
                check_page(
                    window,
                    cx,
                    &fit,
                    "sovra-my-ais-view",
                    "open-models-catalog-btn",
                    Some(CappedColumn {
                        id: "my-ais-column",
                        cap: |tokens| tokens.layout.page_width,
                    }),
                );
                inside_window(
                    window.find(("routing-mode-btn", 0usize)).bounds(),
                    fit.window_px,
                    "routing",
                );

                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Schedules, cx));
                check_page(
                    window,
                    cx,
                    &fit,
                    "sovra-schedules-view",
                    "schedule-create-toggle-btn",
                    Some(CappedColumn {
                        id: "schedules-column",
                        cap: |tokens| tokens.layout.page_width,
                    }),
                );

                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Skills, cx));
                check_page(
                    window,
                    cx,
                    &fit,
                    "sovra-skills-view",
                    "skill-create",
                    Some(CappedColumn {
                        id: "skills-column",
                        cap: |tokens| tokens.layout.page_width,
                    }),
                );

                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
                check_page(
                    window,
                    cx,
                    &fit,
                    "sovra-settings-view",
                    "settings-theme-light",
                    Some(CappedColumn {
                        id: "settings-column",
                        cap: |tokens| tokens.layout.reading_width,
                    }),
                );
                inside_window(
                    window.find("settings-text-larger").bounds(),
                    fit.window_px,
                    "text size",
                );

                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Connections, cx));
                check_page(
                    window,
                    cx,
                    &fit,
                    "connections-page",
                    "connections-refresh",
                    Some(CappedColumn {
                        id: "connections-column",
                        cap: |tokens| tokens.layout.page_width,
                    }),
                );

                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Models, cx));
                check_page(
                    window,
                    cx,
                    &fit,
                    "models-setup-home",
                    "models-browse-all",
                    Some(CappedColumn {
                        id: "models-home-column",
                        cap: |tokens| tokens.layout.reading_width,
                    }),
                );

                shell.update(cx, Shell::logout);
                check_page(
                    window,
                    cx,
                    &fit,
                    "sovra-login-screen",
                    "login-submit-btn",
                    Some(CappedColumn {
                        id: "login-glass-card",
                        cap: |tokens| tokens.layout.dialog_width,
                    }),
                );
                inside_window(
                    window.find("login-language").bounds(),
                    fit.window_px,
                    "language switch",
                );
            })
            .unwrap();
        }
    }
}
