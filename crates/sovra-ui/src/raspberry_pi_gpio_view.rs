//! Native connector reference: pin selection explains numbers, never drives IO.

use crate::{
    design::{self, SovraStyled as _, Surface, TextRole},
    models_view::{ModelsView, caption},
};
use gpui_kit::component::{Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, Focusable as _, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, div,
};
use sovra_core::raspberry_pi::Board;

impl ModelsView {
    pub(crate) fn render_pi_pin_maps(
        &self,
        board: &'static Board,
        service_id: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        render_pin_maps(
            board,
            service_id,
            self.services_screen.pi.gpio_pin.as_ref(),
            |view: &mut Self, service, map, pin, cx| {
                view.services_screen.pi.gpio_pin = Some((service, map, pin));
                cx.notify();
            },
            cx,
        )
    }
}

pub(crate) fn render_pin_maps<V: 'static>(
    board: &'static Board,
    service_id: &str,
    selection: Option<&(String, &'static str, u8)>,
    select: fn(&mut V, String, &'static str, u8, &mut Context<V>),
    cx: &mut Context<V>,
) -> AnyElement {
    let t = design::tokens(cx);
    let mut view = v_flex().w_full().min_w_0().gap(t.space.small)
            .child(div().sovra_text(TextRole::SectionTitle, cx).child("Your GPIO pins"))
            .child(caption("Reference map · not live pin readings. Select a pin to learn what it does.", cx))
            .child(caption(board.gpio_note(), cx))
            .child(caption("Power off before changing wires. GPIO signals use 3.3 V; never wire 5 V or a motor directly to GPIO. LEDs need a series resistor.", cx));
    let maps = board.pin_maps();
    if maps.is_empty() {
        view = view.child(
            div()
                .id(SharedString::from(format!(
                    "pi-carrier-needed-{service_id}"
                )))
                .test_support()
                .aria_label("Carrier board needed to identify physical GPIO pins")
                .sovra_text(TextRole::SectionTitle, cx)
                .child("Which carrier board do you have?"),
        );
    }
    for map in maps {
        let map_id = map.id;
        let mut diagram = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.compact)
            .id(SharedString::from(format!(
                "pi-pin-map-{service_id}-{map_id}"
            )))
            .test_support()
            .aria_label(map.title)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child(map.title),
            )
            .child(caption(map.orientation, cx));
        for pins in map.rows {
            let mut row = h_flex()
                .items_stretch()
                .w_full()
                .min_w_0()
                .gap(t.space.small);
            let mut explanation = None;
            for pin in pins {
                let selected = selection.is_some_and(|(service, map, number)| {
                    service == service_id && *map == map_id && *number == pin.physical
                });
                if selected {
                    explanation = Some(pin.description());
                }
                let service = service_id.to_owned();
                row = row.child(
                    design::button(
                        SharedString::from(format!(
                            "pi-pin-{service_id}-{map_id}-{}",
                            pin.physical
                        )),
                        "",
                        cx,
                    )
                    .small()
                    .selected(selected)
                    .flex_1()
                    .min_w_0()
                    .h_auto()
                    .px(t.space.compact)
                    .py(t.space.compact)
                    .min_h(t.layout.compact_control_height)
                    .accessibility_label(pin.description())
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .sovra_text(TextRole::Caption, cx)
                            .whitespace_normal()
                            .child(format!("{} · {}", pin.physical, pin.label())),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        select(view, service.clone(), map_id, pin.physical, cx);
                    })),
                );
            }
            diagram = diagram.child(row);
            if let Some(explanation) = explanation {
                diagram = diagram.child(
                    div()
                        .id(SharedString::from(format!(
                            "pi-pin-detail-{service_id}-{map_id}"
                        )))
                        .test_support()
                        .aria_label(explanation.clone())
                        .w_full()
                        .min_w_0()
                        .p(t.space.small)
                        .sovra_surface(Surface::Matte, cx)
                        .sovra_text(TextRole::Body, cx)
                        .whitespace_normal()
                        .child(explanation),
                );
            }
        }
        view = view.child(diagram);
    }
    view.child(
        design::button(
            SharedString::from(format!("pi-pin-source-{service_id}")),
            "Open official board guide",
            cx,
        )
        .small()
        .on_click(move |_, _, cx| cx.open_url(board.gpio_source())),
    )
    .into_any_element()
}

#[derive(Default)]
pub(crate) struct PiChatPresentation {
    pub(crate) open: std::collections::HashSet<usize>,
    pub(crate) selection: Option<(String, &'static str, u8)>,
}

/// Reference-only tool output cannot select a device or start a project.
pub(crate) fn render_chat_diagram(
    board_id: &str,
    index: usize,
    state: &PiChatPresentation,
    cx: &mut Context<crate::shell::Shell>,
) -> AnyElement {
    let Some(board) = sovra_core::raspberry_pi::board(board_id) else {
        return div().into_any_element();
    };
    v_flex()
        .w_full()
        .min_w_0()
        .gap(design::tokens(cx).space.small)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(board.name),
        )
        .child(render_pin_maps(
            board,
            &format!("reference-{index}"),
            state.selection.as_ref(),
            |shell: &mut crate::shell::Shell, service, map, pin, cx| {
                shell.chat.list.pause_following_tail();
                shell.chat.pi.selection = Some((service, map, pin));
                shell
                    .chat
                    .list
                    .remeasure_items(0..shell.chat.list.item_count());
                shell.refresh(cx);
            },
            cx,
        ))
        .into_any_element()
}

pub(crate) fn render_chat_card(
    service_id: &str,
    board_id: &str,
    index: usize,
    state: &PiChatPresentation,
    cx: &mut Context<crate::shell::Shell>,
) -> AnyElement {
    let Some(board) = sovra_core::raspberry_pi::board(board_id) else {
        return div().into_any_element();
    };
    let t = design::tokens(cx);
    let open = state.open.contains(&index);
    let mut card = v_flex()
        .id(SharedString::from(format!("pi-chat-card-{index}")))
        .test_support()
        .w_full()
        .min_w_0()
        .sovra_surface(Surface::Matte, cx)
        .p(t.space.medium)
        .gap(t.space.small)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(board.name),
        )
        .child(caption("Selected for this chat", cx))
        .child(
            design::button(
                ("pi-chat-gpio", index),
                if open { "Hide GPIO pins" } else { "GPIO pins" },
                cx,
            )
            .small()
            .selected(open)
            .on_click(cx.listener(move |shell, _, _, cx| {
                shell.chat.list.pause_following_tail();
                if !shell.chat.pi.open.remove(&index) {
                    shell.chat.pi.open.insert(index);
                }
                shell
                    .chat
                    .list
                    .remeasure_items(0..shell.chat.list.item_count());
                shell.refresh(cx);
            })),
        );
    card = card.child(
        design::primary_button(("pi-chat-build", index), "Build a project", cx).on_click(
            cx.listener(|shell, _, window, cx| {
                shell.fill_composer_draft("/coding ", window, cx);
                shell
                    .composer_input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            }),
        ),
    );
    if open {
        card = card.child(render_pin_maps(
            board,
            service_id,
            state.selection.as_ref(),
            |shell: &mut crate::shell::Shell, service, map, pin, cx| {
                shell.chat.list.pause_following_tail();
                shell.chat.pi.selection = Some((service, map, pin));
                shell
                    .chat
                    .list
                    .remeasure_items(0..shell.chat.list.item_count());
                shell.refresh(cx);
            },
            cx,
        ));
    }
    card.into_any_element()
}
