//! Model inspection and side-by-side comparison. Unknown metadata is explicit.

use gpui_kit::component::{
    Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{
    ClipboardItem, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div,
};
use sovra_core::models::{ModelCommand, ModelInfo, ProviderKind};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::models_view::{ModelsView, caption, compact_count, pill, token_price};

impl ModelsView {
    pub(crate) fn render_model_details(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut panel = v_flex()
            .id("model-details")
            .flex_1()
            .h_full()
            .min_h_0()
            .overflow_y_scroll()
            .p(t.space.medium)
            .gap(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                Button::new("model-details-close")
                    .label("Back to catalog")
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
                "This model is no longer in the catalog. Refresh and choose another model.",
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
        let action_label = if active {
            "Current chat model"
        } else if missing_key {
            "Connect to use model"
        } else if model.can_chat() {
            "Use in chat"
        } else {
            "Catalog information only"
        };
        let edit_connection = connection.cloned();
        panel = panel.child(caption(model.author.clone(),cx))
            .child(div().sovra_text(TextRole::SectionTitle,cx).child(model.name.clone()))
            .child(caption(model.id.clone(),cx))
            .child(h_flex().flex_wrap().gap(t.space.compact)
                .child(Button::new("model-copy-id").label("Copy ID").small().on_click(move |_,_,cx| cx.write_to_clipboard(ClipboardItem::new_string(model_id.clone()))))
                .child(Button::new("model-detail-favorite").label(if favorite { "★ Favorited" } else { "☆ Favorite" }).small()
                    .on_click(cx.listener(move |view,_,_,cx| view.send(ModelCommand::SetFavorite { key:favorite_key.clone(),favorite:!favorite },cx))))
                .child(Button::new("model-compare-add").label("Compare").small().disabled(self.compare.len() >= 3 && !self.compare.contains(key))
                    .on_click(cx.listener(move |view,_,_,cx| {
                        if !view.compare.contains(&compare_key) && view.compare.len() < 3 { view.compare.push(compare_key.clone()); }
                        view.compare_open = true; cx.notify();
                    }))))
            .child(design::primary_button("model-activate",action_label,cx).disabled(active || (!model.can_chat() && !missing_key))
                .on_click(cx.listener(move |view,_,window,cx| {
                    if missing_key { view.open_connection(edit_connection.clone(),window,cx); }
                    else { view.send(ModelCommand::Activate { key:activate_key.clone() },cx); }
                })))
            .child(caption(if model.can_chat() { "Selection applies to new chat requests. Existing requests keep their model; switching starts fresh provider history." } else { "This model is included in the complete catalog. The current chat composer cannot produce its output type." },cx))
            .child(div().sovra_text(TextRole::Body,cx).child(if model.description.is_empty() { "No description supplied by this provider.".into() } else { model.description.clone() }))
            .child(detail_line("Context window",compact_count(model.context_length),cx))
            .child(detail_line("Maximum output",compact_count(model.max_output_tokens),cx))
            .child(detail_line("Input",list_or_unknown(&model.input_modalities),cx))
            .child(detail_line("Output",list_or_unknown(&model.output_modalities),cx));
        if model.output_modalities.is_empty() {
            panel = panel.child(caption("This server does not advertise modalities. Chat compatibility will be checked by the server when you send a request.",cx));
        }
        panel = panel.child(section("Pricing", cx));
        if model.pricing.is_empty() {
            panel = panel.child(caption("Not reported by this server. Local execution may use your hardware; cloud or remote usage may incur charges.",cx));
        } else {
            for (field, value) in &model.pricing {
                let (label, formatted) = match field.as_str() {
                    "prompt" => ("Input / 1M tokens".into(), token_price(model, field)),
                    "completion" => ("Output / 1M tokens".into(), token_price(model, field)),
                    "input_cache_read" => {
                        ("Cache read / 1M tokens".into(), token_price(model, field))
                    }
                    "input_cache_write" => {
                        ("Cache write / 1M tokens".into(), token_price(model, field))
                    }
                    "request" => ("Per request".into(), format!("${value}")),
                    "image" => ("Per image".into(), format!("${value}")),
                    _ => (
                        format!("Provider rate: {field}"),
                        format!("{value} (provider units)"),
                    ),
                };
                panel = panel.child(detail_line(label, formatted, cx));
            }
            panel = panel.child(caption("USD estimates from the provider catalog. Routing, cache, media and request charges can affect the final bill.",cx));
        }
        panel = panel.child(section("Capabilities & parameters", cx)).child(
            h_flex().flex_wrap().gap(t.space.compact).children(
                model
                    .supported_parameters
                    .iter()
                    .map(|p| pill(p.clone(), cx)),
            ),
        );
        if model.supported_parameters.is_empty() {
            panel = panel.child(caption("No supported parameters reported. Manual connections can declare tool support when their server supports it.",cx));
        }
        for (label, value) in [
            ("Knowledge cutoff", model.knowledge_cutoff.clone()),
            ("Retires on", model.expiration_date.clone()),
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
                .child(section("Connection", cx))
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
                            "cached; refresh for current availability"
                        } else {
                            "last successful refresh"
                        }
                    ),
                    cx,
                ));
            }
            if connection.kind == ProviderKind::OpenRouter {
                let url = format!("https://openrouter.ai/{}", model.id);
                panel = panel.child(
                    Button::new("model-open-provider")
                        .label("Open provider model page")
                        .ghost()
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
        }
        if model.manual {
            let key = key.clone();
            panel = panel.child(
                Button::new("manual-model-remove")
                    .label("Remove manual entry")
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
        let mut content = v_flex().id("models-comparison").flex_1().min_h_0().gap(t.space.small)
            .child(h_flex().justify_between().items_center()
                .child(div().sovra_text(TextRole::SectionTitle,cx).child(format!("Compare models · {}/3",self.compare.len())))
                .child(design::button("models-comparison-close","Back to catalog",cx).on_click(cx.listener(|view,_,_,cx| { view.compare_open = false; view.selected = None; cx.notify(); }))))
            .child(caption("Add up to three models from their details. Prices reflect advertised rates; unknown values are not treated as zero.",cx));
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
                        Button::new(("comparison-remove", index))
                            .label("Remove from comparison")
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
        ("Context", compact_count(model.context_length)),
        ("Max output", compact_count(model.max_output_tokens)),
        ("Input / 1M tokens", token_price(model, "prompt")),
        ("Output / 1M tokens", token_price(model, "completion")),
        ("Input modalities", list_or_unknown(&model.input_modalities)),
        (
            "Output modalities",
            list_or_unknown(&model.output_modalities),
        ),
        ("Parameters", list_or_unknown(&model.supported_parameters)),
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
