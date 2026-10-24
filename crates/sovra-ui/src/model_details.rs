//! Model inspection and side-by-side comparison. Unknown metadata is explicit.

use gpui_kit::component::{
    Disableable as _, Sizable as _, button::ButtonVariants as _, h_flex, v_flex,
};
use gpui_kit::{
    ClipboardItem, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::models::{ModelCommand, ModelInfo, ProviderKind};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::models_view::{ModelsView, caption, compact_count, pill, token_price};

impl ModelsView {
    pub(crate) fn render_model_details(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut panel = v_flex()
            .id("model-details")
            .test_support()
            .flex_1()
            .h_full()
            .min_h_0()
            .overflow_y_scroll()
            .p(t.space.medium)
            .gap(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                design::button("model-details-close", "Back to all models", cx)
                    .ghost()
                    .small()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.selected = None;
                        cx.notify();
                    })),
            );
        let Some(key) = self.selected.as_ref() else {
            return panel;
        };
        let Some(model) = self.state.model(key) else {
            return panel.child(caption(
                "This model isn't available any more. Press Refresh and pick another one.",
                cx,
            ));
        };
        let connection = self
            .state
            .connections
            .iter()
            .find(|c| c.id == key.connection_id);
        let model_id = model.id.clone();
        let favorite_key = key.clone();
        let activate_key = key.clone();
        let compare_key = key.clone();
        let favorite = self.state.preferences.favorites.contains(key);
        let active = self.state.preferences.active.as_ref() == Some(key);
        let missing_key = connection.is_some_and(|c| {
            c.authentication != sovra_core::models::Authentication::None
                && !c.has_credential
                && !c.environment_credential
        });
        let action_label = if !model.can_chat() {
            "Can't be used in chat"
        } else if missing_key {
            "Add your key to use this"
        } else if active {
            "Currently in use"
        } else {
            "Use this model"
        };
        let status_summary = if !model.can_chat() {
            "This model generates non-text output (such as images, video, or embeddings) and cannot be used for chat."
        } else if missing_key {
            "Ready to connect — add your API key once in Settings or Connections to start chatting."
        } else if active {
            "✓ Active now — your Personal AI is currently using this model for new conversations."
        } else {
            "Ready to use — click \"Use this model\" below to make this your active Personal AI."
        };
        let edit_connection = connection.cloned();
        panel = panel
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap(t.space.compact)
                    .child(caption(model.author.clone(), cx))
                    .child(pill(
                        if active {
                            "✓ Active Model".to_owned()
                        } else if missing_key {
                            "Needs API Key".to_owned()
                        } else {
                            "Available".to_owned()
                        },
                        cx,
                    )),
            )
            .child(div().sovra_text(TextRole::SectionTitle, cx).child(model.name.clone()))
            .child(caption(model.id.clone(), cx))
            .child(caption(status_summary, cx))
            .child(h_flex().flex_wrap().gap(t.space.compact)
                .child(design::button("model-copy-id", "Copy name", cx).small().on_click(move |_,_,cx| cx.write_to_clipboard(ClipboardItem::new_string(model_id.clone()))))
                .child(design::button("model-detail-favorite", if favorite { "★ Favorited" } else { "☆ Favorite" }, cx).small()
                    .on_click(cx.listener(move |view,_,_,cx| view.send(ModelCommand::SetFavorite { key:favorite_key.clone(),favorite:!favorite },cx))))
                .child(design::button("model-compare-add", "Compare", cx).small().disabled(self.compare.len() >= 3 && !self.compare.contains(key))
                    .on_click(cx.listener(move |view,_,_,cx| {
                        if !view.compare.contains(&compare_key) && view.compare.len() < 3 { view.compare.push(compare_key.clone()); }
                        view.compare_open = true; cx.notify();
                    }))))
            .child(design::primary_button("model-activate",action_label,cx).disabled(!model.can_chat() || (active && !missing_key))
                .on_click(cx.listener(move |view,_,window,cx| {
                    if missing_key { view.open_connection(edit_connection.clone(),window,cx); }
                    else { view.send(ModelCommand::Activate { key:activate_key.clone() },cx); }
                })))
            .child(caption(if model.can_chat() { "Your next message will use this model. Replies already in progress finish with the old one." } else { "This model makes something other than text (for example images or video), so it can't be used in chat yet." },cx))
            .child(section("What it does", cx))
            .child(div().sovra_text(TextRole::Body,cx).child(if model.description.is_empty() { "No description available.".into() } else { model.description.clone() }))
            .child(section("At a glance", cx))
            .child(detail_line("Memory (how much text it keeps in mind, in tokens)",compact_count(model.context_length),cx))
            .child(detail_line("Longest reply (tokens)",compact_count(model.max_output_tokens),cx))
            .child(detail_line("Understands",list_or_unknown(&model.input_modalities),cx))
            .child(detail_line("Creates",list_or_unknown(&model.output_modalities),cx));
        if model.output_modalities.is_empty() {
            panel = panel.child(caption("This service doesn't say what the model can do. It will be checked when you send a message.",cx));
        }
        panel = panel.child(section("Price", cx));
        if model.pricing.is_empty() {
            panel = panel.child(caption("No price given. Models on your own computer are free to run; online services may charge.",cx));
        } else {
            for (field, value) in &model.pricing {
                let (label, formatted) = match field.as_str() {
                    "prompt" => (
                        "Reading your messages, per million tokens".into(),
                        token_price(model, field),
                    ),
                    "completion" => (
                        "Writing replies, per million tokens".into(),
                        token_price(model, field),
                    ),
                    "input_cache_read" => (
                        "Re-reading earlier messages, per million tokens".into(),
                        token_price(model, field),
                    ),
                    "input_cache_write" => (
                        "Remembering earlier messages, per million tokens".into(),
                        token_price(model, field),
                    ),
                    "request" => ("Per request".into(), format!("${value}")),
                    "image" => ("Per image".into(), format!("${value}")),
                    _ => (
                        format!("Provider rate: {field}"),
                        format!("{value} (provider units)"),
                    ),
                };
                panel = panel.child(detail_line(label, formatted, cx));
            }
            panel = panel.child(caption("Prices in US dollars, as listed by the service. A million tokens is roughly 750,000 words.",cx));
        }
        panel = panel.child(section("Technical details", cx)).child(
            h_flex().flex_wrap().gap(t.space.compact).children(
                model
                    .supported_parameters
                    .iter()
                    .map(|p| pill(p.clone(), cx)),
            ),
        );
        if model.supported_parameters.is_empty() {
            panel = panel.child(caption("No technical details listed.", cx));
        }
        for (label, value) in [
            ("Knows about events up to", model.knowledge_cutoff.clone()),
            ("Available until", model.expiration_date.clone()),
            ("Tokenizer", model.tokenizer.clone()),
            ("Family", model.family.clone()),
            ("Parameters", model.parameter_size.clone()),
            ("Quantization", model.quantization.clone()),
            ("Modified", model.modified_at.clone()),
            (
                "Download size",
                model.size_bytes.map(|bytes| {
                    format!(
                        "{}.{:02} GB",
                        bytes / 1_000_000_000,
                        (bytes % 1_000_000_000) / 10_000_000
                    )
                }),
            ),
        ] {
            if let Some(value) = value {
                panel = panel.child(detail_line(label, value, cx));
            }
        }
        if let Some(connection) = connection {
            panel = panel
                .child(section("Service", cx))
                .child(caption(
                    format!("{} · {}", connection.name, connection.kind.label()),
                    cx,
                ))
                .child(caption(connection.base_url.clone(), cx));
            if let Some(catalog) = self.state.catalogs.get(&connection.id) {
                panel = panel.child(caption(
                    format!(
                        "{} · {}",
                        catalog_age(catalog.snapshot.fetched_at),
                        if catalog.cached {
                            "press Refresh to check it's still available"
                        } else {
                            "up to date"
                        }
                    ),
                    cx,
                ));
            }
            if connection.kind == ProviderKind::OpenRouter {
                let url = format!("https://openrouter.ai/{}", model.id);
                panel = panel.child(
                    design::button("model-open-provider", "Learn more on openrouter.ai", cx)
                        .ghost()
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
        }
        if model.manual {
            let key = key.clone();
            panel = panel.child(
                design::button("manual-model-remove", "Remove this model", cx)
                    .ghost()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.send(ModelCommand::RemoveManualModel { key: key.clone() }, cx);
                        view.selected = None;
                    })),
            );
        }
        panel
    }

    pub(crate) fn render_comparison(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut content = v_flex()
            .id("models-comparison")
            .flex_1()
            .min_h_0()
            .gap(t.space.small)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .sovra_text(TextRole::SectionTitle, cx)
                            .child(format!("Compare models · {}/3", self.compare.len())),
                    )
                    .child(
                        design::button("models-comparison-close", "Back to all models", cx)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.compare_open = false;
                                view.selected = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(caption(
                "Add up to three models with the Compare button on each model.",
                cx,
            ));
        let mut cards = h_flex()
            .id("models-comparison-scroll")
            .flex_1()
            .min_h_0()
            .overflow_x_scroll()
            .gap(t.space.small)
            .items_start();
        for (index, key) in self.compare.iter().enumerate() {
            let Some(model) = self.state.model(key) else {
                continue;
            };
            let key = key.clone();
            cards = cards.child(
                v_flex()
                    .id(("comparison-card", index))
                    .w(t.layout.detail_width)
                    .h_full()
                    .overflow_y_scroll()
                    .flex_shrink_0()
                    .p(t.space.medium)
                    .gap(t.space.small)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(
                        div()
                            .sovra_text(TextRole::SectionTitle, cx)
                            .child(model.name.clone()),
                    )
                    .child(caption(model.id.clone(), cx))
                    .children(
                        comparison_fields(model)
                            .into_iter()
                            .map(|(label, value)| detail_line(label, value, cx)),
                    )
                    .child(
                        design::button(("comparison-remove", index), "Remove from comparison", cx)
                            .ghost()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.compare.retain(|k| k != &key);
                                cx.notify();
                            })),
                    ),
            );
        }
        content = content.child(cards);
        content
    }
}

fn comparison_fields(model: &ModelInfo) -> Vec<(&'static str, String)> {
    vec![
        ("Memory (tokens)", compact_count(model.context_length)),
        (
            "Longest reply (tokens)",
            compact_count(model.max_output_tokens),
        ),
        ("Reading, per million tokens", token_price(model, "prompt")),
        (
            "Writing, per million tokens",
            token_price(model, "completion"),
        ),
        ("Understands", list_or_unknown(&model.input_modalities)),
        ("Creates", list_or_unknown(&model.output_modalities)),
        (
            "Technical details",
            list_or_unknown(&model.supported_parameters),
        ),
        (
            "Quantization",
            model
                .quantization
                .clone()
                .unwrap_or_else(|| "Not reported".into()),
        ),
    ]
}
fn list_or_unknown(values: &[String]) -> String {
    if values.is_empty() {
        "Not reported".into()
    } else {
        values.join(", ")
    }
}
fn section(label: &'static str, cx: &gpui_kit::App) -> gpui_kit::Div {
    div()
        .sovra_text(TextRole::SectionTitle, cx)
        .mt(design::tokens(cx).space.small)
        .child(label)
}
fn detail_line(
    label: impl Into<gpui_kit::SharedString>,
    value: impl Into<gpui_kit::SharedString>,
    cx: &gpui_kit::App,
) -> gpui_kit::Div {
    v_flex()
        .gap(design::tokens(cx).space.compact)
        .child(caption(label, cx))
        .child(div().sovra_text(TextRole::Body, cx).child(value.into()))
}

fn catalog_age(fetched_at: u64) -> String {
    if fetched_at == 0 {
        return "No successful refresh yet".into();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(fetched_at, |d| d.as_secs());
    let age = now.saturating_sub(fetched_at);
    if age < 60 {
        "Updated just now".into()
    } else if age < 3600 {
        format!("Updated {} minutes ago", age / 60)
    } else if age < 86400 {
        format!("Updated {} hours ago", age / 3600)
    } else {
        format!("Updated {} days ago", age / 86400)
    }
}
