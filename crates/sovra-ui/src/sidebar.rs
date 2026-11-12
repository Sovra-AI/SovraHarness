//! Unified left sidebar: Brand header + collapse toggle, prominent `+ New Chat` hero CTA,
//! chunked & plain-language primary navigation (`Workspace` vs `Your AI & Setup`) with live
//! context badges and helpful tooltips, integrated conversation history, and user footer.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::InputState,
    v_flex,
};
use gpui_kit::{
    Context, ElementId, Entity, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::models::ChromeKey;
use sovra_core::{NavTab, WorkspaceState};

use crate::chat_history_view::render_chat_history_section;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::{Shell, logo};

const PRIMARY_TABS: [NavTab; 6] = [
    NavTab::Chat,
    NavTab::Skills,
    NavTab::Team,
    NavTab::Artifacts,
    NavTab::Schedules,
    NavTab::Groups,
];

const AI_SETUP_TABS: [NavTab; 5] = [
    NavTab::Models,
    NavTab::MyAis,
    NavTab::Knowledge,
    NavTab::Connections,
    NavTab::Settings,
];

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
    let mut sidebar = v_flex()
        .id("sovra-sidebar")
        .test_support()
        .w(t.layout.sidebar_width)
        .h_full()
        .min_h_0()
        .flex_shrink_0()
        .justify_between()
        .p(t.space.small)
        .gap(t.space.small)
        .sovra_surface(Surface::Chrome, cx);
    sidebar = if design::is_rtl(cx) {
        sidebar.border_l_1()
    } else {
        sidebar.border_r_1()
    };
    sidebar
        .border_color(border)
        .overflow_hidden()
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .gap(t.space.small)
                .child(render_brand_header(cx))
                .child(render_new_chat_button(cx))
                .child(
                    div()
                        .id("sidebar-nav-scroll")
                        .w_full()
                        .min_h_0()
                        .flex_shrink(1.0)
                        .overflow_y_scroll()
                        .child(render_nav_list(state, cx)),
                )
                .child(div().w_full().border_b_1().border_color(border))
                .child(render_chat_history_section(
                    state,
                    history_search_input,
                    rename_thread_input,
                    cx,
                )),
        )
        .child(render_sidebar_footer(state, cx))
        .into_any_element()
}

fn render_sidebar_footer(
    state: &WorkspaceState,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .flex_shrink_0()
        .w_full()
        .pt(t.space.compact)
        .gap(t.space.compact)
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            design::button(
                "sidebar-tour-btn",
                design::chrome(cx, ChromeKey::NavTour),
                cx,
            )
            .icon(IconName::Sparkles)
            .tooltip(design::chrome(cx, ChromeKey::NavTourHint))
            .accessibility_label(design::chrome(cx, ChromeKey::NavTour))
            .small()
            .ghost()
            .w_full()
            .on_click(cx.listener(|shell, _, _, cx| shell.start_system_tour(cx))),
        )
        .child(render_user_profile(state, cx))
}

fn rail_btn(
    id: impl Into<ElementId>,
    tip: impl Into<gpui_kit::SharedString>,
    label: impl Into<gpui_kit::SharedString>,
    cx: &Context<Shell>,
) -> Button {
    let t = design::tokens(cx);
    design::button(id, "", cx)
        .tooltip(tip)
        .accessibility_label(label)
        .ghost()
        .xsmall()
        .min_h(t.layout.compact_control_height)
        .px(t.space.compact)
}

fn render_collapsed_sidebar(
    state: &WorkspaceState,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let link = cx.theme().link;
    let expand = rail_btn(
        "chat-history-expand-btn",
        design::chrome(cx, ChromeKey::NavExpand),
        design::chrome(cx, ChromeKey::NavExpand),
        cx,
    )
    .child(
        Icon::new(IconName::PanelLeftOpen)
            .small()
            .text_color(t.text_secondary),
    )
    .on_click(cx.listener(|shell, _, _, cx| shell.toggle_history_rail(cx)));
    let new_chat = rail_btn(
        "chat-new-btn-collapsed",
        design::chrome(cx, ChromeKey::NavNewChatHint),
        design::chrome(cx, ChromeKey::NavNewChatHint),
        cx,
    )
    .child(Icon::new(IconName::Plus).small().text_color(link))
    .on_click(cx.listener(|shell, _, window, cx| shell.start_new_chat(window, cx)));
    let profile = rail_btn("sidebar-user-profile", "Sign out", "Sign out", cx)
        .child(
            h_flex()
                .size(t.layout.icon_medium)
                .items_center()
                .justify_center()
                .rounded(t.radius.pill)
                .bg(t.nav_active_background)
                .sovra_text(TextRole::Caption, cx)
                .child(state.user.initials.clone()),
        )
        .on_click(cx.listener(|shell, _, _, cx| shell.logout(cx)));

    let mut collapsed_sidebar = v_flex()
        .id("sovra-sidebar")
        .test_support()
        .w(t.layout.history_compact_width)
        .h_full()
        .min_h_0()
        .flex_shrink_0()
        .justify_between()
        .p(t.space.compact)
        .items_center()
        .sovra_surface(Surface::Chrome, cx);
    collapsed_sidebar = if design::is_rtl(cx) {
        collapsed_sidebar.border_l_1()
    } else {
        collapsed_sidebar.border_r_1()
    };
    collapsed_sidebar
        .border_color(cx.theme().border)
        .child(
            v_flex()
                .id("chat-history-rail-collapsed")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .test_support()
                .items_center()
                .gap(t.space.hair)
                .child(expand)
                .child(new_chat)
                .children(NavTab::ALL.into_iter().map(|tab| {
                    let is_active = state.active_tab == tab;
                    let (id, icon) = nav_tab_meta(tab);
                    let color = if is_active { link } else { t.text_secondary };
                    rail_btn(
                        id,
                        design::chrome(cx, tab.tooltip_key()),
                        tab.label_in(&state.language),
                        cx,
                    )
                    .selected(is_active)
                    .child(
                        div()
                            .id(("nav-icon", tab as usize))
                            .test_support()
                            .child(Icon::new(icon).small().text_color(color)),
                    )
                    .on_click(cx.listener(move |shell, _, _, cx| shell.select_tab(tab, cx)))
                })),
        )
        .child(profile)
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
                .gap(t.space.small)
                .child(logo("sidebar-logo", t.layout.logo_small))
                .child(div().sovra_text(TextRole::SectionTitle, cx).child("Sovra")),
        )
        .child(
            design::button("chat-history-collapse-btn", "", cx)
                .tooltip(design::chrome(cx, ChromeKey::NavCollapse))
                .accessibility_label(design::chrome(cx, ChromeKey::NavCollapse))
                .ghost()
                .small()
                .min_h(t.layout.compact_control_height)
                .px(t.space.compact)
                .child(
                    Icon::new(IconName::PanelLeftClose)
                        .small()
                        .text_color(t.text_secondary),
                )
                .on_click(cx.listener(|shell, _, _, cx| shell.toggle_history_rail(cx))),
        )
}

fn render_new_chat_button(cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    design::primary_button(
        "chat-new-btn",
        design::chrome(cx, ChromeKey::NavNewChat),
        cx,
    )
    .icon(IconName::Plus)
    .tooltip(design::chrome(cx, ChromeKey::NavNewChatHint))
    .accessibility_label(design::chrome(cx, ChromeKey::NavNewChatHint))
    .small()
    .w_full()
    .rounded(t.radius.input)
    .on_click(cx.listener(|shell, _, window, cx| shell.start_new_chat(window, cx)))
}

fn nav_tab_meta(tab: NavTab) -> (&'static str, IconName) {
    match tab {
        NavTab::Chat => ("nav-chat", IconName::MessageCircle),
        NavTab::Team => ("nav-team", IconName::Users),
        NavTab::Artifacts => ("nav-artifacts", IconName::LibraryBig),
        NavTab::Schedules => ("nav-schedules", IconName::Calendar),
        NavTab::Models => ("nav-models", IconName::Box),
        NavTab::MyAis => ("nav-my-ais", IconName::User),
        NavTab::Knowledge => ("nav-knowledge", IconName::Database),
        NavTab::Skills => ("nav-skills", IconName::BookOpen),
        NavTab::Settings => ("nav-settings", IconName::Settings),
        NavTab::Connections => ("nav-connections", IconName::Link),
        NavTab::Groups => ("nav-groups", IconName::Heart),
    }
}

fn nav_context_badge(tab: NavTab, state: &WorkspaceState) -> Option<(String, bool)> {
    match tab {
        NavTab::Models if state.models_loaded && !state.has_active_llm => Some((
            sovra_core::models::chrome(&state.language, ChromeKey::NavSetupBadge).to_owned(),
            true,
        )),
        NavTab::Artifacts if !state.artifacts.artifacts.is_empty() => {
            Some((state.artifacts.artifacts.len().to_string(), false))
        }
        NavTab::Schedules if !state.schedules.is_empty() => {
            Some((state.schedules.len().to_string(), false))
        }
        NavTab::Connections if state.connections.whatsapp.is_connected() => {
            Some(("WhatsApp".to_owned(), true))
        }
        _ => None,
    }
}

fn render_nav_list(state: &WorkspaceState, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let active = state.active_tab;
    let tour = state.tour_step.map(|s| (s.target_tab(), s.step_number()));
    let mut list = v_flex().w_full().gap(t.space.hair);
    for tab in PRIMARY_TABS {
        let spot = tour
            .filter(|(t_tab, _)| *t_tab == tab)
            .map(|(_, n)| format!("Step {n}"));
        list = list.child(render_nav_item(
            tab,
            tab == active,
            spot,
            nav_context_badge(tab, state),
            cx,
        ));
    }
    list = list.child(
        div()
            .id("sidebar-group-ai-setup")
            .test_support()
            .px(t.space.small)
            .pt(t.space.snug)
            .pb(t.space.hair)
            .sovra_text(TextRole::Caption, cx)
            .text_color(cx.theme().muted_foreground)
            .child(design::chrome(cx, ChromeKey::NavGroupAiSetup)),
    );
    for tab in AI_SETUP_TABS {
        let spot = tour
            .filter(|(t_tab, _)| *t_tab == tab)
            .map(|(_, n)| format!("Step {n}"));
        list = list.child(render_nav_item(
            tab,
            tab == active,
            spot,
            nav_context_badge(tab, state),
            cx,
        ));
    }
    list
}

fn badge_pill(
    id: (&'static str, usize),
    text: String,
    bg: Hsla,
    fg: Hsla,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    div()
        .id(id)
        .test_support()
        .px(t.space.compact)
        .py(t.space.hair)
        .rounded(t.radius.pill)
        .bg(bg)
        .sovra_text(TextRole::Caption, cx)
        .text_color(fg)
        .child(text)
}

fn render_nav_item(
    tab: NavTab,
    is_active: bool,
    spotlight_badge: Option<String>,
    context_badge: Option<(String, bool)>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let (fg, link) = (cx.theme().foreground, cx.theme().link);
    let (id, icon) = nav_tab_meta(tab);
    let tooltip = design::chrome(cx, tab.tooltip_key());
    let label = design::chrome(cx, tab.chrome_key());
    let text_fg = if is_active { fg } else { t.text_secondary };
    let icon_color = if is_active || spotlight_badge.is_some() {
        link
    } else {
        t.text_secondary
    };
    let idx = tab as usize;
    let ctx_pill = spotlight_badge
        .is_none()
        .then_some(context_badge)
        .flatten()
        .map(|(txt, hi)| {
            let (bg, col) = if hi {
                (t.nav_active_background, link)
            } else {
                (t.subtle, t.text_secondary)
            };
            badge_pill(("nav-context-badge", idx), txt, bg, col, cx)
        });
    let spot_pill = spotlight_badge.map(|txt| {
        badge_pill(
            ("nav-spotlight-badge", idx),
            txt,
            t.nav_active_background,
            link,
            cx,
        )
    });

    design::button(id, "", cx)
        .tooltip(tooltip)
        .accessibility_label(label)
        .ghost()
        .small()
        .min_h(t.layout.navigation_height)
        .selected(is_active)
        .w_full()
        .px(t.space.small)
        .items_center()
        .justify_start()
        .gap(t.space.small)
        .rounded(t.radius.input)
        .text_color(text_fg)
        .child({
            let mut row = h_flex()
                .w_full()
                .justify_start()
                .items_center()
                .gap(t.space.small);
            if design::is_rtl(cx) {
                row = row.flex_row_reverse();
            }
            row.child(
                div()
                    .id(("nav-icon", idx))
                    .test_support()
                    .child(Icon::new(icon).small().text_color(icon_color)),
            )
            .child(
                div()
                    .id(("nav-label", idx))
                    .test_support()
                    .flex_1()
                    .truncate()
                    .child(label),
            )
            .children(spot_pill)
            .children(ctx_pill)
        })
        .on_click(cx.listener(move |shell, _, _, cx| shell.select_tab(tab, cx)))
}

fn render_user_profile(
    state: &WorkspaceState,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let avatar = h_flex()
        .size(t.layout.profile_avatar)
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(t.radius.pill)
        .bg(t.nav_active_background)
        .sovra_text(TextRole::Caption, cx)
        .text_color(cx.theme().link)
        .child(state.user.initials.clone());
    let labels = v_flex()
        .min_w_0()
        .flex_1()
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .truncate()
                .child(state.user.full_name.clone()),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .truncate()
                .child("Sign out"),
        );

    design::button("sidebar-user-profile", "", cx)
        .tooltip("Sign out")
        .accessibility_label("Sign out")
        .ghost()
        .h_auto()
        .w_full()
        .items_center()
        .justify_between()
        .p(t.space.compact)
        .rounded(t.radius.input)
        .child(
            h_flex()
                .items_center()
                .min_w_0()
                .flex_1()
                .gap(t.space.small)
                .child(avatar)
                .child(labels),
        )
        .on_click(cx.listener(|shell, _, _, cx| shell.logout(cx)))
}
