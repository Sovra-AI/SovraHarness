//! `SovraSRX` Schedules Command Center view + contextual workspace views (`My AI's`,
//! `Knowledge`, `Models`, `Settings`).

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use sovra_core::{ScheduleItem, ScheduleStatus, WorkspaceState};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

pub(crate) fn render_schedules_view(
    state: &WorkspaceState,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let stats = state.schedule_stats();

    v_flex()
        .id("sovra-schedules-view")
        .flex_1()
        .min_w_0()
        .h_full()
        .p(t.space.large)
        .gap(t.space.medium)
        .overflow_y_scroll()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    v_flex()
                        .gap(px(2.))
                        .child(
                            div()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(cx.theme().link)
                                .child("AUTOMATION · COMMAND CENTER"),
                        )
                        .child(
                            div()
                                .sovra_text(TextRole::PageTitle, cx)
                                .child("Schedules"),
                        )
                        .child(
                            div()
                                .sovra_text(TextRole::Body, cx)
                                .text_color(t.text_secondary)
                                .child(
                                    "Turn recurring work into reliable routines and monitor every run.",
                                ),
                        ),
                )
                .child(
                    design::primary_button("back-to-chat-btn", "Back to Chat", cx).on_click(
                        cx.listener(|shell, _, _, cx| {
                            shell.select_tab(sovra_core::NavTab::Chat, cx);
                        }),
                    ),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .gap(t.space.small)
                .child(render_stat_card(
                    "Active Schedules",
                    &format!("{} / {}", stats.active_schedules, stats.total_schedules),
                    cx,
                ))
                .child(render_stat_card(
                    "Runs Today",
                    &stats.runs_today.to_string(),
                    cx,
                ))
                .child(render_stat_card(
                    "30d Success Rate",
                    &format!("{}%", stats.success_rate_pct),
                    cx,
                )),
        )
        .child(
            v_flex()
                .gap(t.space.small)
                .children(
                    state
                        .schedules
                        .iter()
                        .enumerate()
                        .map(|(idx, item)| render_schedule_row(item, idx, cx)),
                ),
        )
}

fn render_stat_card(label: &str, value: &str, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .flex_1()
        .p(t.space.medium)
        .gap(px(4.))
        .sovra_surface(Surface::Matte, cx)
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(label.to_owned()),
        )
        .child(
            div()
                .sovra_text(TextRole::PageTitle, cx)
                .child(value.to_owned()),
        )
}

fn render_schedule_row(
    item: &ScheduleItem,
    idx: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();
    let schedule_id_toggle = item.id.clone();
    let schedule_id_run = item.id.clone();
    let is_active = item.status == ScheduleStatus::Active;
    let status_color = if is_active {
        theme.success
    } else {
        theme.muted_foreground
    };

    h_flex()
        .id(("schedule-row", idx))
        .w_full()
        .items_center()
        .justify_between()
        .p(t.space.medium)
        .sovra_surface(Surface::Matte, cx)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(4.))
                .child(
                    h_flex()
                        .items_center()
                        .gap(t.space.small)
                        .child(
                            div()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child(item.name.clone()),
                        )
                        .child(
                            div()
                                .px(t.space.compact)
                                .py(px(2.))
                                .rounded(t.radius.pill)
                                .bg(t.subtle)
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(status_color)
                                .child(item.status.label()),
                        ),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(format!(
                            "{} · Next: {} · {}",
                            item.cadence, item.next_run, item.last_result
                        )),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .gap(t.space.small)
                .child(
                    h_flex()
                        .id(("schedule-toggle", idx))
                        .role(Role::Button)
                        .aria_label("Toggle schedule status")
                        .cursor_pointer()
                        .px(t.space.small)
                        .py(px(6.))
                        .rounded(t.radius.button)
                        .sovra_surface(Surface::Subtle, cx)
                        .sovra_text(TextRole::Caption, cx)
                        .child(if is_active { "Pause" } else { "Resume" })
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.toggle_schedule(&schedule_id_toggle, cx);
                        })),
                )
                .child(
                    h_flex()
                        .id(("schedule-run", idx))
                        .role(Role::Button)
                        .aria_label("Run schedule now in chat")
                        .cursor_pointer()
                        .px(t.space.small)
                        .py(px(6.))
                        .items_center()
                        .gap(px(4.))
                        .rounded(t.radius.button)
                        .bg(t.nav_active_background)
                        .sovra_text(TextRole::Caption, cx)
                        .child(
                            Icon::new(IconName::Play)
                                .xsmall()
                                .text_color(t.accent.violet),
                        )
                        .child("Run in Chat")
                        .on_click(cx.listener(move |shell, _, window, cx| {
                            shell.run_schedule_in_chat(&schedule_id_run, window, cx);
                        })),
                ),
        )
}
