//! Attachment identity and thumbnails shared by the composer and saved chat messages.

use crate::design::{self, SovraStyled as _, TextRole};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::{
    App, Image, ImageFormat, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, img,
};
use sovra_core::files::{Attachment, ModelPurpose};
use std::sync::Arc;

pub(crate) fn file_card(file: &Attachment, cx: &App) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let icon = match file.purpose {
        ModelPurpose::Documents => IconName::FileText,
        ModelPurpose::Images => IconName::Image,
        ModelPurpose::Audio => IconName::Music,
        ModelPurpose::Video => IconName::Video,
    };
    let preview = if file.thumbnail.is_empty() {
        div()
            .size(t.layout.attachment_preview)
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .child(Icon::new(icon))
            .into_any_element()
    } else {
        img(Arc::new(Image::from_bytes(
            ImageFormat::Png,
            file.thumbnail.clone(),
        )))
        .size(t.layout.attachment_preview)
        .flex_shrink_0()
        .rounded(t.radius.button)
        .into_any_element()
    };
    let size = if file.size_bytes < 1024 {
        format!("{} bytes", file.size_bytes)
    } else {
        format!("{} KB", file.size_bytes.div_ceil(1024))
    };
    h_flex()
        .id(SharedString::from(format!("attachment-{}", file.id)))
        .min_w_0()
        .gap(t.space.small)
        .items_center()
        .child(preview)
        .child(
            v_flex()
                .min_w_0()
                .gap(t.space.compact)
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .text_ellipsis()
                        .child(file.name.clone()),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(size),
                ),
        )
        .aria_label(format!("Attached file: {}", file.name))
}
