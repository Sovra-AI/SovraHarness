---
name: change-sovra-screens
description: Use when changing how any Sovra screen looks or behaves in a SovraHarness checkout, such as a page layout, a button, colours, spacing or wording on screen.
metadata:
  sovra-title: Change a Sovra screen
  sovra-example: Make the cards on Sovra's Plugins page show who made each plugin
---

# Change a Sovra screen

Sovra's screens are GPUI views in `crates/sovra-ui`, the only crate that may use
`gpui-kit`. Read `docs/styling.md` and the styling and responsive-layout sections
of `AGENTS.md` before changing appearance.

1. **Check the API against the pinned source.** `gpui-kit` changes weekly. Read
   `~/.cargo/registry/src/*/gpui-kit-0.8.0` (and `gpui-component-0.8.0`) rather
   than relying on memory or older GPUI examples. Import through `gpui_kit::*`.
2. **Use the design system.** Colours come from `cx.theme()`; spacing, radii and
   sizes from `design::tokens(cx)`; surfaces and text from `SovraStyled`; buttons
   from `design::button`. Never write `px(N)` or `rems(N)` in a view, and never
   put raw colours outside `crates/sovra-ui/src/design/`.
3. **Make it fit every window.** Decide layout from the view's own width with
   `design::width_probe` and `design::WidthClass`, not the window size (only
   `shell.rs` reads the viewport). Main panes get `flex_1().min_w_0().min_h_0()`,
   text in rows gets `min_w_0` and truncation or wrapping, rows of actions
   `flex_wrap`. Cap reading width with `chat.column_width`,
   `layout.reading_width` or `layout.page_width`.
4. **Keep it usable.** Real GPUI controls, visible keyboard focus, labels, and
   readable disabled, hover and pressed states in both themes. Respect
   `cx.reduce_motion()`. New custom controls get a stable ID, an accessible label
   and `.test_support()`.
5. **Plain words on screen.** Say "helper", "Save", "Open in browser"; never
   "persona", "agent" or "export".
6. **Keep views thin.** Views send `Command`s and render state from `Event`s.
   Keep `Task` and `Subscription` handles in the view, and call `cx.notify()`
   after changing state.
7. **Prove it.** Add or update a headless test in `crates/sovra-ui/tests/` at the
   minimum window (640×420), a wide window (at least 2560px) and 200% text size.
   `cargo xtask check-styles` must pass.
8. **Look at it.** Launch once with `cargo xtask dev` (there is no hot reload) and
   check the screen through Development mode as described in
   `docs/development-mcp.md`. Then follow `finish-a-sovra-change`.
