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
- Responsive layout (root `AGENTS.md` → Responsive layout): rem tokens only, no `px(N)`;
  views measure their pane with `design::width_probe`/`WidthClass`, never the window;
  cap reading widths; test minimum, wide (≥2560px) and 200% interface size.
- Keep the SVG logo embedded through `assets.rs` and render it in colour with `img`.
- Test views headlessly with `#[gpui_kit::test]` + `Client::loopback()` (`tests/shell.rs`).
- Team and Artifacts views (`team_view.rs`, `artifacts_view.rs`) read the roster and
  library from the shell's `WorkspaceState` and change them only through
  `Command::Teams` / `Command::Artifacts`; they own just their form state.
- **Skills page (`skill_library.rs`):** Keep the Skills view simple, visual, and
  jargon-free so a non-technical user ("a grandma") can browse capabilities, click
  **Try example** or **Try in chat**, and create custom skills in plain English. Every
  new agent or capability must appear here via `sovra_core::skills::builtin_skills()`.
