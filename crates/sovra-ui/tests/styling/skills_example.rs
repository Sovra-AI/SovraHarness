//! A long Skills-page example stays inside its card.

#![allow(
    clippy::unwrap_used,
    reason = "headless UI fixtures fail loudly when the scripted runtime is gone"
)]

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Pixels, TestAppContext, Window, px, size};
use sovra_core::NavTab;
use sovra_core::models::InterfaceScale;
use sovra_core::skills::{Skill, SkillKind, builtin_skills};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

#[gpui_kit::test]
fn skill_examples_stay_inside_their_cards(cx: &mut TestAppContext) {
    let skills = builtin_skills();
    let long = skills
        .iter()
        .max_by_key(|skill| skill.friendly_card_copy().example_display.len())
        .unwrap();
    let short = skills
        .iter()
        .min_by_key(|skill| skill.friendly_card_copy().example_display.len())
        .unwrap();
    let procedure = skills
        .iter()
        .filter(|skill| matches!(skill.kind, SkillKind::Procedure))
        .max_by_key(|skill| skill.friendly_card_copy().example_display.len())
        .unwrap();
    let web_address = skills
        .iter()
        .find(|skill| {
            skill
                .friendly_card_copy()
                .example_display
                .contains("https://")
        })
        .unwrap();
    let greenhouse = skills
        .iter()
        .find(|skill| {
            skill
                .friendly_card_copy()
                .example_display
                .contains("greenhouse")
        })
        .unwrap();

    for (width, height, scale) in [(640., 420., 100u16), (2560., 1440., 100), (640., 420., 200)] {
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
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Skills, cx));
                for _ in 0..4 {
                    window.render_frame(cx);
                }
                let where_ = format!("{width}×{height} at {scale}% {mode:?}");
                let tip = window.find("skills-tip");
                let tip = tip.label().unwrap_or_default();
                assert!(tip.contains('/'), "skills tip dropped the shortcut: {tip}");
                assert!(window.try_find("skills-slash-pill-1").is_some());
                let long_bounds = assert_example_inside(window, long, &where_);
                let short_bounds = assert_example_inside(window, short, &where_);
                assert!(
                    long_bounds.size.height > short_bounds.size.height * 2.,
                    "the long example should wrap onto more lines than the short one at {where_}: {long_bounds:?} vs {short_bounds:?}"
                );
                let procedure_bounds = assert_example_inside(window, procedure, &where_);
                let procedure_index = card_index(window, procedure);
                let try_chat = window.find(("skill-use", procedure_index));
                let try_label = try_chat.label().unwrap_or_default();
                assert!(
                    try_label.contains("Try in chat"),
                    "procedure card should offer Try in chat at {where_}: {try_label}"
                );
                assert!(
                    procedure_bounds.bottom() <= try_chat.bounds().top() + px(1.),
                    "example covers Try in chat at {where_}"
                );
                let greenhouse_bounds = assert_example_inside(window, greenhouse, &where_);
                if width <= 640. {
                    let line = window.rem_size() * 0.75 * 1.5;
                    assert!(
                        greenhouse_bounds.size.height > line * 1.4,
                        "folder example stayed one line and runs past the card at {where_}: {greenhouse_bounds:?} line {line:?}"
                    );
                }
                let address_index = card_index(window, web_address);
                assert_example_inside(window, web_address, &where_);
                assert!(
                    window
                        .try_find(format!("skill-example-{address_index}-slash-pill-1"))
                        .is_none(),
                    "a web address in the example was drawn as a book pill at {where_}"
                );
                window.remove_window();
            })
            .unwrap();
        }
    }
}

fn assert_example_inside(window: &mut Window, skill: &Skill, where_: &str) -> Bounds<Pixels> {
    let copy = skill.friendly_card_copy();
    let index = card_index(window, skill);
    let card = window.find(("skill-card", index));
    let label = card.label().unwrap_or_default();
    assert!(
        label.contains(copy.example_display.as_str()),
        "card name dropped the example at {where_}: {label}"
    );
    let example = window.find(("skill-example", index)).bounds();
    let card_bounds = card.bounds();
    assert!(
        example.left() >= card_bounds.left() - px(1.)
            && example.right() <= card_bounds.right() + px(1.),
        "example runs past the card at {where_}: example {example:?} card {card_bounds:?}"
    );
    let action = window.find(("skill-use", index));
    let action_label = action.label().unwrap_or_default();
    assert!(
        action_label.contains("Try in chat") || action_label.contains("Open now"),
        "missing card action at {where_}: {action_label}"
    );
    assert!(
        example.bottom() <= action.bounds().top() + px(1.),
        "example covers {action_label} at {where_}: example {example:?} action {:?}",
        action.bounds()
    );
    if let Some(try_example) = window.try_find(("skill-try-example", index)) {
        let try_label = try_example.label().unwrap_or_default();
        assert!(
            try_label.contains("Try the example"),
            "unexpected example button at {where_}: {try_label}"
        );
        assert!(
            example.bottom() <= try_example.bounds().top() + px(1.),
            "example covers Try the example at {where_}: example {example:?} button {:?}",
            try_example.bounds()
        );
    }
    example
}

fn card_index(window: &Window, skill: &Skill) -> usize {
    let copy = skill.friendly_card_copy();
    (0..160)
        .find(|&index| {
            window.try_find(("skill-card", index)).is_some_and(|card| {
                card.label().is_some_and(|label| {
                    label.contains(copy.friendly_title.as_str())
                        && label.contains(copy.example_display.as_str())
                })
            })
        })
        .unwrap_or_else(|| panic!("missing card {}", copy.friendly_title))
}
