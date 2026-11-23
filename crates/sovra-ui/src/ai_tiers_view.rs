//! My AIs: how long the chat AI thinks, and a different AI for simple, medium and
//! complex questions, sorted by the quick rules or by Laya on this computer.
//!
//! State lives in the runtime (`ModelPreferences::tier_routing`); this view only
//! shows it and sends [`ModelCommand::SetTierRouting`] when a picker changes.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{SearchableVec, SelectEvent, SelectState},
    switch::Switch,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    Window, div,
};
use sovra_core::ai_tiers::{
    DecisionSource, LayaStatus, Sorter, ThinkingLevel, TierRouting, tier_examples, tier_title,
};
use sovra_core::models::{ModelCommand, ModelKey};
use sovra_core::providers::ModelTier;

use crate::design::{self, FitsPane as _, SovraStyled as _, Surface, TextRole};
use crate::models_view::{Choice, ModelsView, Selector, caption, select_control};

/// Separates connection and model IDs inside one picker value. Neither may
/// contain control characters, so this cannot collide.
const KEY_SEPARATOR: char = '\u{1f}';
/// Picker value for "use my chat AI".
const CHAT_MODEL: &str = "";

fn encode(key: &ModelKey) -> String {
    format!("{}{KEY_SEPARATOR}{}", key.connection_id, key.model_id)
}

fn decode(value: &str) -> Option<ModelKey> {
    let (connection_id, model_id) = value.split_once(KEY_SEPARATOR)?;
    Some(ModelKey {
        connection_id: connection_id.to_owned(),
        model_id: model_id.to_owned(),
    })
}

fn thinking_value(level: ThinkingLevel) -> String {
    format!("{level:?}")
}

fn thinking_from(value: &str) -> Option<ThinkingLevel> {
    ThinkingLevel::ALL
        .into_iter()
        .find(|level| thinking_value(*level) == value)
}

fn thinking_choices() -> Vec<Choice> {
    ThinkingLevel::ALL
        .into_iter()
        .map(|level| Choice::new(&thinking_value(level), level.label()))
        .collect()
}

fn tier_index(tier: ModelTier) -> usize {
    tier.as_u8().into()
}

/// Accessible names double as element IDs, so they are fixed per tier.
fn model_label(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Fast => "AI for simple questions",
        ModelTier::Standard => "AI for medium questions",
        ModelTier::Deep => "AI for complex questions",
    }
}

fn thinking_label(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Fast => "Thinking for simple questions",
        ModelTier::Standard => "Thinking for medium questions",
        ModelTier::Deep => "Thinking for complex questions",
    }
}

fn new_selector(
    choices: Vec<Choice>,
    window: &mut Window,
    cx: &mut Context<AiTiersView>,
) -> Entity<Selector> {
    cx.new(|cx| SelectState::new(SearchableVec::new(choices), None, window, cx).searchable(true))
}

pub(crate) struct AiTiersView {
    models: Entity<ModelsView>,
    chat_thinking: Entity<Selector>,
    tier_models: [Entity<Selector>; 3],
    tier_thinking: [Entity<Selector>; 3],
    test_input: Entity<InputState>,
    /// What the pickers last showed, to refresh them only when it changes.
    synced: Option<(TierRouting, Vec<(ModelKey, String)>)>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for AiTiersView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiTiersView").finish_non_exhaustive()
    }
}

impl AiTiersView {
    pub(crate) fn new(
        models: Entity<ModelsView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let chat_thinking = new_selector(thinking_choices(), window, cx);
        let tier_models = [(); 3].map(|()| new_selector(Vec::new(), window, cx));
        let tier_thinking = [(); 3].map(|()| new_selector(thinking_choices(), window, cx));
        let test_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Type a question to see how it is sorted")
        });
        let mut subscriptions = vec![
            cx.observe_in(&models, window, |view, _, window, cx| view.sync(window, cx)),
            cx.subscribe_in(
                &chat_thinking,
                window,
                |view, _, event: &SelectEvent<SearchableVec<Choice>>, _, cx| {
                    let SelectEvent::Confirm(Some(value)) = event else {
                        return;
                    };
                    if let Some(level) = thinking_from(value) {
                        view.update_routing(cx, |routing| routing.chat_thinking = level);
                    }
                },
            ),
            cx.subscribe_in(&test_input, window, |view, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.test(cx);
                }
            }),
        ];
        for tier in ModelTier::ALL {
            let index = tier_index(tier);
            subscriptions.push(cx.subscribe_in(
                &tier_models[index],
                window,
                move |view, _, event: &SelectEvent<SearchableVec<Choice>>, _, cx| {
                    let SelectEvent::Confirm(Some(value)) = event else {
                        return;
                    };
                    let model = decode(value);
                    view.update_routing(cx, |routing| {
                        routing
                            .tiers
                            .entry(tier)
                            .or_insert_with(|| routing_default(tier))
                            .model = model;
                    });
                },
            ));
            subscriptions.push(cx.subscribe_in(
                &tier_thinking[index],
                window,
                move |view, _, event: &SelectEvent<SearchableVec<Choice>>, _, cx| {
                    let SelectEvent::Confirm(Some(value)) = event else {
                        return;
                    };
                    if let Some(level) = thinking_from(value) {
                        view.update_routing(cx, |routing| {
                            routing
                                .tiers
                                .entry(tier)
                                .or_insert_with(|| routing_default(tier))
                                .thinking = level;
                        });
                    }
                },
            ));
        }
        let mut view = Self {
            models,
            chat_thinking,
            tier_models,
            tier_thinking,
            test_input,
            synced: None,
            _subscriptions: subscriptions,
        };
        view.sync(window, cx);
        view
    }

    fn routing(&self, cx: &gpui_kit::App) -> TierRouting {
        self.models
            .read(cx)
            .state()
            .preferences
            .tier_routing
            .clone()
    }

    fn send(&self, command: ModelCommand, cx: &mut Context<Self>) {
        self.models
            .update(cx, |models, cx| models.send(command, cx));
    }

    fn update_routing(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut TierRouting)) {
        let mut routing = self.routing(cx);
        change(&mut routing);
        if routing != self.routing(cx) {
            self.send(ModelCommand::SetTierRouting { routing }, cx);
        }
    }

    fn test(&mut self, cx: &mut Context<Self>) {
        let prompt = self.test_input.read(cx).value().trim().to_owned();
        if !prompt.is_empty() {
            self.send(ModelCommand::TestTierDecision { prompt }, cx);
        }
    }

    /// Refreshes picker items and selections from the latest saved routing.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let routing = self.routing(cx);
        let ready: Vec<(ModelKey, String)> = self
            .models
            .read(cx)
            .switchable_chat_models()
            .into_iter()
            .map(|model| (model.key, model.name))
            .collect();
        let snapshot = (routing.clone(), ready.clone());
        if self.synced.as_ref() == Some(&snapshot) {
            return;
        }
        let items_changed = self
            .synced
            .as_ref()
            .is_none_or(|(_, before)| *before != ready);
        self.synced = Some(snapshot);
        let mut items = vec![Choice::new(CHAT_MODEL, "Use my chat AI")];
        items.extend(
            ready
                .iter()
                .map(|(key, name)| Choice::new(&encode(key), name)),
        );
        self.chat_thinking.update(cx, |select, cx| {
            select.set_selected_value(&thinking_value(routing.chat_thinking), window, cx);
        });
        for tier in ModelTier::ALL {
            let index = tier_index(tier);
            let choice = routing.choice(tier);
            let value = choice
                .model
                .as_ref()
                .map_or_else(|| CHAT_MODEL.to_owned(), encode);
            self.tier_models[index].update(cx, |select, cx| {
                if items_changed {
                    select.set_items(SearchableVec::new(items.clone()), window, cx);
                }
                select.set_selected_value(&value, window, cx);
            });
            self.tier_thinking[index].update(cx, |select, cx| {
                select.set_selected_value(&thinking_value(choice.thinking), window, cx);
            });
        }
        cx.notify();
    }

    fn render_tier_card(
        &self,
        tier: ModelTier,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let index = tier_index(tier);
        v_flex()
            .id(("tier-card", index))
            .test_support()
            .prefer_width(t.layout.card_min_width)
            .flex_1()
            .min_w_0()
            .self_stretch()
            .p(t.space.medium)
            .gap(t.space.small)
            .rounded(t.radius.card)
            .sovra_surface(Surface::Matte, cx)
            .child(
                div()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child(tier_title(tier)),
            )
            .child(caption(tier_examples(tier), cx).whitespace_normal())
            .child(caption("AI", cx))
            .child(div().w_full().child(select_control(
                &self.tier_models[index],
                model_label(tier),
                "Use my chat AI",
                cx,
            )))
            .child(caption("Thinking", cx))
            .child(div().w_full().child(select_control(
                &self.tier_thinking[index],
                thinking_label(tier),
                "AI decides",
                cx,
            )))
            .child(
                caption(self.routing(cx).choice(tier).thinking.description(), cx)
                    .whitespace_normal(),
            )
    }

    fn render_sorter(
        &self,
        routing: &TierRouting,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let laya = self.models.read(cx).state().laya.clone();
        let sorter_button = |id: &'static str,
                             sorter: Sorter,
                             title: &'static str,
                             body: &'static str,
                             cx: &mut Context<Self>| {
            let selected = routing.sorter == sorter;
            design::button(id, "", cx)
                .accessibility_label(format!("{title}. {body}"))
                .selected(selected)
                .toggled(selected)
                .prefer_width(t.layout.card_min_width)
                .flex_1()
                .self_stretch()
                .h_auto()
                .items_start()
                .justify_start()
                .whitespace_normal()
                .p(t.space.medium)
                .rounded(t.radius.card)
                .child(
                    Icon::new(if selected {
                        IconName::CircleCheck
                    } else {
                        IconName::Circle
                    })
                    .small()
                    .text_color(if selected {
                        cx.theme().link
                    } else {
                        t.text_secondary
                    }),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(t.space.compact)
                        .child(
                            div()
                                .whitespace_normal()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child(title),
                        )
                        .child(caption(body, cx).whitespace_normal()),
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.update_routing(cx, |routing| routing.sorter = sorter);
                }))
        };
        let laya_action = match &laya {
            LayaStatus::NotInstalled | LayaStatus::Failed(_) => Some(
                design::primary_button("laya-install-btn", "Download Laya", cx).on_click(
                    cx.listener(|view, _, _, cx| view.send(ModelCommand::InstallLaya, cx)),
                ),
            ),
            LayaStatus::Downloading { .. } => Some(
                design::button("laya-cancel-btn", "Stop download", cx).on_click(
                    cx.listener(|view, _, _, cx| view.send(ModelCommand::CancelLayaInstall, cx)),
                ),
            ),
            LayaStatus::Installed | LayaStatus::Ready => Some(
                design::button("laya-remove-btn", "Delete Laya", cx)
                    .ghost()
                    .on_click(
                        cx.listener(|view, _, _, cx| view.send(ModelCommand::RemoveLaya, cx)),
                    ),
            ),
        };
        let summary = laya.summary();
        v_flex()
            .id("tier-sorter")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .child(div().sovra_text(TextRole::SectionTitle, cx).child("How questions are sorted"))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_stretch()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(sorter_button(
                        "tier-sorter-quick",
                        Sorter::Quick,
                        "Quick rules",
                        "Instant. Looks at length, code and words such as \"prove\" or \"quickly\".",
                        cx,
                    ))
                    .child(sorter_button(
                        "tier-sorter-laya",
                        Sorter::Laya,
                        "Laya, on this computer",
                        "A small sorting AI built into Sovra. Reads 100+ languages. Nothing leaves your computer.",
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .id("laya-status")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .items_center()
                    .gap(t.space.small)
                    .aria_label(format!("Laya: {summary}"))
                    .child(caption(format!("Laya: {summary}"), cx).min_w_0().whitespace_normal())
                    .children(laya_action),
            )
    }

    fn render_tester(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let state = self.models.read(cx).state();
        let routing = state.preferences.tier_routing.clone();
        let result = state.tier_test.as_ref().map(|(prompt, decision)| {
            let who = match decision.source {
                DecisionSource::Asked => "You asked",
                DecisionSource::QuickSorter => "Quick rules",
                DecisionSource::Laya => "Laya",
            };
            let choice = routing.choice(decision.tier);
            let model = choice.model.as_ref().map_or_else(
                || "your chat AI".to_owned(),
                |key| {
                    state
                        .model(key)
                        .map_or_else(|| key.model_id.clone(), |model| model.name.clone())
                },
            );
            format!(
                "\u{201c}{prompt}\u{201d} is a {} question: {} ({who}). It goes to {model} · {}.",
                match decision.tier {
                    ModelTier::Fast => "simple",
                    ModelTier::Standard => "medium",
                    ModelTier::Deep => "complex",
                },
                decision.reason,
                choice.thinking.label()
            )
        });
        v_flex()
            .id("tier-tester")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Try a question"),
            )
            .child(
                caption(
                    "Nothing is sent to any AI. This only shows where a question would go.",
                    cx,
                )
                .whitespace_normal(),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        div()
                            .flex_1()
                            .min_w(t.layout.card_min_width)
                            .child(Input::new(&self.test_input)),
                    )
                    .child(
                        design::button("tier-test-btn", "Sort it", cx)
                            .on_click(cx.listener(|view, _, _, cx| view.test(cx))),
                    ),
            )
            .children(result.map(|text| {
                div()
                    .id("tier-test-result")
                    .test_support()
                    .aria_label(text.clone())
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .child(text)
            }))
    }
}

fn routing_default(tier: ModelTier) -> sovra_core::ai_tiers::TierChoice {
    TierRouting::default().choice(tier)
}

impl Render for AiTiersView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = design::tokens(cx);
        let routing = self.routing(cx);
        let ready = self.models.read(cx).state().preferences.active.is_some();
        let enabled = routing.enabled;
        let switch = Switch::new("tier-routing-switch")
            .accessibility_label("Choose an AI by how hard the question is")
            .checked(enabled)
            .disabled(!ready)
            .tooltip(if enabled {
                "Use one AI for everything"
            } else {
                "Pick an AI by how hard each question is"
            })
            .on_click(cx.listener(|view, checked: &bool, _, cx| {
                let checked = *checked;
                view.update_routing(cx, |routing| routing.enabled = checked);
            }));
        let mut column = v_flex()
            .id("ai-tiers")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.large)
            .child(
                v_flex()
                    .id("chat-thinking")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .p(t.space.medium)
                    .gap(t.space.small)
                    .sovra_surface(Surface::Matte, cx)
                    .child(div().sovra_text(TextRole::SectionTitle, cx).child("How long your chat AI thinks"))
                    .child(
                        caption(
                            if enabled {
                                "Used when sorting by difficulty is off. Each difficulty below has its own setting."
                            } else {
                                "More thinking gives steadier answers to hard questions, but is slower and costs more."
                            },
                            cx,
                        )
                        .whitespace_normal(),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(t.layout.reading_width)

                            .child(select_control(&self.chat_thinking, "Chat AI thinking", "AI decides", cx)),
                    )
                    .child(caption(routing.chat_thinking.description(), cx).whitespace_normal()),
            )
            .child(
                h_flex()
                    .id("tier-routing-row")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .gap(t.space.medium)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(t.space.compact)
                            .child(div().whitespace_normal().sovra_text(TextRole::SectionTitle, cx).child("Choose an AI by how hard the question is"))
                            .child(
                                caption(
                                    if ready {
                                        "Save money and time on easy questions, and use your strongest AI for hard ones. Write \"think hard\" or \"quickly\" in a message to choose yourself."
                                    } else {
                                        "Choose a chat AI first."
                                    },
                                    cx,
                                )
                                .whitespace_normal(),
                            ),
                    )
                    .child(div().flex_shrink_0().child(switch)),
            )
            .child(
                h_flex()
                    .id("tier-cards")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .items_stretch()
                    .flex_wrap()
                    .gap(t.space.small)
                    .children(ModelTier::ALL.map(|tier| self.render_tier_card(tier, cx))),
            );
        if enabled {
            column = column
                .child(self.render_sorter(&routing, cx))
                .child(self.render_tester(cx));
        }
        column
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_values_round_trip() {
        let key = ModelKey {
            connection_id: "openrouter".into(),
            model_id: "anthropic/claude-opus-5-5".into(),
        };
        assert_eq!(decode(&encode(&key)), Some(key));
        assert_eq!(decode(CHAT_MODEL), None);
        for level in ThinkingLevel::ALL {
            assert_eq!(thinking_from(&thinking_value(level)), Some(level));
        }
    }
}
