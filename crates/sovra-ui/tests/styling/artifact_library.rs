//! Folder filters and framework source remain within their pane at every scale.
use super::*;
use sovra_core::artifacts::{
    Artifact, ArtifactCommand, ArtifactDraft, ArtifactEvent, ArtifactKind, ArtifactSource,
};
use sovra_core::{Command, Event, NavTab, Timestamp};

fn reveal(
    window: &mut gpui_kit::Window,
    id: impl Into<gpui_kit::ElementId> + Clone + std::fmt::Debug,
    cx: &mut gpui_kit::App,
) {
    let id = id.into();
    for _ in 0..40 {
        window.render_frame(cx);
        let page = window.find("sovra-artifacts-view").bounds();
        let item = window.find(id.clone()).bounds();
        assert!(
            item.left() >= page.left() && item.right() <= page.right(),
            "{id:?} {item:?} exceeds {page:?}"
        );
        if item.top() >= page.top() && item.bottom() <= page.bottom() {
            return;
        }
        let delta = if item.top() < page.top() {
            page.top() - item.top()
        } else {
            page.bottom() - item.bottom()
        };
        window.scroll(
            "sovra-artifacts-view",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), delta)),
            cx,
        );
    }
    panic!("{id:?} must be reachable by scrolling");
}

/// Horizontally inside the artifacts page. Tall blocks may extend below the fold.
fn within_page(window: &gpui_kit::Window, id: &'static str) {
    let page = window.find("sovra-artifacts-view").bounds();
    let bounds = window.find(id).bounds();
    assert!(
        bounds.left() >= page.left() && bounds.right() <= page.right(),
        "{id} {bounds:?} exceeds {page:?}"
    );
}

/// The control is on the first view: fully inside the page width, and its box meets the page.
fn on_first_view(window: &gpui_kit::Window, id: impl Into<gpui_kit::ElementId> + Clone) {
    let id = id.into();
    let page = window.find("sovra-artifacts-view").bounds();
    let item = window.find(id.clone()).bounds();
    assert!(
        item.left() >= page.left()
            && item.right() <= page.right()
            && item.bottom() > page.top()
            && item.top() < page.bottom(),
        "{id:?} {item:?} misses the first view {page:?}"
    );
}

#[gpui_kit::test]
fn organized_library_and_project_fit_both_themes(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let (client, _commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(
                    sovra_core::models::InterfaceScale::from_percent(scale),
                    cx,
                );
            });
            let mut shell = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| Shell::new(0, window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.unwrap();
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
                window.render_frame(cx);
                within_page(window, "artifacts-empty");
                within_page(window, "artifacts-open-chat");
                reveal(window, "artifacts-open-chat", cx);
                window.click("artifacts-open-chat", cx);
                assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Chat);
            })
            .unwrap();
            let content = serde_json::json!({"files":{"package.json":"{\"scripts\":{\"build\":\"vite build\"}}","src/App.jsx":"export default () => <h1>Garden</h1>"}}).to_string();
            let mut artifact = Artifact::from_draft(
                "project".into(),
                ArtifactDraft {
                    title: "Garden project".into(),
                    kind: ArtifactKind::Project,
                    language: "website-project".into(),
                    content,
                    source: ArtifactSource::default(),
                },
                Timestamp(1),
            );
            artifact.collection = "Garden".into();
            events
                .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![artifact])))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
                window.render_frame(cx);
                let page = window.find("sovra-artifacts-view").bounds();
                assert!(window.try_find("artifact-page-next").is_none());
                for id in ["artifact-folders", "artifact-grid", "artifact-search"] {
                    let bounds = window.find(id).bounds();
                    assert!(
                        bounds.left() >= page.left() && bounds.right() <= page.right(),
                        "{id} {bounds:?} exceeds {page:?}"
                    );
                }
                reveal(window, "artifact-favorites", cx);
                window.click("artifact-favorites", cx);
                window.render_frame(cx);
                within_page(window, "artifacts-no-results");
                reveal(window, "artifact-clear-filters", cx);
                window.click("artifact-clear-filters", cx);
                window.render_frame(cx);
                assert!(window.try_find("artifact-grid").is_some());
                assert!(window.try_find("artifact-clear-filters").is_none());
                shell.update(cx, |shell, cx| shell.open_artifact("project", cx));
                window.render_frame(cx);
                for id in [
                    "artifact-detail",
                    "artifact-title",
                    "artifact-build-project",
                    "artifact-project-source",
                    "artifact-move-folder",
                ] {
                    let bounds = window.find(id).bounds();
                    assert!(
                        bounds.left() >= page.left() && bounds.right() <= page.right(),
                        "{id} {bounds:?} exceeds {page:?}"
                    );
                }
                reveal(window, "artifact-edit", cx);
                window.click("artifact-edit", cx);
                reveal(window, "artifact-edit-title", cx);
                window.click("artifact-edit-title", cx);
                window.input(" draft", cx);
                reveal(window, "artifact-edit-cancel", cx);
                window.click("artifact-edit-cancel", cx);
                for id in ["artifact-keep-editing", "artifact-discard-changes"] {
                    reveal(window, id, cx);
                }
                window.click("artifact-discard-changes", cx);
                assert!(window.try_find("artifact-unsaved-choice").is_none());
                assert!(window.try_find("artifact-edit-form").is_none());
                reveal(window, "artifact-edit", cx);
                window.click("artifact-edit", cx);
                reveal(window, "artifact-edit-save", cx);
                window.click("artifact-edit-save", cx);
            })
            .unwrap();
            events
                .send_blocking(Event::Artifacts(ArtifactEvent::Error(
                    "The test folder is unavailable.".into(),
                )))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "artifact-save-error", cx);
                reveal(window, "artifact-edit-save", cx);
                reveal(window, "artifact-edit-cancel", cx);
                assert!(window.find("artifact-edit-form").bounds().size.height > px(0.));
            })
            .unwrap();
        }
    }
}

fn drain(commands: &async_channel::Receiver<Command>) -> Vec<Command> {
    std::iter::from_fn(|| commands.try_recv().ok()).collect()
}

fn garden_note() -> Artifact {
    Artifact::from_draft(
        "garden".into(),
        ArtifactDraft {
            title: "Garden note".into(),
            kind: ArtifactKind::Document,
            language: String::new(),
            content: "Tomatoes go in the sunny bed.".into(),
            source: ArtifactSource::default(),
        },
        Timestamp(1),
    )
}

/// One short saved piece, with no group-shared section, leads the minimum window.
#[gpui_kit::test]
fn saved_work_leads_the_minimum_window(cx: &mut TestAppContext) {
    let writing = "Tomatoes go in the sunny bed.";
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        let (client, commands, events) = Client::loopback();
        cx.update(|cx| {
            sovra_ui::init(client, cx);
            Theme::change(mode, None, cx);
            design::set_interface_scale(sovra_core::models::InterfaceScale::from_percent(100), cx);
        });
        let mut shell = None;
        let window = cx.open_window(size(px(640.), px(420.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        events
            .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![garden_note()])))
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
            window.render_frame(cx);
            assert!(
                window.try_find("shared-artifacts").is_none(),
                "this library has no group-shared section"
            );
            on_first_view(window, ("artifact-card", 0usize));
            for id in [
                "artifact-search",
                "artifact-folders",
                "artifact-folder-all",
                "artifact-favorites",
            ] {
                on_first_view(window, id);
            }
            on_first_view(window, ("artifact-kind", 0usize));
            on_first_view(window, ("artifact-sort", 0usize));
            window.click("artifact-favorites", cx);
            window.render_frame(cx);
            assert!(
                window.find_all(("artifact-card", 0usize)).is_empty(),
                "favorites hides the unpinned note"
            );
            assert!(window.try_find("artifact-grid").is_none());
            on_first_view(window, "artifact-search");
            reveal(window, "artifact-clear-filters", cx);
            window.click("artifact-clear-filters", cx);
            window.render_frame(cx);
            assert!(window.try_find("artifact-clear-filters").is_none());
            reveal(window, "artifact-search", cx);
            on_first_view(window, "artifact-grid");
            on_first_view(window, ("artifact-card", 0usize));
            reveal(window, ("artifact-open", 0usize), cx);
            window.click(("artifact-open", 0usize), cx);
            window.render_frame(cx);
            on_first_view(window, "artifact-title");
            on_first_view(window, "artifact-preview");
            assert_eq!(window.find("artifact-title").label(), Some("Garden note"));
            let preview = window
                .find("artifact-preview")
                .label()
                .map(str::to_owned)
                .unwrap_or_default();
            assert!(
                preview.contains(writing),
                "the writing stays source text, got {preview:?}"
            );
            assert!(
                window.try_find("artifact-open-browser").is_none(),
                "a document is not run as a page"
            );
            for id in [
                "artifact-back",
                "artifact-copy",
                "artifact-save-copy",
                "artifact-edit",
                "artifact-delete",
            ] {
                reveal(window, id, cx);
            }
            assert_eq!(window.find("artifact-delete").label(), Some("Delete"));
            window.click("artifact-delete", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("artifact-delete").label(),
                Some("Click again to delete")
            );
            assert_eq!(
                window
                    .find("artifact-preview")
                    .label()
                    .map(str::to_owned)
                    .as_deref(),
                Some(writing)
            );
            assert_eq!(window.find("artifact-title").label(), Some("Garden note"));
            reveal(window, "artifact-favorite", cx);
            window.click("artifact-favorite", cx);
        })
        .unwrap();
        let sent = drain(&commands);
        assert!(
            !sent.iter().any(|command| matches!(
                command,
                Command::Artifacts(ArtifactCommand::Delete { .. })
            )),
            "the first delete only asks again: {sent:?}"
        );
        assert!(
            sent.iter().any(|command| matches!(
                command,
                Command::Artifacts(ArtifactCommand::Organize {
                    id,
                    pinned: true,
                    ..
                }) if id == "garden"
            )),
            "favorite still updates the piece: {sent:?}"
        );
    }
}

fn bed_program() -> Artifact {
    Artifact::from_draft(
        "bed".into(),
        ArtifactDraft {
            title: "Sunny bed".into(),
            kind: ArtifactKind::Code,
            language: "rust".into(),
            content: "fn main() { let bed = \"sunny\"; }\n\n// full preview".into(),
            source: ArtifactSource::default(),
        },
        Timestamp(1),
    )
}

fn theme_role(theme: &gpui_kit::component::highlighter::HighlightTheme, name: &str) -> Hsla {
    theme
        .style(name)
        .and_then(|style| style.color)
        .unwrap_or_else(|| panic!("{name} has a colour in the code theme"))
}

/// The colour the saved-work highlighter gave the whole needle.
fn colour_covering(sample: &sovra_ui::SavedWorkHighlight, needle: &str) -> Hsla {
    let start = sample
        .code
        .find(needle)
        .unwrap_or_else(|| panic!("highlighter saw {:?}, which has no {needle}", sample.code));
    let end = start + needle.len();
    sample
        .styles
        .iter()
        .find(|(range, style)| range.start <= start && range.end >= end && style.color.is_some())
        .and_then(|(_, style)| style.color)
        .unwrap_or_else(|| panic!("{needle} is not one coloured run in {:?}", sample.styles))
}

fn assert_rust_colours(
    sample: &sovra_ui::SavedWorkHighlight,
    theme: &gpui_kit::component::highlighter::HighlightTheme,
    place: &str,
) {
    assert_eq!(
        sample.language, "rust",
        "{place} must carry the rust fence so colouring runs, got {:?}",
        sample.language
    );
    assert!(
        sample.code.contains("fn main") && sample.code.contains("\"sunny\""),
        "{place} coloured something else: {:?}",
        sample.code
    );
    let keyword = colour_covering(sample, "fn");
    let string = colour_covering(sample, "\"sunny\"");
    assert_ne!(
        keyword, string,
        "{place} paints the keyword and the quoted string with one colour"
    );
    assert_eq!(keyword, theme_role(theme, "keyword"), "{place} keyword");
    assert_eq!(string, theme_role(theme, "string"), "{place} string");
}

/// Known-language source is coloured on the card and the open piece.
/// A letter stays ordinary writing. The short library still leads the window.
#[gpui_kit::test]
fn coloured_rust_leads_the_minimum_window(cx: &mut TestAppContext) {
    let program = "fn main() { let bed = \"sunny\"; }";
    let dark = gpui_kit::component::highlighter::HighlightTheme::default_dark();
    let light = gpui_kit::component::highlighter::HighlightTheme::default_light();
    assert_ne!(
        theme_role(dark.as_ref(), "keyword"),
        theme_role(light.as_ref(), "keyword"),
        "dark mode must use the dark code colours"
    );
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        let theme = if mode == ThemeMode::Dark {
            dark.clone()
        } else {
            light.clone()
        };
        let (client, commands, events) = Client::loopback();
        cx.update(|cx| {
            sovra_ui::init(client, cx);
            Theme::change(mode, None, cx);
            design::set_interface_scale(sovra_core::models::InterfaceScale::from_percent(100), cx);
        });
        let mut shell = None;
        let window = cx.open_window(size(px(640.), px(420.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        events
            .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![bed_program()])))
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
            window.render_frame(cx);
            let card = sovra_ui::take_saved_work_highlight()
                .expect("the library card colours a rust program");
            assert_rust_colours(&card, theme.as_ref(), "library card");
            assert!(!card.code.contains("\n\n"));
            assert!(window.try_find("shared-artifacts").is_none());
            on_first_view(window, ("artifact-card", 0usize));
            for id in [
                "artifact-search",
                "artifact-folders",
                "artifact-folder-all",
                "artifact-favorites",
            ] {
                on_first_view(window, id);
            }
            let code_kind = sovra_core::artifacts::KINDS
                .iter()
                .position(|info| info.kind == ArtifactKind::Code)
                .unwrap();
            on_first_view(window, ("artifact-kind", code_kind));
            on_first_view(window, ("artifact-sort", 0usize));
            window.click("artifact-favorites", cx);
            window.render_frame(cx);
            assert!(
                window.find_all(("artifact-card", 0usize)).is_empty(),
                "favorites hides the unpinned program"
            );
            on_first_view(window, "artifact-search");
            reveal(window, "artifact-clear-filters", cx);
            window.click("artifact-clear-filters", cx);
            window.render_frame(cx);
            on_first_view(window, ("artifact-card", 0usize));
            reveal(window, ("artifact-open", 0usize), cx);
            window.click(("artifact-open", 0usize), cx);
            // The click paints the library before it opens the piece. Drop that
            // card sample so the next frames have to be the open preview.
            let _ = sovra_ui::take_saved_work_highlight();
            let mut preview = None;
            for _ in 0..3 {
                window.render_frame(cx);
                if let Some(sample) = sovra_ui::take_saved_work_highlight() {
                    preview = Some(sample);
                    break;
                }
            }
            let preview = preview.unwrap_or_else(|| {
                panic!(
                    "opening the program runs the rust highlighter (title {:?}, preview {:?}, calls {})",
                    window
                        .try_find("artifact-title")
                        .map(|item| item.label().map(str::to_owned)),
                    window
                        .try_find("artifact-preview")
                        .map(|item| item.label().map(str::to_owned)),
                    sovra_ui::saved_work_highlight_calls()
                )
            });
            assert_rust_colours(&preview, theme.as_ref(), "open piece");
            assert!(
                preview.code.contains("\n\n// full preview"),
                "full source preserves the blank line omitted by the card"
            );
            on_first_view(window, "artifact-title");
            on_first_view(window, "artifact-preview");
            assert_eq!(window.find("artifact-title").label(), Some("Sunny bed"));
            let shown = window
                .find("artifact-preview")
                .label()
                .map(str::to_owned)
                .unwrap_or_default();
            assert!(
                shown.contains(program),
                "the open piece shows the program, got {shown:?}"
            );
            for id in [
                "artifact-back",
                "artifact-copy",
                "artifact-save-copy",
                "artifact-edit",
                "artifact-delete",
            ] {
                reveal(window, id, cx);
            }
            assert_eq!(window.find("artifact-delete").label(), Some("Delete"));
            window.click("artifact-delete", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("artifact-delete").label(),
                Some("Click again to delete")
            );
            assert_eq!(window.find("artifact-title").label(), Some("Sunny bed"));
            window.remove_window();
        })
        .unwrap();
        let sent = drain(&commands);
        assert!(
            !sent.iter().any(|command| matches!(
                command,
                Command::Artifacts(ArtifactCommand::Delete { .. })
            )),
            "the first delete only asks again: {sent:?}"
        );

        let _ = sovra_ui::take_saved_work_highlight();
        let (client, _commands, events) = Client::loopback();
        cx.update(|cx| {
            sovra_ui::init(client, cx);
            Theme::change(mode, None, cx);
            design::set_interface_scale(sovra_core::models::InterfaceScale::from_percent(100), cx);
        });
        let mut shell = None;
        let window = cx.open_window(size(px(640.), px(420.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        let writing = "Tomatoes go in the sunny bed.";
        events
            .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![garden_note()])))
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
            window.render_frame(cx);
            assert!(
                sovra_ui::take_saved_work_highlight().is_none(),
                "a letter on a card is ordinary writing"
            );
            reveal(window, ("artifact-open", 0usize), cx);
            window.click(("artifact-open", 0usize), cx);
            let _ = sovra_ui::take_saved_work_highlight();
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(
                sovra_ui::take_saved_work_highlight().is_none(),
                "opening a letter does not fence it as code"
            );
            let shown = window
                .find("artifact-preview")
                .label()
                .map(str::to_owned)
                .unwrap_or_default();
            assert_eq!(shown, writing);
            assert!(window.try_find("artifact-open-browser").is_none());
            window.remove_window();
        })
        .unwrap();
    }
}
