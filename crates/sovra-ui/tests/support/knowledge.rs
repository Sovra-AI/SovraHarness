//! Operate the single Knowledge scroller at the same size and zoom as a user.
use gpui_kit::{App, ElementId, ScrollDelta, Window, point, px, test::TestWindowExt as _};

pub(super) fn reveal(window: &mut Window, id: impl Into<ElementId>, cx: &mut App) {
    let id = id.into();
    for _ in 0..40 {
        window.render_frame(cx);
        let target = window.find(id.clone());
        let bounds = target.bounds();
        let viewport = window.find("sovra-knowledge-view").bounds();
        let center = bounds.center().y;
        if target.visible() && center >= viewport.top() && center <= viewport.bottom() {
            return;
        }
        window.scroll(
            "sovra-knowledge-view",
            ScrollDelta::Pixels(point(px(0.), viewport.center().y - center)),
            cx,
        );
    }
    panic!("Could not reach {id:?} on the Knowledge page");
}
