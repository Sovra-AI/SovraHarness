//! `SovraSRX` Schedules Command Center view (`ScheduleDashboard` + `ScheduleEmpty` +
//! `ScheduleForm` from `SovraSRXProduction`).

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::{ScheduleItem, ScheduleStatus, WorkspaceState};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

pub(crate) fn render_schedules_view(
    state: &WorkspaceState,
    schedule_name_input: &Entity<InputState>,
    schedule_cadence_input: &Entity<InputState>,
    schedule_prompt_input: &Entity<InputState>,
    schedule_form_open: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let stats = state.schedule_stats();

    let mut page = v_flex()
        .w_full()
        .max_w(t.layout.page_width)
        .mx_auto()
        .gap(t.space.large)
        .child(render_header(schedule_form_open, cx))
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
        );

    if schedule_form_open {
        page = page.child(render_schedule_form(
            schedule_name_input,
            schedule_cadence_input,
            schedule_prompt_input,
            cx,
        ));
    }

    let content = if state.schedules.is_empty() {
        page.child(render_schedules_placeholder(cx))
    } else {
        page.child(
            v_flex()
                .id("schedules-list")
                .test_support()
                .gap(t.space.small)
                .children(
                    state
                        .schedules
                        .iter()
                        .enumerate()
                        .map(|(idx, item)| render_schedule_row(item, idx, cx)),
                ),
        )
    };
    v_flex()
        .id("sovra-schedules-view")
        .test_support()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .p(t.space.large)
        .child(content)
}

fn render_header(schedule_form_open: bool, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .w_full()
        .flex_wrap()
        .gap(t.space.medium)
        .items_center()
        .justify_between()
        .child(design::page_header(
            "Schedules",
            "Keep your recurring prompts in one place. Run them when you’re ready.",
            cx,
        ))
        .child(
            h_flex()
                .items_center()
                .gap(t.space.small)
                .child(
                    design::button(
                        "schedule-create-toggle-btn",
                        if schedule_form_open {
                            "Cancel"
                        } else {
                            "Create schedule"
                        },
                        cx,
                    )
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.toggle_schedule_form(cx);
                    })),
                )
                .child(
                    design::button("back-to-chat-btn", "Back to chat", cx)
                        .ghost()
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.select_tab(sovra_core::NavTab::Chat, cx);
                        })),
                ),
        )
}

fn render_schedule_form(
    schedule_name_input: &Entity<InputState>,
    schedule_cadence_input: &Entity<InputState>,
    schedule_prompt_input: &Entity<InputState>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("schedule-create-form")
        .test_support()
        .w_full()
        .p(t.space.medium)
        .gap(t.space.small)
        .sovra_surface(Surface::Matte, cx)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("New schedule"),
        )
        .child(
            h_flex()
                .w_full()
                .gap(t.space.small)
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(schedule_name_input)
                            .id("schedule-name-input")
                            .aria_label("Schedule name"),
                    ),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(schedule_cadence_input)
                            .id("schedule-cadence-input")
                            .aria_label("Schedule cadence"),
                    ),
                ),
        )
        .child(
            Input::new(schedule_prompt_input)
                .id("schedule-prompt-input")
                .aria_label("Schedule prompt"),
        )
        .child(h_flex().w_full().justify_end().child(
            design::primary_button("schedule-save-btn", "Save schedule", cx).on_click(cx.listener(
                |shell, _, window, cx| {
                    shell.submit_new_schedule(window, cx);
                },
            )),
        ))
}

fn render_schedules_placeholder(cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("schedules-empty-placeholder")
        .test_support()
        .w_full()
        .p(t.space.large)
        .gap(t.space.small)
        .items_center()
        .sovra_surface(Surface::Matte, cx)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("No schedules yet"),
        )
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child(
                    "Save a prompt and a cadence, then run it from here. Background scheduling is not active yet.",
                ),
        )
        .child(
            design::primary_button("schedule-empty-create-btn", "Create schedule", cx).on_click(
                cx.listener(|shell, _, _, cx| {
                    shell.open_schedule_form(cx);
                }),
            ),
        )
}

fn render_stat_card(label: &str, value: &str, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .flex_1()
        .p(t.space.medium)
        .gap(t.space.compact)
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
    let schedule_id_delete = item.id.clone();
    let is_active = item.status == ScheduleStatus::Active;
    let status_color = if is_active {
        theme.success
    } else {
        theme.muted_foreground
    };

    h_flex()
        .id(("schedule-row", idx))
        .test_support()
        .w_full()
        .flex_wrap()
        .items_center()
        .justify_between()
        .p(t.space.medium)
        .sovra_surface(Surface::Matte, cx)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(t.space.compact)
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
                                .py(t.space.hair)
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
                    design::button(
                        ("schedule-toggle", idx),
                        if is_active { "Pause" } else { "Resume" },
                        cx,
                    )
                    .accessibility_label("Toggle schedule status")
                    .small()
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.toggle_schedule(&schedule_id_toggle, cx);
                    })),
                )
                .child(
                    design::button(("schedule-run", idx), "Run in chat", cx)
                        .accessibility_label("Run schedule now in chat")
                        .icon(IconName::Play)
                        .small()
                        .on_click(cx.listener(move |shell, _, window, cx| {
                            shell.run_schedule_in_chat(&schedule_id_run, window, cx);
                        })),
                )
                .child(
                    design::button(("schedule-delete", idx), "Delete", cx)
                        .accessibility_label("Delete schedule")
                        .ghost()
                        .small()
                        .px(t.space.small)
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.delete_schedule(&schedule_id_delete, cx);
                        })),
                ),
        )
}
