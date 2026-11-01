//! Exact recipient/content review in the chat transcript. Only real button input
//! sends approval commands; model-generated widgets cannot call these controls.

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::{RuntimeClient, Shell};
use gpui_kit::component::{button::ButtonVariants as _, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::Command;
use sovra_core::communications::{MessageDelivery, OutboundMessage};
use sovra_core::connections::ConnectionCommand;

pub(crate) fn render(
    message: &OutboundMessage,
    index: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut actions = h_flex().w_full().flex_wrap().gap(t.space.small);
    if message.delivery == MessageDelivery::Review {
        for (send, label) in [(true, "Send"), (false, "Cancel")] {
            let id = message.id.clone();
            let session_id = message.session_id.clone();
            actions = actions.child(
                design::button(
                    (
                        if send {
                            "message-review-send"
                        } else {
                            "message-review-cancel"
                        },
                        index,
                    ),
                    label,
                    cx,
                )
                .accessibility_label(if send {
                    "Send this exact message to the shown recipient"
                } else {
                    "Cancel this message without sending"
                })
                .on_click(move |_, _, cx| {
                    let _ = cx.global::<RuntimeClient>().0.send(Command::Connections(
                        ConnectionCommand::ReviewMessage {
                            id: id.clone(),
                            session_id: session_id.clone(),
                            send,
                        },
                    ));
                }),
            );
        }
    }
    let text = format!(
        "{}\nFrom: {}\nTo: {} ({})\n\n{}\n\n{}",
        message.channel.label(),
        message.sender,
        message.recipient_label,
        message.recipient_display(),
        message.text,
        message.delivery.label()
    );
    let source = sovra_core::artifacts::ArtifactSource {
        session_id: message.session_id.clone(),
        message_id: message.id.clone(),
        author: "Sovra".into(),
    };
    actions = actions.child(
        design::button(("message-review-save", index), "Save", cx)
            .ghost()
            .accessibility_label("Save this message and its receipt")
            .on_click(move |_, _, cx| {
                let draft =
                    sovra_core::artifacts::draft_from_reply(&text, source.clone(), "Saved message");
                let _ = cx.global::<RuntimeClient>().0.send(Command::Artifacts(
                    sovra_core::artifacts::ArtifactCommand::Save(draft),
                ));
            }),
    );
    v_flex()
        .id(("message-review", index))
        .test_support()
        .w_full()
        .min_w_0()
        .sovra_surface(Surface::Subtle, cx)
        .p(t.space.medium)
        .gap(t.space.small)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(format!("{} message", message.channel.label())),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .child(format!("From: {}", message.sender)),
        )
        .child(div().sovra_text(TextRole::Body, cx).child(format!(
            "To: {} ({})",
            message.recipient_label,
            message.recipient_display()
        )))
        .child(
            div()
                .id(("message-review-text", index))
                .test_support()
                .sovra_text(TextRole::Body, cx)
                .whitespace_normal()
                .child(message.text.clone()),
        )
        .child(
            div()
                .id(("message-review-status", index))
                .test_support()
                .aria_label(message.delivery.label())
                .sovra_text(TextRole::Caption, cx)
                .child(message.delivery.label()),
        )
        .child(actions)
}
