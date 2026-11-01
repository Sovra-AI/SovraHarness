//! Knowledge tab: the explanation is on screen, and the chat transcript is not.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::{NavTab, memory_explanation};
use sovra_runtime::Client;
use sovra_ui::Shell;

#[gpui_kit::test]
fn knowledge_opens_to_personal_details_and_keeps_files_and_help_reachable(cx: &mut TestAppContext) {
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        Root::new(shell, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.click("nav-knowledge", cx);
        window.render_frame(cx);
        assert!(window.try_find("file-index-panel").is_none());
        assert!(window.find("conversation-memory-status").visible());
        window.click("knowledge-memories", cx);
        window.render_frame(cx);
        assert!(window.find("memory-browser").visible());
        assert!(window.try_find("file-index-panel").is_none());
        window.click("knowledge-help", cx);
        window.render_frame(cx);
        assert!(window.find("knowledge-intro").visible());
        assert!(window.try_find("memory-browser").is_none());
        window.click("knowledge-files", cx);
        window.render_frame(cx);
        assert!(window.find("index-folder").visible());
        assert!(window.try_find("knowledge-intro").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn knowledge_sections_stay_reachable_at_minimum_wide_and_double_text_sizes(
    cx: &mut TestAppContext,
) {
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::{ScrollDelta, point};
    use sovra_core::models::InterfaceScale;
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
        let (client, _commands, _events) = Client::loopback();
        cx.update(|cx| {
            sovra_ui::init(client, cx);
            sovra_ui::design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
        });
        let window = cx.open_window(size(px(width), px(height)), |window, cx| {
            let shell = cx.new(|cx| {
                let mut shell = Shell::new(0, window, cx);
                shell.select_tab(NavTab::Knowledge, cx);
                shell
            });
            Root::new(shell, window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            for mode in [ThemeMode::Dark, ThemeMode::Light] {
                Theme::change(mode, Some(window), cx);
                window.render_frame(cx);
                for (tab, content) in [
                    ("knowledge-files", "file-index-panel"),
                    ("knowledge-memories", "conversation-memory-status"),
                    ("knowledge-help", "knowledge-intro"),
                ] {
                    window.click(tab, cx);
                    window.render_frame(cx);
                    let bounds = window.find(content).bounds();
                    assert!(
                        bounds.left() >= px(0.) && bounds.right() <= px(width),
                        "{content}@{scale}: {bounds:?}"
                    );
                    assert!(window.find(content).visible());
                    window.scroll(
                        "sovra-knowledge-view",
                        ScrollDelta::Pixels(point(px(0.), px(-2000.))),
                        cx,
                    );
                    window.render_frame(cx);
                    for id in ["knowledge-files", "knowledge-memories", "knowledge-help"] {
                        let control = window.find(id);
                        assert!(control.visible());
                        assert!(
                            control.bounds().left() >= px(0.)
                                && control.bounds().right() <= px(width)
                        );
                        assert!(control.bounds().bottom() < px(height));
                    }
                }
                window.click("knowledge-files", cx);
                window.render_frame(cx);
                let add = window.find("index-folder");
                assert!(add.visible());
                assert!(
                    add.bounds().bottom() <= px(height),
                    "Add a folder at {scale}%: {:?}",
                    add.bounds()
                );
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn memory_search_consent_discloses_destination_and_fits_each_layout(cx: &mut TestAppContext) {
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::{ScrollDelta, point};
    use sovra_core::{
        Command, Event, MemoryAction, MemoryEvent, MemoryReviewEvent, MemoryReviewResult, models::*,
    };
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (1280., 820., 200)] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                sovra_ui::design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            let mut shell = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| Shell::new(0, window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.unwrap();
            let mut connection = ModelConnection::new("search", ProviderKind::OpenAiCompatible);
            connection.base_url = "https://embedding.example/v1".into();
            connection.manual_models.push(ModelInfo {
                id: "embed-v1".into(),
                output_modalities: vec!["embeddings".into()],
                ..Default::default()
            });
            let target =
                ModelDestination::for_embeddings(&connection, &connection.manual_models[0])
                    .unwrap();
            events
                .send_blocking(Event::Models(ModelEvent::Loaded {
                    connections: vec![connection],
                    preferences: ModelPreferences::default(),
                }))
                .unwrap();
            cx.update(|cx| shell.update(cx, |shell, cx| shell.select_tab(NavTab::Knowledge, cx)));
            while let Ok(command) = commands.try_recv() {
                if let Command::Memory(command) = command {
                    assert!(matches!(command.action, MemoryAction::List(_)));
                    events
                        .send_blocking(Event::Memory(MemoryEvent::Review(MemoryReviewEvent {
                            request_id: command.request_id,
                            result: Ok(MemoryReviewResult::Page(sovra_core::MemoryPage::default())),
                        })))
                        .unwrap();
                }
            }
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.click("knowledge-memories", cx);
                window.render_frame(cx);
                for id in [gpui_kit::ElementId::from("memory-choose-semantic"), ("memory-enable-semantic", 0usize).into()] {
                    for _ in 0..100 {
                        let choice = window.find(id.clone());
                        if choice.visible() && choice.bounds().top() > px(60.) && choice.bounds().bottom() < px(height - 60.) { break; }
                        let dy = if choice.bounds().top() <= px(60.) { 80. } else { -80. };
                        window.scroll("sovra-knowledge-view", ScrollDelta::Pixels(point(px(0.), px(dy))), cx);
                    }
                    let choice = window.find(id.clone());
                    assert!(choice.visible(), "{width}@{scale}: {choice:?}");
                    let page = window.find("memory-browser").bounds();
                    assert!(choice.bounds().right() <= page.right());
                    if id == gpui_kit::ElementId::from(("memory-enable-semantic", 0usize)) {
                        let destination = window.find("memory-semantic-destination-0");
                        assert_eq!(destination.label(), Some(target.base_url.as_str()));
                        assert!(destination.bounds().right() <= page.right());
                        assert!(!std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Memory(command) if matches!(command.action, MemoryAction::ConfigureSemantic(_)))));
                    }
                    window.click(id, cx);
                    window.render_frame(cx);
                }
                let command = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|command| match command { Command::Memory(command) => Some(command), _ => None }).unwrap();
                assert_eq!(command.action, MemoryAction::ConfigureSemantic(Some(target.clone())));
            }).unwrap();
        }
    }
}

#[gpui_kit::test]
fn knowledge_tab_shows_the_memory_story_not_the_chat(cx: &mut TestAppContext) {
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));

    let mut shell_entity = None;
    // Tall enough that the whole example, including the times, is on screen.
    let window = cx.open_window(size(px(1280.), px(2400.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell_entity = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell_entity.unwrap();
    let story = memory_explanation();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-knowledge", cx);
        window.render_frame(cx);
        window.click("knowledge-help", cx);
        window.render_frame(cx);

        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Knowledge);
        assert!(window.find("sovra-knowledge-view").visible());
        assert!(window.try_find("sovra-chat-main").is_none());
        assert!(window.try_find("chat-composer-input").is_none());
        assert!(window.try_find("chat-scroll-area").is_none());
        assert_eq!(window.find("knowledge-example-notice").label(),
            Some("Example only — these people and details are not your saved memories. Open About you to see what Sovra has actually saved."));

        let mut story_text = String::new();
        for (index, step) in story.steps.iter().enumerate() {
            let snapshot = window.find(("knowledge-step", index));
            assert!(snapshot.visible(), "step {index} is not visible");
            let label = snapshot.label().unwrap_or("");
            assert!(
                label.contains(step.heading),
                "step {index} label missing heading: {label}"
            );
            assert!(
                label.contains(step.body),
                "step {index} label missing body: {label}"
            );
            story_text.push_str(label);
            story_text.push(' ');
        }
        let story_text = story_text.to_ascii_lowercase();
        assert!(
            story_text.contains("record"),
            "lifecycle text missing record: {story_text}"
        );
        assert!(
            story_text.contains("current belief"),
            "lifecycle text missing current belief: {story_text}"
        );
        assert!(
            story_text.contains("replacement"),
            "lifecycle text missing replacement: {story_text}"
        );
        assert!(
            story_text.contains("history kept"),
            "lifecycle text missing history kept: {story_text}"
        );

        let about_snapshot = window.find("knowledge-about");
        let about = about_snapshot.label().unwrap_or("").to_owned();
        for belief in story.beliefs {
            for name in belief.about {
                assert!(about.contains(name), "about text missing {name}: {about}");
            }
        }
        let search_snapshot = window.find("knowledge-search");
        let search = search_snapshot.label().unwrap_or("").to_ascii_lowercase();
        assert!(search.contains("meaning"), "{search}");
        assert!(search.contains("word"), "{search}");

        assert!(window.try_find("knowledge-times").is_none());
        window.click("knowledge-times-toggle", cx);
        window.render_frame(cx);
        let times = window.find("knowledge-times");
        assert!(times.visible());
        let times_label = times.label().unwrap_or("");
        for note in story.times {
            assert!(
                times_label.contains(note.heading),
                "times missing {}: {times_label}",
                note.heading
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn semantic_folder_choice_discloses_the_destination_and_fits_each_layout(cx: &mut TestAppContext) {
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::{ScrollDelta, point};
    use sovra_core::{Command, Event, file_index::*, models::*};
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (1280., 820., 200)] {
        let (client, commands, events) = Client::loopback();
        cx.update(|cx| sovra_ui::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(width), px(height)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        let mut connection = ModelConnection::new("local", ProviderKind::Ollama);
        connection.manual_models.push(ModelInfo {
            id: "embed-v1".into(),
            output_modalities: vec!["embeddings".into()],
            ..ModelInfo::default()
        });
        let target =
            semantic::EmbeddingTarget::for_embeddings(&connection, &connection.manual_models[0])
                .unwrap();
        events
            .send_blocking(Event::Models(ModelEvent::Loaded {
                connections: vec![connection],
                preferences: ModelPreferences::default(),
            }))
            .unwrap();
        events
            .send_blocking(Event::FileIndex(IndexEvent::Roots(vec![IndexedRoot {
                id: "folder".into(),
                path: "/chosen/documents".into(),
                status: IndexStatus::Ready,
                progress: IndexProgress::default(),
                issues: vec![],
                completed_at: Some(1),
                auto_refresh: false,
                scan_options: ScanOptions::default(),
            }])))
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            cx.update(|cx| {
                Theme::change(mode, None, cx);
                sovra_ui::design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Knowledge, cx));
                window.render_frame(cx);
                window.scroll(
                    "sovra-knowledge-view",
                    ScrollDelta::Pixels(point(px(0.), px(5000.))),
                    cx,
                );
                window.click("knowledge-files", cx);
                window.render_frame(cx);
                let choices = if window.try_find("choose-semantic-folder").is_some() {
                    vec!["choose-semantic-folder"]
                } else {
                    vec!["index-details-folder", "choose-semantic-folder"]
                };
                for id in choices {
                    for _ in 0..80 {
                        window.render_frame(cx);
                        let choice = window.find(id);
                        let viewport = window.find("sovra-knowledge-view").bounds();
                        if choice.visible()
                            && choice.bounds().bottom() < viewport.bottom()
                            && choice.bounds().top() > viewport.top()
                        {
                            break;
                        }
                        let dy = if choice.bounds().top() <= viewport.top() {
                            60.
                        } else {
                            -60.
                        };
                        window.scroll(
                            "sovra-knowledge-view",
                            ScrollDelta::Pixels(point(px(0.), px(dy))),
                            cx,
                        );
                    }
                    window.click(id, cx);
                }
                window.render_frame(cx);
                assert!(window.try_find("semantic-chooser").is_some());
                while let Ok(command) = commands.try_recv() {
                    assert!(!matches!(
                        command,
                        Command::FileIndex(IndexCommand::ConfigureSemantic { .. })
                    ));
                }
                for _ in 0..40 {
                    let choice = window.find(("enable-semantic", 0usize));
                    if choice.visible()
                        && choice.bounds().bottom() < px(height - 60.)
                        && choice.bounds().top() > px(60.)
                    {
                        break;
                    }
                    window.scroll(
                        "sovra-knowledge-view",
                        ScrollDelta::Pixels(point(px(0.), px(-80.))),
                        cx,
                    );
                }
                let choice = window.find(("enable-semantic", 0usize));
                assert!(choice.visible(), "{width}@{scale}: {choice:?}");
                let page = window.find("file-index-panel").bounds();
                assert!(choice.bounds().right() <= page.right());
                let destination = window.find("semantic-destination-0");
                assert_eq!(destination.label(), Some(target.base_url.as_str()));
                assert!(destination.bounds().right() <= page.right());
                window.click(("enable-semantic", 0usize), cx);
                let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
                assert!(
                    sent.contains(&Command::FileIndex(IndexCommand::ConfigureSemantic {
                        root_id: "folder".into(),
                        target: Some(target.clone())
                    }))
                );
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn live_memory_status_and_service_disclosure_fit_all_layouts(cx: &mut TestAppContext) {
    use gpui_kit::component::{Theme, ThemeMode};
    use sovra_core::{Event, MemoryEvent, MemoryStatus, models::InterfaceScale};
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (1280., 820., 200)] {
        let (client, _, events) = Client::loopback();
        cx.update(|cx| sovra_ui::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(width), px(height)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            cx.update(|cx| {
                Theme::change(mode, None, cx);
                sovra_ui::design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            for status in [
                MemoryStatus::Working,
                MemoryStatus::Paused,
                MemoryStatus::Failed("Remembering could not save this result.".into()),
            ] {
                events
                    .send_blocking(Event::Memory(MemoryEvent::Status(status.clone())))
                    .unwrap();
                cx.run_until_parked();
                cx.update_window(window.into(), |_, window, cx| {
                    shell.update(cx, |shell, cx| shell.select_tab(NavTab::Knowledge, cx));
                    window.render_frame(cx);
                    window.click("knowledge-memories", cx);
                    window.render_frame(cx);
                    let card = window.find("conversation-memory-status");
                    assert!(card.visible());
                    assert!(card.label().unwrap().contains(status.label()));
                    assert!(card.label().unwrap().contains("selected AI service"));
                    assert!(card.bounds().right() <= px(width));
                    assert_eq!(shell.read(cx).workspace().memory_status, status);
                })
                .unwrap();
            }
        }
    }
}
