//! Render ADK-UI components as native GPUI controls, using Sovra theme roles.

use crate::{
    design::{self, SovraStyled as _, Surface, TextRole},
    shell::Shell,
};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, h_flex, progress::Progress, v_flex};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    TestSupportExt as _, div,
};
use sovra_core::UiComponent;

pub(crate) fn render_surface(
    components: &[UiComponent],
    index: usize,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = design::tokens(cx);
    v_flex()
        .id(("native-surface", index))
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .children(
            components
                .iter()
                .enumerate()
                .map(|(i, component)| render_component(component, &format!("{index}-{i}"), cx)),
        )
        .into_any_element()
}

fn render_component(component: &UiComponent, path: &str, cx: &mut Context<Shell>) -> AnyElement {
    let t = design::tokens(cx);
    let children = |items: &[UiComponent], cx: &mut Context<Shell>| -> Vec<AnyElement> {
        items
            .iter()
            .enumerate()
            .map(|(i, item)| render_component(item, &format!("{path}-{i}"), cx))
            .collect()
    };
    match component {
        UiComponent::Text { text, heading } => div()
            .sovra_text(
                if *heading {
                    TextRole::SectionTitle
                } else {
                    TextRole::Body
                },
                cx,
            )
            .child(text.clone())
            .into_any_element(),
        UiComponent::Card {
            title,
            description,
            children: items,
        } => v_flex()
            .sovra_surface(Surface::Matte, cx)
            .p(t.space.medium)
            .gap(t.space.small)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child(title.clone()),
            )
            .children((!description.is_empty()).then(|| {
                div()
                    .text_color(t.text_secondary)
                    .child(description.clone())
            }))
            .children(children(items, cx))
            .into_any_element(),
        UiComponent::Row(items) => h_flex()
            .flex_wrap()
            .gap(t.space.small)
            .children(children(items, cx))
            .into_any_element(),
        UiComponent::Column(items) => v_flex()
            .gap(t.space.small)
            .children(children(items, cx))
            .into_any_element(),
        UiComponent::Table { headers, rows } => {
            let row = |cells: &[String], header: bool| {
                h_flex()
                    .w_full()
                    .gap(t.space.small)
                    .p(t.space.compact)
                    .sovra_surface(
                        if header {
                            Surface::Matte
                        } else {
                            Surface::Subtle
                        },
                        cx,
                    )
                    .children(cells.iter().map(|cell| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .sovra_text(
                                if header {
                                    TextRole::Caption
                                } else {
                                    TextRole::Body
                                },
                                cx,
                            )
                            .child(cell.clone())
                    }))
            };
            v_flex()
                .w_full()
                .gap(t.space.compact)
                .child(row(headers, true))
                .children(rows.iter().map(|cells| row(cells, false)))
                .into_any_element()
        }
        UiComponent::Code(code) => div()
            .w_full()
            .p(t.space.small)
            .bg(t.code_background)
            .font_family(cx.theme().mono_font_family.clone())
            .child(code.clone())
            .into_any_element(),
        UiComponent::Badge(text) => div()
            .px(t.space.small)
            .py(t.space.compact)
            .rounded(t.radius.pill)
            .sovra_surface(Surface::Subtle, cx)
            .sovra_text(TextRole::Caption, cx)
            .child(text.clone())
            .into_any_element(),
        UiComponent::Alert { title, description } => v_flex()
            .p(t.space.medium)
            .gap(t.space.compact)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child(title.clone()),
            )
            .child(description.clone())
            .into_any_element(),
        UiComponent::Progress { label, value } => v_flex()
            .gap(t.space.compact)
            .child(format!("{label} · {value}%"))
            .child(
                Progress::new(gpui_kit::SharedString::from(format!(
                    "surface-progress-{path}"
                )))
                .accessibility_label(label.clone())
                .value(f32::from(*value)),
            )
            .into_any_element(),
        UiComponent::Button {
            label,
            action_id,
            disabled,
        } => {
            if let Some(setup_target) = action_id.strip_prefix("setup:") {
                let target = setup_target.to_owned();
                let button_id = gpui_kit::SharedString::from(format!("surface-setup-{target}"));
                let button = if matches!(
                    target.as_str(),
                    "openrouter-key" | "selected-key" | "recommended-models"
                ) {
                    design::primary_button(button_id, label.clone(), cx)
                } else {
                    design::button(button_id, label.clone(), cx)
                };
                return button
                    .disabled(*disabled)
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.handle_llm_setup_action(&target, window, cx);
                    }))
                    .into_any_element();
            }
            let prompt = if label.trim().is_empty() {
                action_id.clone()
            } else {
                label.clone()
            };
            design::button(
                gpui_kit::SharedString::from(format!("surface-action-{path}")),
                label.clone(),
                cx,
            )
            .disabled(*disabled)
            .on_click(cx.listener(move |shell, _, window, cx| {
                shell.send_prompt_text(&prompt, window, cx);
            }))
            .into_any_element()
        }
        UiComponent::Divider => div()
            .w_full()
            .border_b_1()
            .border_color(cx.theme().border)
            .into_any_element(),
    }
}
