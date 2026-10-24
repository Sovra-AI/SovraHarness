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
