//! Advanced: every expert setting, closed until asked for. The presets fill all
//! of it in, so the simple path never opens this.

use super::{
    TrainingView,
    knobs::{Knob, Section},
    parts,
};
use crate::design::{self, FitsPane as _, SovraStyled as _, Surface, TextRole};
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, h_flex, input::NumberInput, v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    TestSupportExt as _, div,
};
use sovra_core::training::{LocalOptimizer, LrSchedule, TrainingCommand};

const SCHEDULES: [(LrSchedule, &str, &str); 3] = [
    (LrSchedule::Constant, "training-schedule-steady", "Steady"),
    (LrSchedule::Cosine, "training-schedule-ease", "Ease down"),
    (
        LrSchedule::WarmupCosine,
        "training-schedule-warmup",
        "Warm up, then ease down",
    ),
];

const OPTIMIZERS: [(LocalOptimizer, &str, &str); 2] = [
    (
        LocalOptimizer::Sgd,
        "training-optimizer-sgd",
        "Plain steps (SGD)",
    ),
    (
        LocalOptimizer::Adam,
        "training-optimizer-adam",
        "Adaptive steps (Adam)",
    ),
];

impl TrainingView {
    /// A setting's name and one-line hint, sized to wrap above its control in
    /// a narrow pane.
    fn setting_words(label: &'static str, hint: &'static str, cx: &Context<Self>) -> gpui_kit::Div {
        let t = design::tokens(cx);
        v_flex()
            .prefer_width(t.layout.card_min_width)
            .gap(t.space.hair)
            .child(
                div()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .child(label),
            )
            .child(parts::caption(hint, cx))
    }

    fn setting_row(cx: &Context<Self>) -> gpui_kit::Div {
        let t = design::tokens(cx);
        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .items_center()
            .gap(t.space.small)
    }

    fn knob_row(&self, knob: Knob, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut row =
            Self::setting_row(cx).child(Self::setting_words(knob.label(), knob.hint(), cx));
        if let Some(input) = self.knobs.get(knob.index()) {
            row = row.child(
                h_flex()
                    .id(knob.id())
                    .test_support()
                    .w(t.training.field_width)
                    .max_w_full()
                    .flex_shrink_0()
                    .child(
                        NumberInput::new(input)
                            .accessibility_label(knob.label())
                            .disabled(self.locked()),
                    ),
            );
        }
        row
    }

    fn schedule_row(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut choices = h_flex().min_w_0().flex_wrap().gap(t.space.compact);
        for (schedule, id, label) in SCHEDULES {
            let chosen = self.form.settings.schedule == schedule;
            choices = choices.child(
                design::button(id, label, cx)
                    .small()
                    .accessibility_label(if chosen {
                        format!("Step size over the run: {label}, chosen")
                    } else {
                        format!("Step size over the run: {label}")
                    })
                    .selected(chosen)
                    .disabled(self.locked())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        let optimizer = view.form.settings.optimizer;
                        view.settings_edited(schedule, optimizer, cx);
                    })),
            );
        }
        Self::setting_row(cx)
            .child(Self::setting_words(
                "Step size over the run",
                "Whether the round step size stays the same, eases down, or warms up first.",
                cx,
            ))
            .child(choices)
    }

    fn optimizer_row(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut choices = h_flex().min_w_0().flex_wrap().gap(t.space.compact);
        for (optimizer, id, label) in OPTIMIZERS {
            let chosen = self.form.settings.optimizer == optimizer;
            choices = choices.child(
                design::button(id, label, cx)
                    .small()
                    .accessibility_label(if chosen {
                        format!("Steps inside a packet: {label}, chosen")
                    } else {
                        format!("Steps inside a packet: {label}")
                    })
                    .selected(chosen)
                    .disabled(self.locked())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        let schedule = view.form.settings.schedule;
                        view.settings_edited(schedule, optimizer, cx);
                    })),
            );
        }
        Self::setting_row(cx)
            .child(Self::setting_words(
                "Steps inside a packet",
                "The rule a computer uses for its learning steps on one packet.",
                cx,
            ))
            .child(choices)
    }

    /// Helping one named computer right now, without the Groups permission.
    fn manual_help(&self, cx: &mut Context<Self>) -> Option<gpui_kit::Div> {
        let t = design::tokens(cx);
        let group = self.state.group.as_ref()?;
        if group.peers.is_empty() {
            return None;
        }
        let blocked = self.pending || self.state.busy();
        let mut section = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .child(parts::title("Help one computer now", cx))
            .child(parts::caption(
                "Helping normally starts by itself when Help the group train is on in Groups. Here you can help one computer's training straight away. It must have started with Train with my group.",
                cx,
            ));
        for (index, peer) in group.peers.iter().enumerate() {
            let device = peer.device;
            section = section.child(
                Self::setting_row(cx)
                    .child(parts::body(peer.name.clone(), cx).prefer_width(t.layout.card_min_width))
                    .child(
                        design::button(("training-help-peer", index), "Help now", cx)
                            .small()
                            .accessibility_label(format!("Help {} now", peer.name))
                            .disabled(blocked)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.send(TrainingCommand::Help(device), cx);
                            })),
                    ),
            );
        }
        Some(section)
    }

    pub(super) fn advanced_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let open = self.advanced_open;
        let mut card = parts::card(self.compact(), cx).child(
            h_flex()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(t.space.small)
                .child(
                    v_flex()
                        .prefer_width(t.layout.card_min_width)
                        .gap(t.space.hair)
                        .child(parts::title("Advanced", cx))
                        .child(parts::caption(
                            match self.form.preset {
                                Some(preset) => format!(
                                    "Using the recommended settings for {}. You never have to open this.",
                                    preset.title()
                                ),
                                None => "Using your own custom settings.".to_owned(),
                            },
                            cx,
                        )),
                )
                .child(
                    design::button(
                        "training-advanced-toggle",
                        if open {
                            "Hide advanced settings"
                        } else {
                            "Show advanced settings"
                        },
                        cx,
                    )
                    .selected(open)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.advanced_open = !view.advanced_open;
                        cx.notify();
                    })),
                ),
        );
        if !open {
            return card.id("training-advanced").test_support();
        }
        card = card.child(
            h_flex().min_w_0().flex_wrap().gap(t.space.small).child(
                design::button("training-reset", "Reset to recommended", cx)
                    .disabled(self.locked())
                    .on_click(cx.listener(|view, _, _, cx| view.reset_settings(cx))),
            ),
        );
        if let Some(problem) = self.problem()
            && !self.state.phase.busy()
        {
            card = card.child(parts::alert("training-advanced-problem", problem, cx));
        }
        for section in Section::ALL {
            let mut block = v_flex()
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                .p(t.space.small)
                .sovra_surface(Surface::Subtle, cx)
                .child(parts::title(section.title(), cx));
            for knob in Knob::ALL {
                if knob.section() != section {
                    continue;
                }
                block = block.child(self.knob_row(knob, cx));
                // The two named choices sit beside the numbers they go with.
                if knob == Knob::OuterLearningRate {
                    block = block.child(self.schedule_row(cx));
                }
                if knob == Knob::LearningRate {
                    block = block.child(self.optimizer_row(cx));
                }
            }
            card = card.child(block);
        }
        if let Some(help) = self.manual_help(cx) {
            card = card.child(help);
        }
        card.id("training-advanced").test_support()
    }
}
