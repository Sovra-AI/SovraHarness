//! The choices for a run: the starting point, how thorough to be, the group,
//! the name and the one Start button.

use super::{TrainingView, facts, parts, words};
use crate::design::{self, FitsPane as _, SovraStyled as _, Surface, TextRole};
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, button::ButtonVariants as _, h_flex,
    input::Input, switch::Switch, v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::training::{BASE_MODELS, BaseModel, ModelOrigin, TrainingBase, TrainingPreset};

/// Continue-from rows shown before "Show all".
const CONTINUE_PREVIEW: usize = 3;

fn base_id(model: BaseModel) -> &'static str {
    match model {
        BaseModel::Starter => "training-base-starter",
        BaseModel::Small => "training-base-small",
        BaseModel::Medium => "training-base-medium",
        BaseModel::Large => "training-base-large",
    }
}

fn preset_id(preset: TrainingPreset) -> &'static str {
    match preset {
        TrainingPreset::Quick => "training-preset-quick",
        TrainingPreset::Balanced => "training-preset-balanced",
        TrainingPreset::Thorough => "training-preset-thorough",
    }
}

fn capitalised(text: &str) -> String {
    let mut letters = text.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

impl TrainingView {
    pub(super) fn base_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let locked = self.locked();
        let mut card = parts::card(self.compact(), cx)
            .child(parts::title("Starting point", cx))
            .child(parts::body(
                "Pick the size of model to teach. Bigger models learn more and take longer.",
                cx,
            ));
        let option = |name: SharedString, lines: Vec<String>, button: gpui_kit::AnyElement| {
            let mut words = v_flex()
                .prefer_width(t.layout.card_min_width)
                .gap(t.space.hair)
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Body, cx)
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(name),
                );
            for line in lines {
                words = words.child(parts::caption(line, cx));
            }
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap(t.space.small)
                .p(t.space.small)
                .sovra_surface(Surface::Subtle, cx)
                .child(words)
                .child(h_flex().flex_shrink_0().max_w_full().child(button))
        };
        for row in BASE_MODELS {
            let model = row.model;
            let chosen = self.form.base == TrainingBase::Fresh(model);
            let architecture = row.architecture;
            let mut lines = vec![
                row.summary.to_owned(),
                format!(
                    "{} values to learn, saved as {}.",
                    words::count(architecture.parameters() as u64),
                    words::file_size(architecture.file_len() as u64)
                ),
            ];
            if let Some(hardware) = &self.state.hardware {
                lines.push(if hardware.recommended_model() == model {
                    "Recommended for this computer.".to_owned()
                } else if hardware.comfortable(model) {
                    "Comfortable on this computer.".to_owned()
                } else {
                    "Likely to be slow on this computer.".to_owned()
                });
            }
            let button =
                design::button(base_id(model), if chosen { "Chosen" } else { "Choose" }, cx)
                    .accessibility_label(if chosen {
                        format!("{}: chosen", row.title)
                    } else {
                        format!("Choose {}", row.title)
                    })
                    .selected(chosen)
                    .disabled(locked)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.choose_base(TrainingBase::Fresh(model), cx);
                    }))
                    .into_any_element();
            card = card.child(option(row.title.into(), lines, button));
        }
        let models = &self.state.models;
        if !models.is_empty() {
            card = card.child(parts::caption(
                "Or teach one of your models some more. It keeps its size.",
                cx,
            ));
            let shown = if self.all_models_listed {
                models.len()
            } else {
                CONTINUE_PREVIEW
            };
            for (index, model) in models.iter().enumerate().take(shown) {
                let id = model.card.id;
                let chosen = self.form.base == TrainingBase::Continue(id);
                let origin = match model.card.origin {
                    ModelOrigin::TrainedHere => "trained here".to_owned(),
                    ModelOrigin::Received { .. } => format!("from {}", model.card.trained_by),
                };
                let lines = vec![format!(
                    "{} model, {} rounds so far, {origin}.",
                    model.card.base.info().title,
                    words::count(model.card.rounds.into())
                )];
                let button = design::button(
                    ("training-base-continue", index),
                    if chosen { "Chosen" } else { "Continue this" },
                    cx,
                )
                .accessibility_label(if chosen {
                    format!("Continue from {}: chosen", model.card.name)
                } else {
                    format!("Continue from {}", model.card.name)
                })
                .selected(chosen)
                .disabled(locked)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.choose_base(TrainingBase::Continue(id), cx);
                }))
                .into_any_element();
                card = card.child(option(
                    format!("Continue from {}", model.card.name).into(),
                    lines,
                    button,
                ));
            }
            if models.len() > CONTINUE_PREVIEW {
                card = card.child(
                    h_flex().min_w_0().flex_wrap().child(
                        design::button(
                            "training-base-more",
                            if self.all_models_listed {
                                "Show fewer".to_owned()
                            } else {
                                format!("Show all {} models", models.len())
                            },
                            cx,
                        )
                        .small()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.all_models_listed = !view.all_models_listed;
                            cx.notify();
                        })),
                    ),
                );
            }
        }
        card.id("training-base").test_support()
    }

    pub(super) fn preset_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let locked = self.locked();
        let facts = facts(&self.state, &self.unknown_hardware, self.pasted);
        let with_group = self.form.with_group(facts);
        let architecture = self.form.architecture(&self.state.models);
        let mut options = h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .items_stretch()
            .gap(t.space.small);
        for preset in TrainingPreset::ALL {
            let chosen = self.form.preset == Some(preset);
            let estimate = match (
                &self.state.hardware,
                architecture,
                self.form.recommended(preset, facts),
            ) {
                (Some(hardware), Some(architecture), Some(settings)) => {
                    let time = capitalised(&words::duration(
                        hardware.rough_duration_ms(architecture, &settings),
                    ));
                    if with_group {
                        format!("{time} on this computer, less with your group helping.")
                    } else {
                        format!("{time} on this computer.")
                    }
                }
                _ => "The time appears once this computer has been checked.".to_owned(),
            };
            options = options.child(
                v_flex()
                    .prefer_width(t.layout.card_min_width)
                    .gap(t.space.compact)
                    .p(t.space.small)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(
                        h_flex().min_w_0().flex_wrap().child(
                            design::button(preset_id(preset), preset.title(), cx)
                                .accessibility_label(if chosen {
                                    format!("{}: chosen", preset.title())
                                } else {
                                    format!("Choose {}", preset.title())
                                })
                                .selected(chosen)
                                .disabled(locked)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.choose_preset(preset, cx);
                                })),
                        ),
                    )
                    .child(parts::body(preset.summary(), cx))
                    .child(parts::caption(estimate, cx)),
            );
        }
        let mut card = parts::card(self.compact(), cx)
            .child(parts::title("How thorough?", cx))
            .child(options);
        if self.form.preset.is_none() {
            card = card.child(parts::status(
                "training-preset-custom",
                "Custom: your own Advanced settings are in use. Choose one above to go back to recommended ones.",
                cx,
            ));
        }
        card.id("training-presets").test_support()
    }

    fn group_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut section = v_flex().w_full().min_w_0().gap(t.space.small);
        let Some(group) = &self.state.group else {
            return section.child(parts::caption(
                "Create or join a group in Groups to train together and give everyone the finished model.",
                cx,
            ));
        };
        let capable = self.form.group_capable(&self.state.models);
        let on = self
            .form
            .with_group(facts(&self.state, &self.unknown_hardware, self.pasted));
        section = section.child(
            h_flex().min_w_0().flex_wrap().child(
                Switch::new("training-group-switch")
                    .checked(on)
                    .label("Train with my group")
                    .accessibility_label(if on {
                        "Train with my group: on"
                    } else {
                        "Train with my group: off"
                    })
                    .disabled(self.locked() || !capable)
                    .on_click(cx.listener(|view, checked: &bool, _, cx| {
                        view.form.group_choice = Some(*checked);
                        cx.notify();
                    })),
            ),
        );
        section = section.child(parts::body(
            if capable {
                format!(
                    "Computers in {} that have turned on Help the group train will join in automatically, and everyone in the group gets the finished model. To help, they are sent pieces of the text you chose.",
                    group.name
                )
            } else {
                "This model is too large for other computers to help train, so it trains on this computer. When it is done, press Share with my group in Your models to give it to everyone.".to_owned()
            },
            cx,
        ));
        let mut members = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.hair)
            .p(t.space.small)
            .sovra_surface(Surface::Subtle, cx);
        let helping = group.helpers();
        let summary = if group.peers.is_empty() {
            format!(
                "No other computers are in {} yet. Add one in Groups.",
                group.name
            )
        } else {
            format!(
                "{} other {} in {}, {} helping now.",
                group.peers.len(),
                if group.peers.len() == 1 {
                    "computer"
                } else {
                    "computers"
                },
                group.name,
                helping
            )
        };
        members = members.child(parts::caption(summary.clone(), cx));
        for peer in &group.peers {
            let mut facts = vec![if peer.online { "online" } else { "away" }.to_owned()];
            if peer.helping {
                facts.push("helping now".to_owned());
            }
            if peer.rounds > 0 {
                facts.push(format!("{} rounds given", words::count(peer.rounds.into())));
            }
            members = members.child(parts::body(
                format!("{} · {}", peer.name, facts.join(" · ")),
                cx,
            ));
        }
        section.child(
            members
                .id("training-group-members")
                .test_support()
                .role(Role::Status)
                .aria_label(summary),
        )
    }

    pub(super) fn start_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let locked = self.locked();
        let problem = self.problem();
        let has_run = self.state.plan.is_some();
        let mut card = parts::card(self.compact(), cx)
            .child(parts::title("Start", cx))
            .child(self.group_section(cx))
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.compact)
                    .child(parts::caption("Name for your model", cx))
                    .child(
                        Input::new(&self.name)
                            .id("training-name")
                            .aria_label("Name for your model")
                            .disabled(locked)
                            .w_full(),
                    ),
            );
        if let Some(problem) = &problem
            && !self.state.phase.busy()
        {
            card = card.child(parts::alert("training-plan-problem", problem.clone(), cx));
        }
        card.child(
            h_flex().min_w_0().flex_wrap().gap(t.space.small).child(
                design::button(
                    "training-start",
                    if has_run {
                        "Start new training"
                    } else {
                        "Start training"
                    },
                    cx,
                )
                .primary()
                .disabled(locked || problem.is_some())
                .on_click(cx.listener(|view, _, _, cx| view.start(cx))),
            ),
        )
        .child(parts::caption(
            "You can pause whenever you like. Progress is saved as it goes, and training waits while you chat. Starting again keeps the model from the run before.",
            cx,
        ))
        .id("training-start-card")
        .test_support()
    }
}
