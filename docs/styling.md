# Sovra native styling

The native design system: tokens, both themes, branding, and how layouts
follow the pane at every window size.

The product design lives in `crates/sovra-ui/src/design/`. GPUI Component receives
registered **Sovra Dark** and **Sovra Light** themes during `sovra_ui::init`, before
the first window. Dark is the initial mode. The
**Settings → Appearance** control switches modes and is saved with model
preferences (`ModelCommand::SetAppearance`), so it is restored on the next launch.

## Branding

Only the Sovra mark is imported as an image asset. It remains SVG, including its
original radial gradient. `assets.rs` embeds it with `include_bytes!` and delegates
other paths to the GPUI icon bundle. No asset fetches, runtime file paths, browser
framework, CSS engine, external fonts or added runtime dependencies are required.
Use `img(assets::SOVRA_LOGO)` with a named image wrapper; GPUI's `svg()` paints an
alpha mask and would discard the brand colours. Do not add raster/logo variants.

## Using the system

- Standard controls: use GPUI Component. Colours and interaction states already
  come from `cx.theme()`; `design::button(id, label, cx)` adds Sovra button geometry.
  Use `design::primary_button` for the brand gradient and neutral shadow. GPUI owns
  hover/pressed/focus styling; do not register a second `.hover()` on its buttons.
- Product surfaces: `SovraStyled::sovra_surface` provides `Canvas`, `Chrome`, `Glass`,
  `Subtle`, `Matte` and `Elevated` recipes. Use subtle/matte for repeated rows; reserve
  elevation for floating panels. Set spacing with `design::tokens(cx).space`.
- Text: `sovra_text(TextRole::{Display,PageTitle,SectionTitle,Body,Caption}, cx)` sets
  family, size, leading and weight together. Use `theme.foreground`,
  `tokens.text_secondary` or `theme.muted_foreground` for text. Use `theme.link` for
  text accents, not `theme.primary` (which is a fill).
- Layout: use `tokens.layout` for reading/page widths and control geometry. Use
  flexible containers and `max_w` so the window can shrink. Space, layout and chat
  geometry are rems and radii are root-scaled pixels, so everything follows the
  interface scale with typography. See *Responsive layout* below.
- Category badges: `tokens.tiles.categories` is a six-tone categorical palette for
  telling catalogue groups apart (the Skills page gives each section one), and
  `tokens.tiles.neutral` marks quiet states such as "Off". Each `TileTone` pairs a
  background with an ink that keeps 4.5:1 in both themes; use the pair together.
- Code/transcripts: use `theme.mono_font_family` and `mono_font_size`,
  `tokens.code_background` and `assistant_background`; virtualize long lists.
- Theme changes: `Theme::change(mode, None, cx)` uses the registered Sovra palettes.
  For theme edits use `Theme::update`, which reconciles component backgrounds,
  solid colours and the Base theme projection, then refreshes windows. Never
  cache a theme snapshot in a view or mutate `Theme::global_mut` directly.

```rust,ignore
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use gpui_kit::component::{button::ButtonVariants as _, v_flex};

let t = design::tokens(cx);
v_flex()
    .w_full().max_w(t.layout.reading_width)
    .sovra_surface(Surface::Matte, cx)
    .p(t.space.large).gap(t.space.small)
    .child(div().sovra_text(TextRole::SectionTitle, cx).child("Providers"))
    .child(design::button("add-provider", "Add provider", cx).primary());
```

New reusable roles belong in `design/` with semantic names, not feature names or
colour names in views. Keep theme-private palette details private. This is a typed
Rust system; do not duplicate it in JSON, CSS, another crate, or agent rule files.
Extend recipes when behaviour is already supplied by GPUI, and extract a widget
when two real callers share behaviour. Avoid wrapper frameworks and parallel
theme globals.

## Native materials and controls

Use the current palettes and tokens in `design/` as the source of truth. Material
fills are precomposed against the canvas; the app does not use browser CSS or
per-frame backdrop filters. Controls use system fonts, native keyboard behavior,
visible focus, and labelled icon actions. The logo preserves its original SVG colors.

Theme and token tests check contrast and scale behavior. Motion tokens honor
`cx.reduce_motion()`. This does not establish automatic synchronization with every
operating system's accessibility settings.

## Checks

`cargo xtask ci` includes `check-styles`, which rejects common raw colour constructors
and hex literals outside `src/design/`, and extra/missing branding assets. Agent hooks
also run this check. It is a lightweight source guard, not a complete Rust parser;
review semantic choices and avoid bypassing it with aliases/strings.

UI tests exercise both modes, component/Base synchronization, gradient survival,
text/control/focus contrast, reduced-motion tokens, pointer/keyboard appearance switching and bounds
at minimum/default window sizes. An asset test decodes/rasterizes the real embedded
SVG and checks that it has coloured pixels and that GPUI icons still load. Headless
tests do not prove platform font appearance or GPU pixel fidelity; review actual
windows when changing typography, layout or renderer behaviour.

## Researched guidance

Use the pinned crate source for APIs; the following references explain the
design and accessibility principles:

- [GPUI Kit theme guidance](https://gpui-kit.com/component/theme/): semantic colours,
  registered themes and renderable gradient tokens.
- [GPUI Kit crate documentation](https://docs.rs/gpui-kit/0.8.0/gpui_kit/): use the
  umbrella dependency and its component/assets layers together.
- [GPUI asset source](https://docs.rs/gpui-kit/0.8.0/gpui_kit/trait.AssetSource.html):
  application-provided loading/listing. The bundled source is composed, not replaced.
- [W3C text contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html):
  4.5:1 normal text; used as a desktop readability benchmark, not a conformance claim.
- [W3C non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html):
  3:1 for control identification and meaningful states.

The native adaptations are application decisions based on these constraints, not
claims that the upstream library prescribes Sovra's visual design.

## Conversational chat

Chat uses plain assistant prose, a compact user bubble, and aligned transcript
and composer widths. `chat.column_width` caps the reading column. Enter submits
and Shift+Enter inserts a newline. Each assistant
message retains a `TextViewState`: incremental Markdown parsing preserves text
selection and supports incomplete formatting while streaming. Code and tables
scroll horizontally within the reading column; copy controls use native buttons.
`stream_fade(true)` follows the component theme's motion policy and reduced-motion
setting. Chunks are never artificially delayed; already available events are
applied in bounded batches to avoid redundant UI updates.

The pinned GPUI `ListState` owns variable-height virtualization and `FollowMode::Tail`.
It follows additions and Markdown remeasurement until the user scrolls up, keeps
the reading position during further updates, and resumes at the bottom or via
Jump to latest. Sending a new message also resumes following. This uses the
native list's layout/scroll state rather than estimated text heights or timers.


## Workspace composition

The original migration is provenance, not a constraint to preserve every web
visual. The current native design emphasizes reading and one obvious next action.
It retains the embedded Sovra SVG, registered light/dark themes and violet identity.

### Design principles

These are product decisions drawn from the guidance, not
claims that a design system prescribes this exact palette or layout.

| Evidence | Application in Sovra |
| --- | --- |
| [Cloudscape: generative AI chat](https://cloudscape.design/gen-ai/patterns/generative-ai-chat/) distinguishes authors, constrains dedicated chat bubbles and separates contextual actions. | Bare assistant prose, quiet right-aligned user bubbles and a stable reading column. Prompt starters fill an editable draft. Conversation actions are named icon buttons on the selected thread. |
| [Cloudscape: generative AI loading states](https://cloudscape.design/gen-ai/patterns/generative-ai-loading-states/) separates processing from generation and pairs indicators with readable status. | Keep the existing thinking/elapsed state, incremental answer, stop action and visible queued-message count. No invented percentage progress. |
| [Carbon for AI](https://carbondesignsystem.com/guidelines/carbon-for-ai/) treats explainability as part of the workflow and makes detail available on demand. | Reasoning, model attribution, routing details and advanced connection fields remain discoverable without dominating the answer or setup. |
| [Nielsen Norman Group: progressive disclosure](https://www.nngroup.com/articles/progressive-disclosure/) places common actions first while retaining secondary functionality. | State-aware model setup, one-line setting descriptions, collapsed technical settings and selected-thread actions. |
| [W3C: contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html), [focus](https://www.w3.org/WAI/WCAG22/Understanding/focus-visible.html) and [target size](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html). | Test normal text at 4.5:1 and input/focus identification at 3:1 in both palettes. Native buttons provide keyboard operation. Compact controls are at least 28px high; navigation is 36px. These are desktop design benchmarks, not a web-conformance claim. |

### Component intent

- **Shell:** quiet chrome, a compact status line and a 256px sidebar. Navigation
  rows have consistent icon/text alignment and comfortable targets. At narrow
  widths, the existing icon rail preserves content space.
- **Conversation:** reading comes first. Neutral user bubbles replace repeated
  gradients; the brand remains in the avatar. The composer is an 18px rounded
  surface with a clear focus treatment, send/stop controls and keyboard hint.
- **Welcome:** a small brand mark, one clear greeting and four editable starters.
  No dashboard statistics or decorative motion compete with the message box.
- **Pages:** `design::page_header` keeps title, description and spacing consistent.
  Wide task pages use a bounded content column. Form and catalog controls wrap at
  smaller widths instead of pushing actions outside the window.
- **Settings and models:** short labels, brief explanations and grouped surfaces.
  Persistent choices are visually quieter than a next-step setup action. Avoid
  gradients on every recommended model.
- **Knowledge:** readable explanatory cards, no nested elevation. The optional
  inspector yields its width on compact windows.
- **Schedules:** the same page/card rhythm, a swimlane of past and upcoming runs,
  and a “now” marker. Repeating tasks run in the background; pause one anytime.
- **Agent cards and forms:** shared matte surfaces, section typography and native
  actions inherit the same system. Reserve shadows for floating material, not
  every repeated row.
- **Training:** one bounded column that reads top to bottom as a single task:
  what this computer has, what to learn, the starting point, how thorough, and
  one primary **Start training** action. Recommended choices are preselected
  so the page works without a decision; expert settings stay collapsed under
  **Advanced** (progressive disclosure). Words stay plain ("round", "helper",
  "computer"); technical terms appear only inside Advanced.
- **Training chart:** the kit's `AreaChart` with transparent fills draws the
  loss and the check loss as two lines on one scale, coloured from
  `cx.theme().chart_1` and `chart_2`. The kit's `LineChart` holds one series,
  so it is not used here. The chart has no gaps for missing values, so
  `TrainingMetrics::chart` in core carries the check value forward, and the
  view draws a single series when no check was ever measured. The view
  converts core's whole-number thousandths to floats; core stays integer. The
  chart takes its height from `tokens.training.chart_height` (a shorter
  `chart_compact_height` in compact panes) and the pane's full width, with
  `appear(false)` because it repaints on every progress event. Colour is
  never the only signal: a legend names each line, the same values are in
  the number tiles, and the wrapper carries a role and a spoken summary
  because the chart itself exposes none. Axis text is the kit's fixed-pixel
  text and does not follow the interface scale; the tiles carry the readable
  numbers.
- **Training numbers:** hardware facts and progress tiles are wrapping rows of
  `tokens.training.tile_width` tiles, measured from the pane with
  `design::width_probe`, so they reflow at the minimum window and at 200%
  text size instead of shrinking. Written sample text and a tried model's
  answer scroll inside `sample_max_height`. Loss is shown with a plain-word
  reading from core beside the figure.

### Rust and GPUI implementation rules

Verified against the installed pinned sources: `gpui-component-0.8.0/src/button/button.rs`,
`src/theme/mod.rs`, and `gpui-pre-0.3.9/src/elements/div.rs`.
`Button` owns keyed focus, keyboard activation, accessibility, hover, selection
and disabled behavior. A div with `Role::Button` alone does not implement those
behaviors. Use a real button for cards that act as a single choice. Multi-line
button children must explicitly reset `whitespace_normal()` because ordinary
button labels use a nonwrapping inner container.

Keep colors and reusable geometry in `design/`; derive extension tokens from
`cx.theme().mode`. Do not introduce per-view theme snapshots, runtime assets,
remote fonts, CSS classes or additional styling dependencies. Preserve the cached
transcript and variable-height virtualization; a visual refinement must not make
unrelated controls reparse streamed Markdown. Validate native pixels as well as
headless layout, focus and protocol tests.


## Responsive layout

The rules for agents are in root `AGENTS.md` →
*Responsive layout*. `check-styles` enforces literal sizes and window reads.

### Scale: rems everywhere, one root

GPUI's `Root` sets `window.set_rem_size(theme.font_size)` every frame. `rems()`
lengths therefore scale "just like zooming a web page" (`Window::set_rem_size`),
and so do the gpui-component controls, which use the rem-based `h_8`/`px_2`
scale. Sovra tokens are authored in 16px reference pixels and stored as `Rems`.
Radii are pixels (GPUI buttons take `Pixels`) multiplied by the live root. Only
1px hairlines, shadows and OS window bounds stay as logical pixels.

| Concern | Decision | Source |
| --- | --- | --- |
| Display density | Lay out in logical pixels/rems only. GPUI converts with `scale_factor()` and snaps to device pixels. | [Windows: effective pixels and scaling](https://learn.microsoft.com/en-us/windows/apps/design/layout/screen-sizes-and-breakpoints-for-responsive-design) |
| User text size | macOS has no Dynamic Type for Mac apps, so Sovra provides **Text size** 75–200% (Settings, View menu, ⌘= ⌘- ⌘0), persisted as `InterfaceScale`. `design::set_interface_scale` rewrites both registered theme configs, so mode switches keep it. | [Apple HIG: Typography](https://developer.apple.com/design/human-interface-guidelines/typography), [Accessibility (200% text)](https://developer.apple.com/design/human-interface-guidelines/accessibility), [Zed `ui_font_size`](https://zed.dev/docs/reference/all-settings) |
| Breakpoints | `WidthClass` uses Material 3 window classes in rems: compact <37.5, medium <52.5, expanded <75, large <100, extra-large. Because they are rems, zooming in moves a window into a narrower class, like `em` media queries. | [Material 3 window size classes](https://developer.android.com/develop/ui/compose/layouts/adaptive/use-window-size-classes) |
| Container, not device | Only the shell (the window root) reads the window. Nested views use `design::width_probe`, a `canvas` that measures the parent's bounds in prepaint and notifies only when the class changes. This is Zed's centre-pane measurement pattern. | [Apple HIG: Layout](https://developer.apple.com/design/human-interface-guidelines/layout), [MDN: container queries](https://developer.mozilla.org/en-US/docs/Web/CSS/Guides/Containment/Container_queries) |
| Pane order | Below Large, the Knowledge inspector undocks; the model detail pane undocks when the models pane itself is below Expanded. Below Expanded, the sidebar becomes an icon rail with an overlay. The user's explicit collapse still wins. | [Apple HIG: Sidebars](https://developer.apple.com/design/human-interface-guidelines/sidebars), [Material canonical layouts](https://developer.android.com/develop/ui/compose/layouts/adaptive/canonical-layouts), [Windows responsive techniques](https://learn.microsoft.com/en-us/windows/apps/design/layout/responsive-design) |
| Large monitors | Chat prose is capped at `chat.column_width` (46rem, ≈80 characters of reading text) and centred. Forms use `reading_width` (48rem), card pages `page_width` (67.5rem). There are no full-bleed paragraphs. | [WCAG 1.4.8 Visual presentation](https://www.w3.org/WAI/WCAG22/Understanding/visual-presentation.html), [Practical Typography: line length](https://practicaltypography.com/line-length.html) |
| Window | First launch uses 1280×820, clamped to 90% of the display's visible area (dock and taskbar excluded) and centred. The minimum is 640×420. | [Apple: Designing for macOS](https://developer.apple.com/design/human-interface-guidelines/designing-for-macos) |

### Patterns

```rust,ignore
// A nested view adapts to its own pane, not the window.
struct MyView { width_class: design::WidthClass, /* … */ }

impl Render for MyView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = design::tokens(cx);
        let two_pane = self.width_class.fits_side_pane();
        div()
            .relative()
            .size_full()
            .min_w_0()
            .child(design::width_probe(cx, |view: &mut Self| &mut view.width_class))
            .child(
                h_flex().size_full().min_w_0()
                    .child(div().flex_1().min_w_0().max_w(t.layout.reading_width) /* main */)
                    .when(two_pane, |row| row.child(div().w(t.layout.detail_width).flex_shrink_0())),
            )
    }
}
```

A missing size is a new semantic token in `design/tokens.rs`, never a literal in a view.

### Verification

`crates/sovra-ui/tests/styling.rs` covers:

- every workspace page at the minimum window, and again at 200% text size;
- a 2560×1440 window, where the chat column stays capped and centred;
- zoom collapsing the sidebar to a rail and surviving a light/dark switch;
- unit tests for the width classes and the window fitting.

Headless tests do not prove GPU pixel fidelity. Also look at a real window at a
small size, on a large monitor and at a 1.0 or 2.0 scale factor.
