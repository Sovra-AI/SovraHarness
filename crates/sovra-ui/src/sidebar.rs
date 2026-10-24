//! Unified left sidebar: Brand header + collapse toggle, prominent `+ New Chat` CTA at the top,
//! primary navigation (`Chat`, `My AI's`, `Knowledge`, `Models`, `Schedules`, `Settings`),
//! integrated conversation history (`Pinned`, `Today`, `Yesterday`, `Previous 7 Days`, etc.,
//! search, `Chats | Scheduled` filter pills, and AI-titled threads), and user profile footer.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, button::ButtonVariants as _, h_flex,
    input::InputState, v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div, px,
};
use sovra_core::{NavTab, WorkspaceState};

use crate::chat_history_view::render_chat_history_section;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::{Shell, logo};

pub(crate) fn render_sidebar(
    state: &WorkspaceState,
    history_search_input: &Entity<InputState>,
    rename_thread_input: &Entity<InputState>,
    collapsed: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    if collapsed {
        return render_collapsed_sidebar(state, cx).into_any_element();
    }

    let t = design::tokens(cx);
    let border = cx.theme().border;

    v_flex()
        .id("sovra-sidebar")
        .test_support()
        .w(t.layout.sidebar_width)
        .h_full()
        .min_h_0()
        .flex_shrink_0()
        .justify_between()
        .p(t.space.compact)
        .gap(t.space.compact)
        .sovra_surface(Surface::Chrome, cx)
        .border_r_1()
        .border_color(border)
        .overflow_hidden()
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .gap(px(2.))
                .child(render_brand_header(cx))
                .child(render_new_chat_button(cx))
                .child(render_nav_list(state, cx))
                .child(div().w_full().border_b_1().border_color(border))
                .child(render_chat_history_section(
                    state,
                    history_search_input,
                    rename_thread_input,
                    cx,
                )),
        )
        .child(
            v_flex()
                .flex_shrink_0()
                .gap(t.space.compact)
                .child(
                    design::button("sidebar-tour-btn", "Guided System Tour", cx)
                        .small()
                        .ghost()
                        .w_full()
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.start_system_tour(cx);
                        })),
                )
                .child(render_user_profile(state, cx)),
        )
        .into_any_element()
}

fn render_collapsed_sidebar(
    state: &WorkspaceState,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let border = cx.theme().border;

    v_flex()
        .id("sovra-sidebar")
        .test_support()
        .w(t.layout.history_compact_width)
        .h_full()
        .min_h_0()
        .flex_shrink_0()
        .justify_between()
        .py(px(4.))
        .px(t.space.compact)
        .items_center()
        .sovra_surface(Surface::Chrome, cx)
        .border_r_1()
        .border_color(border)
        .child(
            v_flex()
                .id("chat-history-rail-collapsed")
                .test_support()
                .items_center()
                .gap(px(2.))
                .child(logo("sidebar-logo", px(20.)))
                .child(
                    design::button("chat-history-expand-btn", "", cx)
                        .accessibility_label("Expand sidebar and chat history")
                        .ghost()
                        .xsmall()
                        .min_h(px(24.))
                        .px(t.space.compact)
                        .child(
                            Icon::new(IconName::PanelLeftOpen)
                                .small()
                                .text_color(t.text_secondary),
                        )
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.toggle_history_rail(cx);
                        })),
                )
                .child(
                    design::button("chat-new-btn-collapsed", "", cx)
                        .accessibility_label("Start a new chat")
                        .ghost()
                        .xsmall()
                        .min_h(px(24.))
                        .px(t.space.compact)
                        .child(
                            Icon::new(IconName::Plus)
                                .small()
                                .text_color(t.accent.violet),
                        )
                        .on_click(cx.listener(|shell, _, window, cx| {
                            shell.start_new_chat(window, cx);
                        })),
                )
                .children(NavTab::ALL.into_iter().map(|tab| {
                    let is_active = state.active_tab == tab;
                    let (id, icon) = nav_tab_meta(tab);
                    let icon_color = if is_active {
                        t.accent.violet
                    } else {
                        t.text_secondary
                    };
                    design::button(id, "", cx)
                        .accessibility_label(tab.label())
                        .ghost()
                        .xsmall()
                        .min_h(px(24.))
                        .selected(is_active)
                        .px(t.space.compact)
                        .child(
                            div()
                                .id(("nav-icon", tab as usize))
                                .test_support()
                                .child(Icon::new(icon).small().text_color(icon_color)),
                        )
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.select_tab(tab, cx);
                        }))
                })),
        )
        .child(
            design::button("sidebar-user-profile", "", cx)
                .accessibility_label("User profile and sign out")
                .ghost()
                .xsmall()
                .min_h(px(24.))
                .px(t.space.compact)
                .child(
                    h_flex()
                        .size(px(24.))
                        .items_center()
                        .justify_center()
                        .rounded(t.radius.pill)
                        .bg(t.nav_active_background)
                        .sovra_text(TextRole::Caption, cx)
                        .child(state.user.initials.clone()),
                )
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.logout(cx);
                })),
        )
}

fn render_brand_header(cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .px(t.space.compact)
        .child(
            h_flex()
                .items_center()
                .gap(t.space.compact)
                .child(logo("sidebar-logo", t.layout.logo_small))
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("S O V R A"),
                ),
        )
        .child(
            design::button("chat-history-collapse-btn", "", cx)
                .accessibility_label("Collapse sidebar")
                .ghost()
                .small()
                .px(t.space.compact)
                .child(
                    Icon::new(IconName::PanelLeftClose)
                        .small()
                        .text_color(t.text_secondary),
                )
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.toggle_history_rail(cx);
                })),
        )
}

fn render_new_chat_button(cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    design::primary_button("chat-new-btn", "+ New Chat", cx)
        .accessibility_label("Start a new chat")
        .small()
        .w_full()
        .rounded(t.radius.input)
        .on_click(cx.listener(|shell, _, window, cx| {
            shell.start_new_chat(window, cx);
        }))
}

fn nav_tab_meta(tab: NavTab) -> (&'static str, IconName) {
    match tab {
        NavTab::Chat => ("nav-chat", IconName::MessageCircle),
        NavTab::MyAis => ("nav-my-ais", IconName::User),
        NavTab::Knowledge => ("nav-knowledge", IconName::Database),
        NavTab::Models => ("nav-models", IconName::Box),
        NavTab::Schedules => ("nav-schedules", IconName::Calendar),
        NavTab::Settings => ("nav-settings", IconName::Settings),
    }
}

fn render_nav_list(state: &WorkspaceState, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let active = state.active_tab;
    let tour_target = state.tour_step.map(|s| (s.target_tab(), s.step_number()));
    v_flex()
        .gap(px(1.))
        .children(NavTab::ALL.into_iter().map(|tab| {
            let badge = tour_target
                .filter(|(t, _)| *t == tab)
                .map(|(_, step)| format!("Step {step}"));
            render_nav_item(tab, tab == active, badge, cx)
        }))
}

fn render_nav_item(
    tab: NavTab,
    is_active: bool,
    spotlight_badge: Option<String>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();
    let (id, icon) = nav_tab_meta(tab);
    let fg = if is_active {
        theme.foreground
    } else {
        t.text_secondary
    };
    let icon_color = if is_active || spotlight_badge.is_some() {
        t.accent.violet
    } else {
        t.text_secondary
    };

    design::button(id, "", cx)
        .accessibility_label(tab.label())
        .ghost()
        .xsmall()
        .min_h(px(22.))
        .selected(is_active)
        .child(
            h_flex()
                .w_full()
                .justify_start()
                .items_center()
                .gap(t.space.small)
                .child(
                    div()
                        .id(("nav-icon", tab as usize))
                        .test_support()
                        .child(Icon::new(icon).small().text_color(icon_color)),
                )
                .child(
                    div()
                        .id(("nav-label", tab as usize))
                        .test_support()
                        .flex_1()
                        .child(tab.label()),
                )
                .children(spotlight_badge.map(|badge_text| {
                    div()
                        .id(("nav-spotlight-badge", tab as usize))
                        .test_support()
                        .px(t.space.compact)
                        .py(px(2.))
                        .rounded(t.radius.pill)
                        .bg(t.subtle)
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.accent.violet)
                        .child(badge_text)
                })),
        )
        .w_full()
        .px(t.space.small)
        .items_center()
        .justify_start()
        .gap(t.space.small)
        .rounded(t.radius.input)
        .text_color(fg)
        .on_click(cx.listener(move |shell, _, _, cx| {
            shell.select_tab(tab, cx);
        }))
}

fn render_user_profile(
    state: &WorkspaceState,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .id("sidebar-user-profile")
        .test_support()
        .role(Role::Button)
        .aria_label("User profile and sign out")
        .cursor_pointer()
        .w_full()
        .items_center()
        .justify_between()
        .p(t.space.compact)
        .rounded(t.radius.input)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            h_flex()
                .items_center()
                .gap(t.space.compact)
                .child(
                    h_flex()
                        .size(px(32.))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded(t.radius.pill)
                        .bg(t.nav_active_background)
                        .sovra_text(TextRole::Caption, cx)
                        .child(state.user.initials.clone()),
                )
                .child(
                    v_flex()
                        .child(
                            div()
                                .sovra_text(TextRole::Caption, cx)
                                .child(state.user.full_name.clone()),
                        )
                        .child(
                            div()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(t.text_secondary)
                                .child(state.user.workspace_label.clone()),
                        ),
                ),
        )
        .child(
            Icon::new(IconName::ChevronDown)
                .xsmall()
                .text_color(t.text_secondary),
        )
        .on_click(cx.listener(|shell, _, _, cx| {
            shell.logout(cx);
        }))
}
