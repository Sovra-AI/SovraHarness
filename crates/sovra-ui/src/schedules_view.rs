//! Schedules command center: plain-words create, live stats, and a swimlane of runs.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Bounds, Context, Entity, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, canvas, div,
    fill, point, size,
};
use sovra_core::{
    ScheduleItem, ScheduleStatus, TimelineMark, TimelineMarkKind, TimelineRange, WorkspaceState,
    timeline_marks,
};

use crate::design::{self, FitsPane as _, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScheduleFormStatus {
    Empty,
    Draft,
    Saving,
}

impl ScheduleFormStatus {
    fn saving(self) -> bool {
        self == Self::Saving
    }
    fn has_draft(self) -> bool {
        self != Self::Empty
    }
}

pub(crate) struct ScheduleForm<'a> {
    pub name: &'a Entity<InputState>,
    pub cadence: &'a Entity<InputState>,
    pub prompt: &'a Entity<InputState>,
    pub open: bool,
    pub status: ScheduleFormStatus,
    pub scroll: &'a gpui_kit::ScrollHandle,
    pub error: Option<&'a str>,
    pub compact: bool,
    pub range: TimelineRange,
    /// This computer is in a group and may add to it: offer "Share with my group".
    pub can_share: bool,
}

pub(crate) fn render_schedules_view(
    state: &WorkspaceState,
    form: &ScheduleForm<'_>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let stats = state.schedule_stats();
    let gap = if form.compact {
        t.space.medium
    } else {
        t.space.large
    };
    let pad = if form.compact {
        t.space.medium
    } else {
        t.space.large
    };

    let mut page = v_flex()
        .id("schedules-column")
        .test_support()
        .w_full()
        .min_w_0()
        .max_w(t.layout.page_width)
        .mx_auto()
        .gap(gap)
        .child(render_header(
            form.open,
            form.status.has_draft(),
            form.compact,
            cx,
        ));

    if !form.compact || !form.open {
        page = page.child(render_stats(stats, form.compact, cx));
    }

    if let Some(error) = form.error {
        page = page.child(
            div()
                .id("schedule-error")
                .test_support()
                .w_full()
                .min_w_0()
                .sovra_text(TextRole::Body, cx)
                .child(design::paragraph(error.to_owned())),
        );
    }
    if form.open {
        page = page.child(render_schedule_form(form, cx));
    }

    let show_empty = state.schedules.is_empty() && !form.open;
    let show_timeline = !state.schedules.is_empty() && (!form.compact || !form.open);
    let show_list = !state.schedules.is_empty();

    if show_empty {
        page = page.child(render_schedules_placeholder(form.status.has_draft(), cx));
    }
    if show_timeline {
        page = page.child(render_timeline(state, form.compact, form.range, cx));
    }
    if show_list {
        page = page.child(render_schedule_list(state, form.can_share, cx));
    }

    v_flex()
        .id("sovra-schedules-view")
        .test_support()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .track_scroll(form.scroll)
        .p(pad)
        .child(page)
}

fn render_header(
    schedule_form_open: bool,
    has_draft: bool,
    compact: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let title = if compact {
        div()
            .sovra_text(TextRole::PageTitle, cx)
            .child("Schedules")
            .into_any_element()
    } else {
        design::page_header(
            "Schedules",
            "Run a task once or repeatedly. Keep Sovra open and your computer awake. Pause a task anytime.",
            cx,
        )
        .into_any_element()
    };
    h_flex()
        .w_full()
        .flex_wrap()
        .gap(t.space.medium)
        .items_center()
        .justify_between()
        .child(title)
        .child(
            h_flex()
                .items_center()
                .gap(t.space.small)
                .child(
                    design::button(
                        "schedule-create-toggle-btn",
                        if schedule_form_open {
                            "Hide form"
                        } else if has_draft {
                            "Continue task"
                        } else {
                            "Create schedule"
                        },
                        cx,
                    )
                    .on_click(cx.listener(|shell, _, window, cx| {
                        shell.toggle_schedule_form(window, cx);
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
    form: &ScheduleForm<'_>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex().id("schedule-create-form").test_support().w_full().min_w_0()
        .max_w(t.layout.reading_width).mx_auto().p(t.space.medium).gap(t.space.small)
        .sovra_surface(Surface::Matte, cx)
        .child(div().sovra_text(TextRole::SectionTitle, cx).child("New scheduled task"))
        .child(div().id("schedule-name-label").test_support().sovra_text(TextRole::Caption, cx).child("Task name"))
        .child(Input::new(form.name).id("schedule-name-input").aria_label("Task name").disabled(form.status.saving()))
        .child(div().id("schedule-cadence-label").test_support().sovra_text(TextRole::Caption, cx).child("When should it run?"))
        .child(Input::new(form.cadence).id("schedule-cadence-input").aria_label("When should it run?").disabled(form.status.saving()))
        .child(div().id("schedule-cadence-help").test_support().sovra_text(TextRole::Caption, cx)
            .text_color(t.text_secondary).child("Try: every weekday at 9, every 4 hours, or in 1 minute. Leave blank for every day at 9:00."))
        .child(div().id("schedule-prompt-label").test_support().sovra_text(TextRole::Caption, cx).child("What should Sovra do?"))
        .child(Input::new(form.prompt).id("schedule-prompt-input").aria_label("What should Sovra do?").disabled(form.status.saving()))
        .child(div().sovra_text(TextRole::Caption, cx).text_color(t.text_secondary)
            .child("Keep Sovra open and your computer awake for the task to run."))
        .child(h_flex().w_full().flex_wrap().justify_end().child(
            design::primary_button("schedule-save-btn", if form.status.saving() { "Saving…" } else { "Save schedule" }, cx)
                .disabled(form.status.saving()).on_click(cx.listener(|shell, _, _, cx| shell.submit_new_schedule(cx)))
        ))
}

fn render_schedule_list(
    state: &WorkspaceState,
    can_share: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("schedules-list")
        .test_support()
        .gap(t.space.small)
        .children(
            state
                .schedules
                .iter()
                .enumerate()
                .map(|(idx, item)| render_schedule_row(state, item, idx, can_share, cx)),
        )
}

fn render_schedules_placeholder(
    has_draft: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
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
                .child("No repeating tasks yet"),
        )
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child(
                    "Tell Sovra what to do and when, in ordinary words — like in 1 minute, just once, or every weekday at 9. It will run in the background, and you can pause it whenever you like.",
                ),
        )
        .child(
            design::primary_button("schedule-empty-create-btn", if has_draft { "Continue task" } else { "Create schedule" }, cx).on_click(
                cx.listener(|shell, _, window, cx| {
                    shell.open_schedule_form(window, cx);
                }),
            ),
        )
}

fn render_stats(
    stats: sovra_core::ScheduleStats,
    compact: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let on_now = format!("{} of {}", stats.active_schedules, stats.total_schedules);
    let ran = stats.runs_today.to_string();
    let rate = format!("{}%", stats.success_rate_pct);
    if compact {
        return h_flex()
            .w_full()
            .flex_wrap()
            .gap(t.space.medium)
            .child(render_stat_chip("On now", &on_now, cx))
            .child(render_stat_chip("Ran today", &ran, cx))
            .child(render_stat_chip("Last 30 days", &rate, cx))
            .into_any_element();
    }
    h_flex()
        .w_full()
        .flex_wrap()
        .gap(t.space.small)
        .child(render_stat_card("On now", &on_now, cx))
        .child(render_stat_card("Ran today", &ran, cx))
        .child(render_stat_card("Finished in the last 30 days", &rate, cx))
        .into_any_element()
}

fn render_stat_chip(label: &str, value: &str, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .gap(t.space.compact)
        .items_baseline()
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(label.to_owned()),
        )
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(value.to_owned()),
        )
}

fn render_stat_card(label: &str, value: &str, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .prefer_width(t.layout.card_min_width)
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
                .sovra_text(TextRole::SectionTitle, cx)
                .child(value.to_owned()),
        )
}

fn render_timeline(
    state: &WorkspaceState,
    compact: bool,
    range: TimelineRange,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();
    let book = schedule_book(state);
    let marks = timeline_marks(&book, state.now(), range);
    #[expect(
        clippy::cast_precision_loss,
        reason = "lane count is a handful of rows"
    )]
    let lanes = state.schedules.len().max(1) as f32;
    let content_h = t.graph.timeline_lane * lanes + t.space.medium * 2.;
    let min_h = if compact {
        t.graph.timeline_compact_height
    } else {
        t.graph.timeline_height
    };
    let scene = TimelineScene {
        backdrop: t.graph.backdrop,
        grid: t.graph.grid,
        now: t.accent.amber,
        success: t.accent.green,
        failed: t.accent.red,
        skipped: t.accent.slate,
        inflight: t.accent.amber,
        next: t.accent.violet,
        hairline: t.layout.hairline,
        ring: t.graph.ring,
        label_width: t.graph.timeline_label_width,
        lane: t.graph.timeline_lane,
        dot: t.graph.timeline_dot,
        pad: t.space.medium,
        names: state
            .schedules
            .iter()
            .map(|item| item.name.clone())
            .collect(),
        marks: marks
            .iter()
            .filter_map(|mark| {
                let lane = state
                    .schedules
                    .iter()
                    .position(|item| item.id == mark.schedule_id)?;
                Some(PlacedMark {
                    lane,
                    kind: mark.kind,
                    at: unit(mark.at.0, state.now().0, range.duration_ms()),
                })
            })
            .collect(),
    };
    let drawing = canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window, _| {
            paint_timeline(bounds, &scene, window);
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full();

    let pad = if compact {
        t.space.small
    } else {
        t.space.medium
    };
    let mut chart = v_flex()
        .id("schedules-timeline")
        .test_support()
        .w_full()
        .gap(t.space.small)
        .p(pad)
        .sovra_surface(Surface::Matte, cx)
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .gap(t.space.small)
                .items_center()
                .justify_between()
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("When they run"),
                )
                .child(
                    h_flex()
                        .id("schedules-range")
                        .test_support()
                        .gap(t.space.compact)
                        .flex_wrap()
                        .children(TimelineRange::ALL.iter().copied().enumerate().map(
                            |(idx, option)| {
                                let selected = option == range;
                                let button =
                                    design::button(("schedule-range", idx), option.label(), cx)
                                        .small();
                                let button = if selected { button.primary() } else { button };
                                button.on_click(cx.listener(move |shell, _, _, cx| {
                                    shell.set_schedule_timeline_range(option, cx);
                                }))
                            },
                        )),
                ),
        );
    if !compact {
        chart = chart.child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child("Each row is one task. Dots on the left already ran. Rings on the right are next. The line in the middle is now."),
        );
    }
    chart = chart.child(
        div()
            .relative()
            .w_full()
            .min_w_0()
            .min_h(min_h)
            .h(content_h)
            .overflow_hidden()
            .rounded(t.radius.card)
            .border_1()
            .border_color(cx.theme().border)
            .bg(t.graph.backdrop)
            .child(drawing)
            .child(
                h_flex()
                    .absolute()
                    .top_0()
                    .left(t.graph.timeline_label_width)
                    .right_0()
                    .h_full()
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("schedules-now-marker")
                            .test_support()
                            .aria_label("Now")
                            .w(t.layout.hairline)
                            .h_full()
                            .bg(t.accent.amber),
                    )
                    .child(div().flex_1()),
            )
            .child(
                v_flex()
                    .absolute()
                    .top(t.space.small)
                    .left(t.space.small)
                    .w(t.graph.timeline_label_width)
                    .children(state.schedules.iter().enumerate().map(|(idx, item)| {
                        div()
                            .id(("schedule-lane", idx))
                            .test_support()
                            .h(t.graph.timeline_lane)
                            .px(t.space.compact)
                            .flex()
                            .items_center()
                            .min_w_0()
                            .child(
                                div()
                                    .sovra_text(TextRole::Caption, cx)
                                    .text_color(theme.foreground)
                                    .truncate()
                                    .child(item.name.clone()),
                            )
                    })),
            ),
    );
    if compact {
        chart
    } else {
        chart
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .px(t.graph.timeline_label_width)
                    .child(
                        div()
                            .sovra_text(TextRole::Caption, cx)
                            .text_color(t.text_secondary)
                            .child("Earlier"),
                    )
                    .child(
                        div()
                            .sovra_text(TextRole::Caption, cx)
                            .text_color(t.text_secondary)
                            .child("Now"),
                    )
                    .child(
                        div()
                            .sovra_text(TextRole::Caption, cx)
                            .text_color(t.text_secondary)
                            .child("Later"),
                    ),
            )
            .child(render_legend(&marks, cx))
    }
}

fn render_legend(marks: &[TimelineMark], cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .id("schedules-timeline-legend")
        .test_support()
        .w_full()
        .flex_wrap()
        .gap(t.space.small)
        .items_center()
        .children(marks.iter().enumerate().map(|(idx, mark)| {
            let (fill_color, ring) = mark_ink(mark.kind, cx);
            let label = match mark.kind {
                TimelineMarkKind::Succeeded => "Finished",
                TimelineMarkKind::Failed => "Didn't finish",
                TimelineMarkKind::Skipped => "Skipped",
                TimelineMarkKind::InFlight => "Running now",
                TimelineMarkKind::Next => "Coming up",
            };
            h_flex()
                .id(("schedule-run-mark", idx))
                .test_support()
                .aria_label(format!("{} · {label}", mark.schedule_name))
                .items_center()
                .gap(t.space.compact)
                .child(if ring {
                    div()
                        .w(t.graph.timeline_dot)
                        .h(t.graph.timeline_dot)
                        .rounded_full()
                        .bg(t.graph.backdrop)
                        .border_1()
                        .border_color(fill_color)
                } else {
                    div()
                        .w(t.graph.timeline_dot)
                        .h(t.graph.timeline_dot)
                        .rounded_full()
                        .bg(fill_color)
                })
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(format!("{} · {label}", mark.schedule_name)),
                )
        }))
}

fn mark_ink(kind: TimelineMarkKind, cx: &Context<Shell>) -> (Hsla, bool) {
    let t = design::tokens(cx);
    match kind {
        TimelineMarkKind::Succeeded => (t.accent.green, false),
        TimelineMarkKind::Failed => (t.accent.red, false),
        TimelineMarkKind::Skipped => (t.accent.slate, false),
        TimelineMarkKind::InFlight => (t.accent.amber, false),
        TimelineMarkKind::Next => (t.accent.violet, true),
    }
}

fn render_schedule_row(
    state: &WorkspaceState,
    item: &ScheduleItem,
    idx: usize,
    can_share: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    // Schedules that send messages on someone's behalf are never shared.
    let shared = (can_share && item.send.is_none()).then(|| {
        let template = sovra_core::group::Template::Schedule {
            name: item.name.clone(),
            cadence: item.cadence_label(),
            prompt: item.prompt.clone(),
        };
        design::button(("schedule-share", idx), "Share with my group", cx)
            .accessibility_label("Share this schedule with your group")
            .ghost()
            .small()
            .on_click(move |_, _, cx| {
                let _ = cx
                    .global::<crate::RuntimeClient>()
                    .0
                    .send(sovra_core::Command::Group(
                        sovra_core::group::GroupCommand::ShareTemplate {
                            template: template.clone(),
                            with: None,
                        },
                    ));
            })
    });
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
    let cadence = item.cadence_label();
    let next = item.next_run_label(state.now());
    let last = item.last_result_label(&state.schedule_runs, state.now());

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
                        .child(format!("{cadence} · Next: {next} · {last}")),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .gap(t.space.small)
                .flex_wrap()
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
                .children(shared)
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

fn schedule_book(state: &WorkspaceState) -> sovra_core::ScheduleBook {
    sovra_core::ScheduleBook {
        next_seq: 0,
        schedules: state.schedules.clone(),
        runs: state.schedule_runs.clone(),
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "timeline placement is visual, not a clock"
)]
fn unit(at: i64, now: i64, span: i64) -> f32 {
    if span <= 0 {
        return 0.5;
    }
    let start = now.saturating_sub(span);
    let domain = span.saturating_mul(2);
    if domain <= 0 {
        return 0.5;
    }
    ((at.saturating_sub(start) as f64) / (domain as f64)).clamp(0.0, 1.0) as f32
}

#[derive(Clone)]
struct PlacedMark {
    lane: usize,
    kind: TimelineMarkKind,
    at: f32,
}

#[derive(Clone)]
struct TimelineScene {
    backdrop: Hsla,
    grid: Hsla,
    now: Hsla,
    success: Hsla,
    failed: Hsla,
    skipped: Hsla,
    inflight: Hsla,
    next: Hsla,
    hairline: Pixels,
    ring: Pixels,
    label_width: gpui_kit::Rems,
    lane: gpui_kit::Rems,
    dot: gpui_kit::Rems,
    pad: gpui_kit::Rems,
    names: Vec<String>,
    marks: Vec<PlacedMark>,
}

fn paint_timeline(bounds: Bounds<Pixels>, scene: &TimelineScene, window: &mut Window) {
    window.paint_quad(fill(bounds, scene.backdrop));
    let rem = window.rem_size();
    let label_w = scene.label_width.to_pixels(rem);
    let lane_h = scene.lane.to_pixels(rem);
    let pad = scene.pad.to_pixels(rem);
    let dot = scene.dot.to_pixels(rem);
    let track_left = bounds.origin.x + label_w;
    let track_width = (bounds.size.width - label_w).max(Pixels::ZERO);
    let track_top = bounds.origin.y + pad;
    let now_x = track_left + track_width * 0.5;
    window.paint_quad(fill(
        Bounds::new(
            point(now_x, bounds.origin.y),
            size(scene.hairline, bounds.size.height),
        ),
        scene.now,
    ));
    for index in 0..scene.names.len() {
        #[expect(
            clippy::cast_precision_loss,
            reason = "lane index is a handful of rows"
        )]
        let y = track_top + lane_h * (index as f32) + lane_h * 0.5;
        window.paint_quad(fill(
            Bounds::new(point(track_left, y), size(track_width, scene.hairline)),
            scene.grid,
        ));
    }
    for mark in &scene.marks {
        #[expect(
            clippy::cast_precision_loss,
            reason = "lane index is a handful of rows"
        )]
        let y = track_top + lane_h * (mark.lane as f32) + lane_h * 0.5;
        let x = track_left + track_width * mark.at;
        let square = Bounds::new(point(x - dot * 0.5, y - dot * 0.5), size(dot, dot));
        let color = match mark.kind {
            TimelineMarkKind::Succeeded => scene.success,
            TimelineMarkKind::Failed => scene.failed,
            TimelineMarkKind::Skipped => scene.skipped,
            TimelineMarkKind::InFlight => scene.inflight,
            TimelineMarkKind::Next => scene.next,
        };
        if mark.kind == TimelineMarkKind::Next {
            window.paint_quad(
                fill(square, scene.backdrop)
                    .border_widths(scene.ring)
                    .border_color(color)
                    .corner_radii(dot * 0.5),
            );
        } else {
            window.paint_quad(fill(square, color).corner_radii(dot * 0.5));
        }
    }
}
