//! Application-level actions, key bindings and the native menu bar.

use gpui_kit::{App, KeyBinding, Menu, MenuItem, actions};
use sovra_core::Command;
use sovra_core::models::{InterfaceScale, ModelCommand};

use crate::{RuntimeClient, design};

actions!(sovra, [Quit, ZoomIn, ZoomOut, ResetZoom]);

pub(crate) fn init(product_name: &'static str, cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &ZoomIn, cx| zoom(design::interface_scale(cx).larger(), cx));
    cx.on_action(|_: &ZoomOut, cx| zoom(design::interface_scale(cx).smaller(), cx));
    cx.on_action(|_: &ResetZoom, cx| zoom(InterfaceScale::default(), cx));
    cx.bind_keys([
        KeyBinding::new("secondary-q", Quit, None),
        // Browser/Zed conventions; `=` is the unshifted `+` key on most layouts.
        KeyBinding::new("secondary-=", ZoomIn, None),
        KeyBinding::new("secondary-+", ZoomIn, None),
        KeyBinding::new("secondary--", ZoomOut, None),
        KeyBinding::new("secondary-0", ResetZoom, None),
    ]);
    // A single-window app: closing the last window quits, as users expect.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
    cx.set_menus([
        Menu {
            name: product_name.into(),
            items: vec![MenuItem::action(format!("Quit {product_name}"), Quit)],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Actual Size", ResetZoom),
                MenuItem::action("Zoom In", ZoomIn),
                MenuItem::action("Zoom Out", ZoomOut),
            ],
            disabled: false,
        },
    ]);
}

/// Applies the level immediately, then persists it with the other preferences.
pub(crate) fn zoom(scale: InterfaceScale, cx: &mut App) {
    design::set_interface_scale(scale, cx);
    let command = Command::Models(ModelCommand::SetInterfaceScale { scale });
    if let Err(error) = cx.global::<RuntimeClient>().0.send(command) {
        tracing::warn!(%error, "could not save the interface size");
    }
}
