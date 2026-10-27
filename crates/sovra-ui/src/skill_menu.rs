//! Native slash suggestions. Text editing stays with the GPUI input; only popup
//! navigation keys are captured while suggestions are visible.

use gpui_kit::component::{Selectable as _, button::ButtonVariants as _, v_flex};
use gpui_kit::{
    Context, Focusable as _, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, ScrollHandle, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, div,
};
use sovra_core::NavTab;
use sovra_core::skills::{Skill, SkillAction, SkillKind, slash_query};

use crate::design::{self, SovraStyled as _, TextRole};
use crate::shell::Shell;

#[derive(Debug, Default)]
pub(crate) struct MenuState {
    pub(crate) selected: usize,
    pub(crate) dismissed: bool,
    pub(crate) notice: Option<String>,
    scroll: ScrollHandle,
}

impl Shell {
    pub(crate) fn skill_matches(&self, cx: &gpui_kit::App) -> Option<Vec<Skill>> {
        if self.skill_menu.dismissed {
            return None;
        }
        let input = self.composer_input.read(cx);
        if !input.selected_range().is_empty() {
            return None;
        }
        let text = input.value();
        let query = slash_query(&text, input.cursor())?;
        Some(self.skills.search(query).into_iter().cloned().collect())
    }

    pub(crate) fn accept_skill_suggestion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(matches) = self.skill_matches(cx) else {
            return false;
        };
        let Some(skill) = matches.get(
            self.skill_menu
                .selected
                .min(matches.len().saturating_sub(1)),
        ) else {
            return false;
        };
        self.choose_skill(&skill.name, window, cx);
        true
    }

    pub(crate) fn choose_skill(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(skill) = self.skills.find(name).cloned() else {
            return;
        };
        let text = self.composer_input.read(cx).value().to_string();
        let arguments = text
            .trim_start()
            .find(char::is_whitespace)
            .map_or("", |index| text.trim_start()[index..].trim_start());
        match skill.kind {
            SkillKind::Action(action) => self.run_skill_action(action, arguments, window, cx),
            SkillKind::Procedure => {
                self.state.team_mode = false;
                self.fill_composer_draft(&format!("/{} {arguments}", skill.name), window, cx);
                self.composer_input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            }
        }
    }

    pub(crate) fn run_skill_action(
        &mut self,
        action: SkillAction,
        arguments: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(action, SkillAction::AddFiles | SkillAction::IndexFolder) {
            self.fill_composer_draft(arguments, window, cx);
        }
        match action {
            SkillAction::AddFiles => self.choose_files(window, cx),
            SkillAction::IndexFolder => self.choose_index_folder(window, cx),
            SkillAction::Help => {
                self.select_tab(NavTab::Skills, cx);
            }
            SkillAction::NewChat => self.start_new_chat(window, cx),
            SkillAction::Models => self.select_tab(NavTab::Models, cx),
            SkillAction::Knowledge => self.select_tab(NavTab::Knowledge, cx),
            SkillAction::Schedules => self.select_tab(NavTab::Schedules, cx),
            SkillAction::Agents => self.select_tab(NavTab::MyAis, cx),
            SkillAction::TeamChat => self.ask_team(None, window, cx),
            SkillAction::Artifacts => self.select_tab(NavTab::Artifacts, cx),
            SkillAction::Connections => {
                self.select_tab(NavTab::Settings, cx);
                self.models().update(cx, |models, cx| {
                    models.send_connection(
                        sovra_core::connections::ConnectionCommand::StartWhatsAppPairing,
                        cx,
                    );
                });
            }
            SkillAction::Settings => self.select_tab(NavTab::Settings, cx),
        }
    }

    pub(crate) fn skill_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        self.skill_navigation(&event.keystroke.key, window, cx);
    }

    pub(crate) fn skill_navigation(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(matches) = self.skill_matches(cx) else {
            cx.propagate();
            return;
        };
        let count = matches.len();
        match key {
            "escape" => self.skill_menu.dismissed = true,
            "down" if count > 0 => {
                self.skill_menu.selected = (self.skill_menu.selected + 1) % count;
            }
            "up" if count > 0 => {
                self.skill_menu.selected = (self.skill_menu.selected + count - 1) % count;
            }
            "enter" | "tab" if count > 0 => {
                self.accept_skill_suggestion(window, cx);
            }
            _ => {
                cx.propagate();
                return;
            }
        }
        cx.stop_propagation();
        window.prevent_default();
        self.skill_menu
            .scroll
            .scroll_to_item(self.skill_menu.selected);
        cx.notify();
    }
}

pub(crate) fn render(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let matches = shell.skill_matches(cx)?;
    let selected = shell.skill_menu.selected;
    let t = design::tokens(cx);
    Some(
        v_flex()
            .id("skill-command-menu")
            .test_support()
            .w_full()
            .flex_shrink_0()
            .min_w_0()
            .max_h(t.layout.command_menu_height)
            .overflow_y_scroll()
            .track_scroll(&shell.skill_menu.scroll)
            .p(t.space.compact)
            .gap(t.space.hair)
            .sovra_surface(design::Surface::Elevated, cx)
            .children(matches.is_empty().then(|| {
                div()
                    .p(t.space.small)
                    .sovra_text(TextRole::Caption, cx)
                    .child("No matching skills. Escape closes this menu.")
            }))
            .children(matches.into_iter().enumerate().map(|(index, skill)| {
                let name = skill.name.clone();
                design::button(
                    ("skill-command", index),
                    format!("/{} · {}", skill.name, skill.title),
                    cx,
                )
                .ghost()
                .selected(index == selected)
                .w_full()
                .justify_start()
                .accessibility_label(format!(
                    "/{}. {} {}",
                    skill.name, skill.description, skill.argument_hint
                ))
                .tooltip(format!("{} {}", skill.description, skill.argument_hint))
                .on_click(
                    cx.listener(move |shell, _, window, cx| shell.choose_skill(&name, window, cx)),
                )
            })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::{Root, Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle, px, size};
    use sovra_core::{Command, models::InterfaceScale};
    use sovra_runtime::Client;

    fn open(
        cx: &mut TestAppContext,
        dimensions: gpui_kit::Size<gpui_kit::Pixels>,
    ) -> (Entity<Shell>, WindowHandle<Root>) {
        let mut shell = None;
        let window = cx.open_window(dimensions, |window, cx| {
            let view = cx.new(|cx| Shell::new(window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        (shell.unwrap(), window)
    }

    fn draft(shell: &Entity<Shell>, text: &str, window: &mut Window, cx: &mut gpui_kit::App) {
        shell.update(cx, |shell, cx| {
            shell.fill_composer_draft(text, window, cx);
            shell
                .composer_input
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        });
        window.render_frame(cx);
    }

    #[gpui_kit::test]
    fn slash_keyboard_completion_never_submits_and_actions_stay_local(cx: &mut TestAppContext) {
        let (client, commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let (shell, window) = open(cx, size(px(1280.), px(820.)));
        cx.update_window(window.into(), |_, window, cx| {
            draft(&shell, "/web", window, cx);
        })
        .unwrap();
        cx.simulate_keystrokes(window.into(), "tab");
        cx.update_window(window.into(), |_, window, cx| {
            assert_eq!(
                shell.read(cx).composer_input.read(cx).value().as_ref(),
                "/web-search "
            );
            assert_eq!(
                shell.read(cx).state.messages,
                [] as [sovra_core::ChatMessage; 0]
            );
            draft(&shell, "/models", window, cx);
        })
        .unwrap();
        cx.simulate_keystrokes(window.into(), "enter");
        cx.update_window(window.into(), |_, window, cx| {
            assert_eq!(shell.read(cx).state.active_tab, NavTab::Models);
            shell.update(cx, |shell, cx| shell.select_tab(NavTab::Chat, cx));
            draft(&shell, "/unknown something", window, cx);
            shell.update(cx, |shell, cx| shell.submit_composer(window, cx));
            assert!(
                shell
                    .read(cx)
                    .skill_menu
                    .notice
                    .as_ref()
                    .unwrap()
                    .contains("Unknown skill")
            );
            assert_eq!(
                shell.read(cx).composer_input.read(cx).value().as_ref(),
                "/unknown something"
            );
        })
        .unwrap();
        assert!(
            !std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                command,
                Command::RunPrompt { .. } | Command::RunWithFiles { .. }
            ))
        );
    }

    #[gpui_kit::test]
    fn cancelling_file_skill_picker_preserves_entire_draft(cx: &mut TestAppContext) {
        let (client, commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let (shell, window) = open(cx, size(px(1280.), px(820.)));
        cx.update_window(window.into(), |_, window, cx| {
            draft(&shell, "/file-system compare these", window, cx);
            shell.update(cx, |shell, cx| shell.submit_composer(window, cx));
        })
        .unwrap();
        cx.simulate_path_prompt_response(|options| {
            assert!(options.files && options.multiple && !options.directories);
            None
        });
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            assert_eq!(
                shell.read(cx).composer_input.read(cx).value().as_ref(),
                "/file-system compare these"
            );
            shell.update(cx, |shell, cx| shell.submit_composer(window, cx));
        })
        .unwrap();
        cx.simulate_path_prompt_response(|_| Some(vec!["/chosen/report.txt".into()]));
        cx.run_until_parked();
        shell.read_with(cx, |shell, cx| {
            assert_eq!(
                shell.composer_input.read(cx).value().as_ref(),
                "compare these"
            );
        });
        assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Files(sovra_core::files::FileCommand::Prepare { path, .. }) if path == std::path::Path::new("/chosen/report.txt"))));
    }

    #[gpui_kit::test]
    fn team_mode_and_explicit_skills_share_the_catalogue_without_losing_requests(
        cx: &mut TestAppContext,
    ) {
        let (client, commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let (shell, window) = open(cx, size(px(1280.), px(820.)));
        cx.update_window(window.into(), |_, window, cx| {
            draft(&shell, "/team compare two plans", window, cx);
            shell.update(cx, |shell, cx| shell.submit_composer(window, cx));
            assert!(shell.read(cx).state.team_mode);
            assert_eq!(
                shell.read(cx).composer_input.read(cx).value().as_ref(),
                "compare two plans"
            );
            shell.update(cx, |shell, cx| shell.submit_composer(window, cx));
            draft(&shell, "/tab", window, cx);
            window.click(("skill-command", 0usize), cx);
            assert!(!shell.read(cx).state.team_mode);
            assert_eq!(
                shell.read(cx).composer_input.read(cx).value().as_ref(),
                "/table "
            );
        })
        .unwrap();
        assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
            Command::Teams(sovra_core::teams::TeamCommand::Run { prompt, .. }) if prompt == "compare two plans")));
    }

    #[gpui_kit::test]
    fn arrows_wrap_and_tab_accepts_the_selected_skill(cx: &mut TestAppContext) {
        let (client, _commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let (shell, window) = open(cx, size(px(1280.), px(820.)));
        cx.update_window(window.into(), |_, window, cx| {
            draft(&shell, "/", window, cx);
        })
        .unwrap();
        cx.simulate_keystrokes(window.into(), "down");
        shell.read_with(cx, |shell, _| assert_eq!(shell.skill_menu.selected, 1));
        cx.simulate_keystrokes(window.into(), "up up");
        let expected = shell.read_with(cx, |shell, cx| {
            let matches = shell.skill_matches(cx).unwrap();
            assert_eq!(shell.skill_menu.selected, matches.len() - 1);
            format!("/{} ", matches.last().unwrap().name)
        });
        cx.simulate_keystrokes(window.into(), "tab");
        shell.read_with(cx, |shell, cx| {
            assert_eq!(shell.composer_input.read(cx).value().as_ref(), expected);
        });
    }

    #[gpui_kit::test]
    fn index_skill_uses_a_single_folder_picker_and_cancellation_keeps_the_draft(
        cx: &mut TestAppContext,
    ) {
        let (client, commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let (shell, window) = open(cx, size(px(1280.), px(820.)));
        cx.update_window(window.into(), |_, window, cx| {
            draft(&shell, "/index-file-system project notes", window, cx);
            shell.update(cx, |shell, cx| shell.submit_composer(window, cx));
        })
        .unwrap();
        cx.simulate_path_prompt_response(|options| {
            assert!(options.directories && !options.files && !options.multiple);
            None
        });
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            assert_eq!(
                shell.read(cx).composer_input.read(cx).value().as_ref(),
                "/index-file-system project notes"
            );
            shell.update(cx, |shell, cx| shell.submit_composer(window, cx));
        })
        .unwrap();
        cx.simulate_path_prompt_response(|_| Some(vec!["/chosen/project".into()]));
        cx.run_until_parked();
        shell.read_with(cx, |shell, cx| {
            assert_eq!(shell.state.active_tab, NavTab::Knowledge);
            assert_eq!(
                shell.composer_input.read(cx).value().as_ref(),
                "project notes"
            );
        });
        assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
            Command::FileIndex(sovra_core::file_index::IndexCommand::AddRoot { path }) if path == std::path::Path::new("/chosen/project"))));
    }

    #[gpui_kit::test]
    fn slash_popup_fits_minimum_wide_and_double_text_sizes_in_both_themes(cx: &mut TestAppContext) {
        let (client, _commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let dimensions = size(px(width), px(height));
            let (shell, window) = open(cx, dimensions);
            cx.update_window(window.into(), |_, window, cx| {
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                for mode in [ThemeMode::Dark, ThemeMode::Light] {
                    Theme::change(mode, Some(window), cx);
                    draft(&shell, "/", window, cx);
                    window.render_frame(cx);
                    let bounds = window.find("skill-command-menu").bounds();
                    assert!(
                        bounds.left() >= px(0.) && bounds.right() <= dimensions.width,
                        "{bounds:?}"
                    );
                    assert!(
                        bounds.top() >= px(0.) && bounds.bottom() <= dimensions.height,
                        "{scale}%: {bounds:?}"
                    );
                    assert!(window.find("chat-composer-input").visible());
                }
            })
            .unwrap();
            cx.simulate_keystrokes(window.into(), "escape");
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("skill-command-menu").is_none());
                assert_eq!(shell.read(cx).composer_input.read(cx).value().as_ref(), "/");
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn skills_library_filters_categories_shows_examples_and_auto_slugs_custom_skills(
        cx: &mut TestAppContext,
    ) {
        let (client, commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let (shell, window) = open(cx, size(px(1280.), px(820.)));
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.select_tab(NavTab::Skills, cx));
            window.render_frame(cx);
            assert!(window.find("sovra-skills-view").visible());
            assert!(window.find("skills-how-it-works").visible());
            assert!(window.find("skill-category-filters").visible());

            // Filter by Web search category
            window.click("skill-filter-research", cx);
            window.render_frame(cx);
            assert!(window.find(("skill-card", 13usize)).visible());

            // Expand "How it works" instructions on the web-search card
            window.click(("skill-details", 13usize), cx);
            window.render_frame(cx);
            assert!(window.find(("skill-instructions-box", 13usize)).visible());

            // Click "Try in chat" on web-search -> switches to Chat and fills `/web-search `
            window.click(("skill-use", 13usize), cx);
            window.render_frame(cx);
            assert_eq!(shell.read(cx).state.active_tab, NavTab::Chat);
            assert_eq!(
                shell.read(cx).composer_input.read(cx).value().as_ref(),
                "/web-search "
            );

            // Return to Skills and create a custom skill without typing a technical command slug
            shell.update(cx, |shell, cx| shell.select_tab(NavTab::Skills, cx));
            window.render_frame(cx);
            window.click("skill-create", cx);
            window.render_frame(cx);
            assert!(window.find("skill-editor").visible());
            shell.update(cx, |shell, cx| {
                shell.skill_library.set_editor_fields_for_test(
                    "",
                    "Sunday Family Dinner",
                    "Plan a cozy 3-course Sunday family meal and grocery list.",
                    "Ask about dietary needs, suggest 3 seasonal menus, and make a shopping list.",
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            window.click("skill-save", cx);
            window.render_frame(cx);
        })
        .unwrap();

        let saved = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|cmd| match cmd {
            Command::Skills(sovra_core::skills::SkillCommand::Save(skill)) => Some(*skill),
            _ => None,
        });
        let saved = saved.expect("saving a skill dispatches SkillCommand::Save");
        assert_eq!(saved.name, "sunday-family-dinner");
        assert_eq!(saved.title, "Sunday Family Dinner");
    }
}
