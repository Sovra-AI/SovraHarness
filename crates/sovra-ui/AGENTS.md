# sovra-ui

GPUI views. Owns windows, views, actions, key bindings, menus and theming.

- The only crate that may use `gpui-kit`. Check APIs against the pinned source (see root
  `AGENTS.md` → GPUI notes), not memory.
- Reach the runtime via `cx.global::<RuntimeClient>()`; send `Command`s, react to `Event`s.
  Fold events with `sovra-core` reducers — views hold state, they don't compute domain logic.
- One view per file under `src/`; reusable widgets go in a `widgets/` module once there are two
  users. Prefer `gpui_kit::component` widgets over hand-rolled ones.
- Read `../../docs/styling.md` before styling. Colours from `cx.theme()`; product
  spacing/radii/materials/motion from `design::tokens(cx)`. Use `SovraStyled` recipes
  and `design::button`. New shared roles go in `src/design/`, never raw literals in views.
- Every interactive element needs an accessible label and visible keyboard focus.
  Check both Sovra modes and minimum window bounds. Preserve native control behaviour.
- Keep the SVG logo embedded through `assets.rs` and render it in colour with `img`.
- Test views headlessly with `#[gpui_kit::test]` + `Client::loopback()` (`tests/shell.rs`).
