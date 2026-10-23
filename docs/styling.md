# Sovra native styling

The product design lives in `crates/sovra-ui/src/design/`. GPUI Component receives
registered **Sovra Dark** and **Sovra Light** themes during `sovra_ui::init`, before
the first window. Dark is the initial mode, matching the source app. The shell's
appearance control changes the current session; persistence belongs to future preferences.

## Source and migration

Imported on 2026-10-23 from the local `SovraSRXProduction` working tree, revision
`732fad30f3e233bd0b9c69b4b8c6ccd617a0d6c7`. The older sibling `SovraSRX/ui` has an
earlier MUI theme and no Sovra SVG. Production contains the current product token
system and logo. The working-tree files, rather than an assumed clean revision,
were the migration source:

| Source under `SovraSRXProduction/frontend/` | Native home |
| --- | --- |
| `src/app/globals.css` product dark/light variables | `design/palette.rs`, `design/tokens.rs` |
| `src/lib/theme/{dark,light,shared,component-overrides}.ts` | component theme adapter and styling recipes |
| `src/lib/design-tokens.ts` | shared violet accents and focus policy |
| `public/SovraLOGO.svg` | `assets/branding/sovra-logo.svg` (byte-identical) |

Source SHA-256 fingerprints:

```text
globals.css      56ae8cb34704958728cf178d45390b053507f3500f42d9d8089ede6bc74a7874
shared.ts        d9dbd8b2b521400cbd2521d6c368226620f8802ff70b45324a916bd3478b3a7e
design-tokens.ts 075a0a954634013c421afc050e1ac736c2d552e3056c644c9f7eea3d865cfd5e
SovraLOGO.svg    d777d67fe37af097692113efbbd00c4b8444ffe664f72e9ace19f523a966cd1d
```

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
  flexible containers and `max_w` so the window can shrink. Sizes are logical
  pixels; typography is in rems and scales with GPUI's root font size.
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

## Native adaptations

| Source treatment | Desktop decision |
| --- | --- |
| Black canvas, cool matte surfaces, violet accents, purple light-mode ink | Preserve the product CSS roles; where CSS and older MUI defaults disagree, CSS product tokens win. |
| Glass blur, saturation, layered backdrop glows | Precompose material fills on the known canvas. Preserve material borders and neutral elevation; no per-frame backdrop filters or animated marketing decorations. Material colours remain opaque and predictable on all desktop renderers. |
| Three-stop 102° violet/deep-violet/indigo button gradient | GPUI 0.8 supports two stops. Use 102° violet → indigo, with deep-violet hover. Darken violet and indigo (`#5566e6` for filled controls) to keep white labels above 4.5:1. The SVG logo itself is unchanged. |
| Low-alpha light-mode captions and hairline input boundaries | Use opaque purple captions and stronger input/focus roles. Decorative borders keep source alpha. Status fills use contrast-safe foregrounds; light green/amber are darkened. |
| SF/Jakarta browser font stack | GPUI's installed system sans and monospace resolution; SF on macOS, native fallback elsewhere. No assumed/downloaded font. |
| Fluid browser typography/spacing | Source desktop caps: 52/36/22/16/13 text sizes, 6/12/18/24/36/48 spacing, 13/12/18/22 button/input/card/panel radii. Native flex layout handles resizing. Letter spacing is left to font shaping; the pinned Styled API has no tracking setter. |
| 40px desktop controls; 44px mobile targets | Keep 40 logical pixels for this desktop app. No mobile CSS breakpoints or browser safe-area rules. |
| Web enter/exit/press motion | Named 240/160/100ms durations; control/material 180/220ms. `tokens.motion` returns zero durations/displacement when `cx.reduce_motion()` is enabled. The shell adds no decorative animation. GPUI's reduction flag is not a claim of automatic OS preference synchronization. |
| Button hover elevation | The source shadow tokens are available for app-owned controls. GPUI buttons keep one neutral shadow while their built-in hover/pressed fill and focus treatments remain intact. |

Marketing landing-page layouts, web resets, mobile navigation clearance, feature
page-specific layouts and obsolete glow aliases are not desktop styling primitives.
Their shared product equivalents are covered above; future native features consume
the same tokens instead of importing web component implementations.

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

Checked online on 2026-10-23, then verified APIs in the pinned local
`gpui-kit 0.8.0`, `gpui-component 0.8.0`, `gpui-pre 0.3.9` sources:

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
