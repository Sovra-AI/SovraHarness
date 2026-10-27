//! Knowledge tab: the explanation is on screen, and the chat transcript is not.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::{NavTab, memory_explanation};
use sovra_runtime::Client;
use sovra_ui::Shell;

#[gpui_kit::test]
fn knowledge_tab_shows_the_memory_story_not_the_chat(cx: &mut TestAppContext) {
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));

    let mut shell_entity = None;
    // Tall enough that the whole example, including the times, is on screen.
    let window = cx.open_window(size(px(1280.), px(2400.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(window, cx));
        shell_entity = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell_entity.unwrap();
    let story = memory_explanation();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-knowledge", cx);
        window.render_frame(cx);

        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Knowledge);
        assert!(window.find("sovra-knowledge-view").visible());
        assert!(window.try_find("sovra-chat-main").is_none());
        assert!(window.try_find("chat-composer-input").is_none());
        assert!(window.try_find("chat-scroll-area").is_none());

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
            let view = cx.new(|cx| Shell::new(window, cx));
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
                for _ in 0..30 {
                    let choice = window.find("choose-semantic-folder");
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
                window.click("choose-semantic-folder", cx);
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
