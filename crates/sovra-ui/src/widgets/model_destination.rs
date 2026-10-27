//! Shared destination disclosure for explicitly authorized background model work.

use gpui_kit::component::{button::Button, v_flex};
use gpui_kit::{
    Context, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::models::ModelDestination;

use crate::design::{self, SovraStyled as _, TextRole};

pub(crate) fn choice<T: 'static>(
    target: &ModelDestination,
    destination_id: ElementId,
    button: Button,
    cx: &Context<T>,
) -> impl IntoElement + use<T> {
    let t = design::tokens(cx);
    v_flex()
        .min_w_0()
        .w_full()
        .gap(t.space.compact)
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .sovra_text(TextRole::Body, cx)
                .child(target.key.model_id.clone()),
        )
        .child(
            div()
                .min_w_0()
                .sovra_text(TextRole::Caption, cx)
                .id(destination_id)
                .test_support()
                .aria_label(target.base_url.clone())
                .child(target.base_url.clone()),
        )
        .child(button)
}
