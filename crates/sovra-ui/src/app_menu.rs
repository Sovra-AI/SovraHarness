//! Application-level actions, key bindings and the native menu bar.

use gpui_kit::{App, KeyBinding, Menu, MenuItem, actions};

actions!(sovra, [Quit]);

pub(crate) fn init(product_name: &'static str, cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
    // A single-window app: closing the last window quits, as users expect.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
    cx.set_menus([Menu {
        name: product_name.into(),
        items: vec![MenuItem::action(format!("Quit {product_name}"), Quit)],
        disabled: false,
    }]);
}
