//! Your models: everything trained here or received from the group, with a way
//! to try each one, share it and find its file.

use super::{TrainingView, parts, words};
use crate::design::{self, FitsPane as _, SovraStyled as _, Surface, TextRole};
use gpui_kit::component::{Disableable as _, Selectable as _, h_flex, input::Input, v_flex};
use gpui_kit::{
    ClipboardItem, Context, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::training::{LibraryModel, ModelOrigin, TrainingCommand};

impl TrainingView {
    fn trial_panel(&self, model: &LibraryModel, cx: &mut Context<Self>) -> gpui_kit::Div {
        let t = design::tokens(cx);
        let trial = self
            .state
            .trial
            .as_ref()
            .filter(|trial| trial.model == model.card.id);
        let busy = self.pending || self.state.trying();
        let mut panel = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .child(parts::caption(
                "Type a beginning and the model continues it in the style it learned.",
                cx,
            ))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        Input::new(&self.try_prompt)
                            .id("training-try-prompt")
                            .aria_label("Beginning for the model to continue")
                            .disabled(busy)
                            .prefer_width(t.layout.card_min_width),
                    )
                    .child(
                        design::button("training-try-run", "Continue this text", cx)
                            .disabled(busy)
                            .on_click(cx.listener(|view, _, _, cx| view.run_trial(cx))),
                    ),
            );
        let written = match trial {
            Some(trial) if trial.running => Some("Writing…".to_owned()),
            Some(trial) => trial
                .output
                .as_ref()
                .map(|output| format!("{}{output}", trial.prompt)),
            None => None,
        };
        if let Some(written) = written {
            panel = panel.child(
                div()
                    .id("training-try-output")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(written.clone())
                    .w_full()
                    .min_w_0()
                    .max_h(t.training.sample_max_height)
                    .overflow_y_scroll()
                    .p(t.space.small)
                    .whitespace_normal()
                    .sovra_surface(Surface::Canvas, cx)
                    .sovra_text(TextRole::Reading, cx)
                    .child(written),
            );
        }
        panel
    }

    fn model_row(
        &self,
        index: usize,
        model: &LibraryModel,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let card = &model.card;
        let id = card.id;
        let here = card.origin == ModelOrigin::TrainedHere;
        let origin = if here {
            "Trained here".to_owned()
        } else {
            format!("From {}", card.trained_by)
        };
        let loss = card
            .check_loss_milli
            .or(card.loss_milli)
            .map(|loss| format!(" · loss {}", words::loss(loss)))
            .unwrap_or_default();
        let facts = format!(
            "{} model · {} values · {} · {} rounds{loss}",
            card.base.info().title,
            words::count(card.parameters),
            words::file_size(card.bytes),
            words::count(card.rounds.into())
        );
        let group = self.state.group.as_ref();
        let shared = group.is_some_and(|group| model.offered_to(group.id));
        let (delivered, total) = model.delivered();
        let delivery = if shared && total > 0 {
            Some(format!("On {delivered} of {total} computers in your group"))
        } else if shared {
            Some("Shared with your group. No other computers are in it yet.".to_owned())
        } else if !here {
            Some("Given to you by your group".to_owned())
        } else {
            None
        };
        let open = self.trying == Some(id);
        let mut row = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .p(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                div()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(card.name.clone()),
            )
            .child(parts::caption(origin, cx))
            .child(parts::caption(facts, cx));
        if let Some(delivery) = delivery {
            let waiting: Vec<&str> = model
                .delivery
                .iter()
                .filter(|delivery| !delivery.delivered)
                .map(|delivery| delivery.name.as_str())
                .collect();
            row = row.child(
                div()
                    .id(("training-model-delivery", index))
                    .test_support()
                    .role(Role::Status)
                    .aria_label(delivery.clone())
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .child(delivery),
            );
            if shared && !waiting.is_empty() {
                row = row.child(parts::caption(
                    format!(
                        "Still to arrive on: {}. It is sent when that computer is next online.",
                        waiting.join(", ")
                    ),
                    cx,
                ));
            }
        }
        let mut actions = h_flex().min_w_0().flex_wrap().gap(t.space.small).child(
            design::button(("training-model-try", index), "Try it", cx)
                .accessibility_label(format!("Try {}", card.name))
                .selected(open)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.trying = if view.trying == Some(id) {
                        None
                    } else {
                        Some(id)
                    };
                    cx.notify();
                })),
        );
        if here && group.is_some() && !shared {
            actions = actions.child(
                design::button(("training-model-share", index), "Share with my group", cx)
                    .accessibility_label(format!("Share {} with my group", card.name))
                    .disabled(self.pending)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.send(TrainingCommand::Share(id), cx);
                    })),
            );
        }
        let location = model.saved_at.clone();
        actions = actions.child(
            design::button(("training-model-copy", index), "Copy file location", cx)
                .accessibility_label(format!("Copy the file location of {}", card.name))
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(location.clone()));
                }),
        );
        row = row.child(actions);
        if open {
            row = row.child(self.trial_panel(model, cx));
        }
        row.id(("training-model", index)).test_support()
    }

    pub(super) fn library_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut card = parts::card(self.compact(), cx).child(parts::title("Your models", cx));
        if self.state.models.is_empty() {
            card = card.child(parts::status(
                "training-models-empty",
                "Models you train, and models your group gives you, appear here.",
                cx,
            ));
        } else {
            card = card.child(parts::caption(
                "Models are kept until you remove their files yourself. Trying a model only writes text. Nothing in a model is ever run as a program.",
                cx,
            ));
        }
        for (index, model) in self.state.models.iter().enumerate() {
            card = card.child(self.model_row(index, model, cx));
        }
        card.id("training-models").test_support()
    }
}
