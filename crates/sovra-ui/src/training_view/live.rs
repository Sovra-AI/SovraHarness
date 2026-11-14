//! The live view: progress, the loss chart, the numbers and what the model
//! writes right now. Everything here is the runtime's own measurement.

use super::{TrainingView, parts, words};
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, chart::AreaChart, h_flex, progress::Progress, v_flex,
};
use gpui_kit::{
    Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::training::{ChartPoint, TrainingCommand, TrainingPhase, guess_quality};

/// One point of the chart. Losses are shown as whole units, not thousandths.
#[derive(Clone)]
struct Row {
    round: SharedString,
    loss: f64,
    check: f64,
}

fn units(milli: u32) -> f64 {
    f64::from(milli) / 1000.
}

impl TrainingView {
    fn phase_words(&self) -> &'static str {
        match self.state.phase {
            TrainingPhase::Loading => "Opening saved training…",
            TrainingPhase::Ready => "Ready",
            TrainingPhase::Preparing => "Getting ready…",
            TrainingPhase::Running if self.state.sharing.is_some() => "Learning with your group",
            TrainingPhase::Running => "Learning",
            TrainingPhase::Pausing => "Finishing this round…",
            TrainingPhase::Paused => "Paused. Everything so far is saved",
            TrainingPhase::Complete => "Finished. Your model is in Your models",
            TrainingPhase::Unavailable => "Training is not available in this session",
        }
    }

    fn legend(label: &'static str, colour: Hsla, cx: &Context<Self>) -> gpui_kit::Div {
        let t = design::tokens(cx);
        h_flex()
            .min_w_0()
            .items_center()
            .gap(t.space.compact)
            .child(
                div()
                    .flex_shrink_0()
                    .w(t.training.legend_mark)
                    .h(t.training.legend_mark_height)
                    .rounded(t.radius.small)
                    .bg(colour),
            )
            .child(parts::caption(label, cx))
    }

    fn chart(&self, points: &[ChartPoint], cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let learning = cx.theme().chart_1;
        let checking = cx.theme().chart_2;
        let checked = points
            .last()
            .is_some_and(|point| point.check_loss_milli.is_some());
        let summary = match points.last() {
            Some(point) => match point.check_loss_milli {
                Some(check) => format!(
                    "Loss chart. At round {} the learning loss is {} and the check loss is {}.",
                    words::count(point.round.into()),
                    words::loss(point.loss_milli),
                    words::loss(check)
                ),
                None => format!(
                    "Loss chart. At round {} the learning loss is {}.",
                    words::count(point.round.into()),
                    words::loss(point.loss_milli)
                ),
            },
            None => "Loss chart. Nothing has been measured yet.".to_owned(),
        };
        let frame = div()
            .id("training-chart")
            .test_support()
            .role(Role::Image)
            .aria_label(summary)
            .w_full()
            .min_w_0()
            .h(if self.compact() {
                t.training.chart_compact_height
            } else {
                t.training.chart_height
            })
            .p(t.space.small)
            .sovra_surface(Surface::Subtle, cx);
        // Two points make a line; before that there is nothing honest to draw.
        if points.len() < 2 {
            return frame.child(parts::caption(
                "The chart appears after the first few rounds.",
                cx,
            ));
        }
        let rows: Vec<Row> = points
            .iter()
            .map(|point| Row {
                round: point.round.to_string().into(),
                loss: units(point.loss_milli),
                // Only read when every point carries a check value.
                check: units(point.check_loss_milli.unwrap_or(point.loss_milli)),
            })
            .collect();
        let mut chart = AreaChart::<Row, SharedString, f64>::new(rows)
            .id("training-loss-chart")
            .x(|row: &Row| row.round.clone())
            .y(|row: &Row| row.loss)
            .stroke(learning)
            .fill(learning.opacity(0.))
            .name("Learning");
        if checked {
            chart = chart
                .y(|row: &Row| row.check)
                .stroke(checking)
                .fill(checking.opacity(0.))
                .name("Check");
        }
        // The chart redraws with every snapshot, so it never replays its entrance.
        frame.child(chart.linear().x_tick_count(4).y_axis(true).appear(false))
    }

    pub(super) fn live_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let state = &self.state;
        let progress = &state.progress;
        let metrics = &state.metrics;
        let phase = state.phase;
        let name = state
            .plan
            .as_ref()
            .map_or("Your model", |plan| plan.name.as_str());
        let rounds = format!(
            "{} of {}",
            words::count(progress.rounds.into()),
            words::count(progress.total_rounds.into())
        );
        let status = if progress.total_rounds > 0 {
            format!("{} · round {rounds}.", self.phase_words())
        } else {
            format!("{}.", self.phase_words())
        };
        let points = metrics.chart();
        let latest = points.last();
        let loss = progress
            .loss_milli
            .or_else(|| latest.map(|point| point.loss_milli));
        let check = latest.and_then(|point| point.check_loss_milli);
        let checking = state
            .plan
            .as_ref()
            .is_none_or(|plan| plan.settings.check_share_percent > 0);
        let complete = phase == TrainingPhase::Complete;
        let speed = if metrics.tokens_per_second > 0 {
            format!(
                "{} letters a second",
                words::count(metrics.tokens_per_second.into())
            )
        } else if complete {
            "Done".to_owned()
        } else {
            "Measuring…".to_owned()
        };
        let left = match metrics.eta_ms {
            _ if complete => "Done".to_owned(),
            Some(ms) => words::duration(ms),
            None => "Measuring…".to_owned(),
        };
        let helpers = match (&state.sharing, &state.group) {
            (Some(_), Some(group)) => format!(
                "{} helping now, {} rounds given",
                group.helpers(),
                words::count(state.helped_rounds.into())
            ),
            _ if state.helped_rounds > 0 => {
                format!("{} rounds given", words::count(state.helped_rounds.into()))
            }
            _ => "This computer only".to_owned(),
        };
        let mut card = parts::card(self.compact(), cx)
            .child(parts::title(format!("Training {name}"), cx))
            .child(parts::status("training-status", status.clone(), cx))
            .child(
                Progress::new("training-progress")
                    .accessibility_label(status)
                    .loading(phase == TrainingPhase::Preparing)
                    .value(f32::from(progress.percent())),
            )
            .child(self.chart(&points, cx));
        let mut legend = h_flex()
            .min_w_0()
            .flex_wrap()
            .gap(t.space.medium)
            .child(Self::legend("Learning loss", cx.theme().chart_1, cx));
        if check.is_some() {
            legend = legend.child(Self::legend(
                "Check loss, on text it has not learned from",
                cx.theme().chart_2,
                cx,
            ));
        }
        card = card.child(legend).child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_stretch()
                .gap(t.space.small)
                .child(parts::tile("training-tile-round", "Round", rounds, cx))
                .child(parts::tile("training-tile-speed", "Speed", speed, cx))
                .child(parts::tile("training-tile-time", "Time left", left, cx))
                .child(parts::tile(
                    "training-tile-loss",
                    "Loss",
                    loss.map_or_else(|| "Not measured yet".to_owned(), words::loss),
                    cx,
                ))
                .child(parts::tile(
                    "training-tile-check",
                    "Check loss",
                    match check {
                        Some(check) => words::loss(check),
                        None if checking => "Not measured yet".to_owned(),
                        None => "Off".to_owned(),
                    },
                    cx,
                ))
                .child(parts::tile("training-tile-helpers", "Helpers", helpers, cx)),
        );
        card = card.child(parts::caption(
            match loss {
                Some(loss) => format!(
                    "Loss is how surprised the model is by each next letter, so lower is better. {} A check loss that stops falling means it has learned what this text can teach.",
                    guess_quality(loss)
                ),
                None => "Loss is how surprised the model is by each next letter, so lower is better.".to_owned(),
            },
            cx,
        ));
        let sample = metrics.sample.clone().filter(|sample| !sample.is_empty());
        let mut writing = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.compact)
            .child(parts::title("What it writes right now", cx));
        writing = match sample {
            Some(sample) => writing
                .child(
                    div()
                        .id("training-sample")
                        .test_support()
                        .role(Role::Status)
                        .aria_label(sample.clone())
                        .w_full()
                        .min_w_0()
                        .max_h(t.training.sample_max_height)
                        .overflow_y_scroll()
                        .p(t.space.small)
                        .whitespace_normal()
                        .sovra_surface(Surface::Subtle, cx)
                        .sovra_text(TextRole::Reading, cx)
                        .child(sample),
                )
                .child(parts::caption(
                    format!(
                        "Written at round {}. It starts as nonsense and comes closer to your text as it learns.",
                        words::count(metrics.sample_round.into())
                    ),
                    cx,
                )),
            None => writing.child(parts::status(
                "training-sample",
                "The first sample appears after the first check.",
                cx,
            )),
        };
        card = card.child(writing);
        if state.sharing.is_some() {
            card = card.child(parts::status(
                "training-sharing-status",
                format!(
                    "Your group can help with this training. {} rounds have come from other computers.",
                    words::count(state.helped_rounds.into())
                ),
                cx,
            ));
        }
        let mut actions = h_flex()
            .min_w_0()
            .flex_wrap()
            .gap(t.space.small)
            .child(
                design::button("training-pause", "Pause", cx)
                    .disabled(self.pending || phase != TrainingPhase::Running)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.send(TrainingCommand::Pause, cx);
                    })),
            )
            .child(
                design::button("training-resume", "Resume", cx)
                    .disabled(self.locked() || phase != TrainingPhase::Paused)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.send(TrainingCommand::Resume, cx);
                    })),
            )
            .child(
                design::button("training-stop", "Finish now and keep the model", cx)
                    .disabled(
                        self.pending
                            || !matches!(phase, TrainingPhase::Running | TrainingPhase::Paused),
                    )
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.send(TrainingCommand::Stop, cx);
                    })),
            );
        if state.sharing.is_some() {
            actions = actions.child(
                design::button("training-stop-sharing", "Carry on without my group", cx)
                    .disabled(self.pending)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.send(TrainingCommand::StopSharing, cx);
                    })),
            );
        }
        card.child(actions)
            .child(parts::caption(
                "Finishing early loses nothing: the model as it is now goes into Your models.",
                cx,
            ))
            .id("training-live")
            .test_support()
    }
}
