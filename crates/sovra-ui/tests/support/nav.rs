//! Sidebar clicks that work at every window size.
//!
//! The sidebar's lists scroll when they outgrow the window: a small window, a
//! large text size, or the title bar Sovra draws itself on macOS and Windows. A
//! person scrolls the link into view before clicking it, and so do these tests.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{App, ElementId, ScrollDelta, Window, point, px};

/// The sidebar's scrolling lists: the icon rail, the full link list, and chats.
const SCROLLERS: [&str; 3] = [
    "chat-history-rail-collapsed",
    "sidebar-nav-scroll",
    "chat-history-list",
];

/// Scrolls whichever sidebar list holds `id` until it can be clicked.
fn reveal_in_sidebar(window: &mut Window, id: impl Into<ElementId>, cx: &mut App) {
    let id = id.into();
    for _ in 0..60 {
        window.render_frame(cx);
        let target = window.find(id.clone());
        let Some(list) = SCROLLERS
            .iter()
            .find(|list| target.path().contains(&ElementId::Name((**list).into())))
        else {
            return;
        };
        // A click lands on the centre, and a focus ring may overhang the list's
        // edge, so the centre being inside the list is what counts.
        let (area, item) = (window.find(*list).bounds(), target.bounds());
        let middle = item.center().y;
        if target.visible() && middle > area.top() && middle < area.bottom() {
            return;
        }
        let dy = if middle <= area.top() { 40. } else { -40. };
        window.scroll(*list, ScrollDelta::Pixels(point(px(0.), px(dy))), cx);
    }
    panic!("{id:?} never scrolled into view in the sidebar");
}

/// Clicks a sidebar link or control after bringing it into view.
pub(crate) fn click_in_sidebar(window: &mut Window, id: impl Into<ElementId>, cx: &mut App) {
    let id = id.into();
    reveal_in_sidebar(window, id.clone(), cx);
    window.click(id, cx);
}
