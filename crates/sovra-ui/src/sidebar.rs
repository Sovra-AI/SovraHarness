//!Left navigation sidebar matching the main chat interface design + `SovraSRX` Schedules.

use gpui_kit::assets::IconName; // full Lucide catalog
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, button::ButtonVariants as _, h_flex,
    v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div, px,
};
use sovra_core::{NavTab, WorkspaceState};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::{Shell, logo};

pub(crate) fn render_sidebar(
    state: &WorkspaceState,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();

    v_flex()
        .id("sovra-sidebar")
        .w(t.layout.sidebar_width)
        .h_full()
        .min_h_0()
        .flex_shrink_0()
        .justify_between()
        .p(t.space.medium)
        .sovra_surface(Surface::Chrome, cx)
        .border_r_1()
        .border_color(theme.border)
        .overflow_y_scroll()
        .child(
            v_flex()
                .gap(t.space.medium)
                .child(render_brand_header(cx))
                .child(render_nav_list(state.active_tab, cx)),
        )
        .child(
            v_flex()
                .gap(t.space.medium)
                .child(render_skyline_tagline(cx))
                .child(render_user_profile(state, cx)),
        )
}

fn render_brand_header(cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .gap(px(4.))
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
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child("YOUR KNOWLEDGE INTO INTELLIGENCE"),
        )
}

fn render_nav_list(active: NavTab, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex().gap(t.space.compact).children(
        NavTab::ALL
            .into_iter()
            .map(|tab| render_nav_item(tab, tab == active, cx)),
    )
}

fn render_nav_item(
    tab: NavTab,
    is_active: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();
    let (id, icon) = match tab {
        NavTab::Chat => ("nav-chat", IconName::MessageCircle),
        NavTab::MyAis => ("nav-my-ais", IconName::User),
        NavTab::Knowledge => ("nav-knowledge", IconName::Database),
        NavTab::Models => ("nav-models", IconName::Box),
        NavTab::Schedules => ("nav-schedules", IconName::Calendar),
        NavTab::Settings => ("nav-settings", IconName::Settings),
    };
    let fg = if is_active {
        theme.foreground
    } else {
        t.text_secondary
    };
    let icon_color = if is_active {
        t.accent.violet
    } else {
        t.text_secondary
    };

    design::button(id, "", cx)
        .accessibility_label(tab.label())
        .ghost()
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
                        .child(tab.label()),
                ),
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

fn render_skyline_tagline(cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let heights = [16., 28., 22., 48., 34., 64., 38., 26., 18.];
    v_flex()
        .items_start()
        .gap(t.space.compact)
        .py(t.space.small)
        .child(
            h_flex()
                .items_end()
                .gap(px(3.))
                .h(px(64.))
                .children(heights.into_iter().map(|h| {
                    div()
                        .w(px(6.))
                        .h(px(h))
                        .rounded(px(2.))
                        .bg(t.material_border)
                })),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child("A more capable you, everywhere."),
        )
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
