# Sovra native styling

The native design system: tokens, both themes, branding, and how layouts
follow the pane at every window size.

The product design lives in `crates/sovra-ui/src/design/`. GPUI Component receives
registered **Sovra Dark** and **Sovra Light** themes during `sovra_ui::init`, before
the first window. Dark is the initial mode, matching the source app. The
**Settings → Appearance** control switches modes and is saved with model
preferences (`ModelCommand::SetAppearance`), so it is restored on the next launch.

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
  flexible containers and `max_w` so the window can shrink. Space, layout and chat
  geometry are rems and radii are root-scaled pixels, so everything follows the
  interface scale with typography. See *Responsive layout* below.
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
| Original black canvas and purple light-mode ink | The October 25 refinement uses charcoal and paper surfaces with neutral ink. Violet remains the identity, selected-state and primary-action accent. |
| Glass blur, saturation, layered backdrop glows | Precompose material fills on the known canvas. Preserve material borders and neutral elevation; no per-frame backdrop filters or animated marketing decorations. Material colours remain opaque and predictable on all desktop renderers. |
| Three-stop 102° violet/deep-violet/indigo button gradient | GPUI 0.8 supports two stops. Use 102° violet → indigo, with deep-violet hover. Darken violet and indigo (`#5566e6` for filled controls) to keep white labels above 4.5:1. The SVG logo itself is unchanged. |
| Low-alpha light-mode captions and hairline input boundaries | Use opaque purple captions and stronger input/focus roles. Decorative borders keep source alpha. Status fills use contrast-safe foregrounds; light green/amber are darkened. |
| SF/Jakarta browser font stack | GPUI's installed system sans and monospace resolution; SF on macOS, native fallback elsewhere. No assumed/downloaded font. |
| Fluid browser typography/spacing | Native roles: 36px welcome, 28px page title, 16px section title, 14px UI body, 16px reading text, 12px captions. Spacing: 2/4/6/8/16/24/32/48 (as rems of a 16px root); radii: 6/10/10/14/18. Native flex layout and rem width classes handle resizing. Letter spacing stays with font shaping. |
| Desktop controls | 2.25rem (36px at 100%) for standard controls/navigation, 1.75rem (28px) for compact actions. Icon actions retain accessible names and tooltips. Width classes replace CSS media queries; no browser safe-area rules. |
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

## Conversational chat

The normal chat follows `SovraSRXProduction`'s `assistant-message.tsx`, shared
`sovra-chat-column` width and `use-stick-to-bottom.ts` reading behaviour: plain
assistant prose, a compact user bubble, and aligned transcript/composer widths.
The native layout uses `layout.reading_width`, with no task banner or fabricated
attachment/voice actions. Enter submits and Shift+Enter inserts a newline.

[GPUI TextView guidance](https://gpui-kit.com/component/text-view/) was checked
on 2026-10-24 and verified against the pinned 0.8.0 sources. Each assistant
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


## October 25 refinement: a calm native workspace

The original migration is provenance, not a constraint to preserve every web
visual. The current native design emphasizes reading and one obvious next action.
It retains the embedded Sovra SVG, registered light/dark themes and violet identity.

### Research and application

Reviewed on 2026-10-25. These are product decisions drawn from the guidance, not
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

Researched on 2026-10-25 and verified against the pinned `gpui-pre 0.3.9` /
`gpui-component 0.8.0` sources. The rules for agents are in root `AGENTS.md` →
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

`tests/styling.rs` covers:

- every workspace page at the minimum window, and again at 200% text size;
- a 2560×1440 window, where the chat column stays capped and centred;
- zoom collapsing the sidebar to a rail and surviving a light/dark switch;
- unit tests for the width classes and the window fitting.

Headless tests do not prove GPU pixel fidelity. Also look at a real window at a
small size, on a large monitor and at a 1.0 or 2.0 scale factor.

## Beginner usability review (November 12, in progress)

The review covers the whole product, including first use, discovery, everyday
tasks, recovery, keyboard access and small windows. A completed source review
is not proof that a feature is usable in the running application.

Research checked on 2026-11-12:

- [Nielsen Norman Group: usability heuristics](https://www.nngroup.com/articles/ten-usability-heuristics/):
  visible choices, familiar words, predictable actions and useful error recovery.
- [W3C: clear purpose](https://www.w3.org/WAI/WCAG2/supplemental/patterns/o1p01-clear-purpose/):
  explain what a page is for and make the next action recognizable.
- [W3C: step-by-step instructions](https://www.w3.org/WAI/WCAG2/supplemental/patterns/o4p07-step-instructions/):
  divide unfamiliar tasks into clear steps.

Changes in the first pass put Skills beside Chat, name the account footer's
actual Sign out action, and make Skills' Open now controls open their feature
instead of requiring a second command submission. Navigation shortcuts preserve
an unfinished chat draft; starting a new chat remains an explicit separate action.
The tour's stale sample-memory and unavailable-scheduling descriptions were
replaced after checking the live memory view and runtime scheduler. Schedule
instructions now explain that Sovra must remain open.

The second pass adds one-step recovery from combined saved-work filters,
removes unused controls from an empty library, and adds a direct Open chat
action there. It also fixes name-only account recovery: the sign-in page offers
the saved name, and the runtime accepts that path only for accounts created
without email, password or Google credentials. Password and Google accounts are
excluded by a pure domain rule and checked again by the runtime.

Remaining review and verification scope:

| Flow | Evidence still needed |
| --- | --- |
| Sign-in, first chat and AI setup | Follow each setup stage, missing-key errors and recovery in the native app. |
| Sidebar, Skills and tour | Verify shortcuts and examples, keyboard access, compact navigation and tour accuracy. |
| Chat, attachments, history and saved work | Follow create, find, edit, save and remove paths; inspect empty and error states. |
| Models, personal helpers and teams | Check beginner defaults, optional advanced controls and disabled-state explanations. |
| Knowledge and file indexing | Follow add, review, find, correct and remove paths with test data. |
| Schedules | Tour wording checked against the runtime; exercise create, automatic run, pause and failure recovery. |
| Connections and groups | Follow connection, invitation, sharing and disconnect flows without sending real messages. |
| Settings, terminal and phone access | Verify labels, prerequisites, confirmations and revocation. |
| Cross-cutting layout | Minimum, wide and 200% text tests; both themes; actual native screenshots where supported. |

Native MCP inspection is available on the current Linux renderer; screenshot
capture returned “Screenshot capture is unavailable on this platform renderer.”
An OS window capture subsequently worked. It revealed Arabic login description
and hint text painting outside the card despite passing element-bounds tests.
Fix and recheck this actual rendering defect; do not treat headless bounds as
evidence that all visible text fits.

The shared `design::paragraph` helper uses logical-order wrapping for static
right-to-left text in page headers and sign-in instructions. It uses pinned
`gpui-bidi` 0.1.1 with the existing GPUI 0.3.9 shaper; it does not replace editable
inputs or rich chat content. See the library's
[paragraph implementation notes](https://docs.rs/gpui-bidi/0.1.1/gpui_bidi/paragraph/index.html).
Its regression checks actual shaped text runs and reading order, in addition to
container bounds. Sign-in fields also keep translated labels visible while typing,
following [W3C form-label guidance](https://www.w3.org/WAI/tutorials/forms/labels/).
The Arabic paragraph regression and sign-in control reachability checks pass at
minimum/wide/200% sizes in both themes. The paragraph check permits the shaper's
measured trailing-space advance; native captures separately confirm the painted
glyphs. Development MCP plus OS window captures verified the fresh-account and
returning-account Arabic screens, including the long first-use subtitle and hint.
Entering a name and submitting the fresh-account form reached the workspace.
These checks do not cover bidirectional text editing or all rich-text surfaces.

Third-pass verification: all 186 UI tests pass. The full gate passed fmt, Clippy,
layering, styling, documentation, dependency policy and unused-dependency checks,
then stopped after 169 workspace tests (167 passed, two failed). The failures were
linked-helper memory evidence after restart and the indexed-knowledge provider
fixture expecting text without its new `Source: budget.txt` prefix. Full CI is
still not green. The next recovery issue found in source is saved-work editing:
Back and Cancel leave the edit without an unsaved-change confirmation.

First-pass verification is incomplete: two full CI runs stopped on different
memory lifecycle assertions (the first failing test passed alone). A subsequent
UI test build and development reload encountered incomplete concurrent WorkLog
enum/render changes. Re-run the full gate and live checks once the shared tree
builds; do not treat these attempts as passing verification.

Second-pass targeted evidence: all 12 authentication domain/runtime tests passed,
including sign-out/restart recovery and rejection of password/Google account IDs.
The headless native-control tests passed for profile sign-in, combined-filter
recovery, and both flows' minimum/wide/200% layouts in both themes. The broader
UI run finished with 183 passed and two failures in concurrent progress-board
tests; the full gate remains required. Through Development MCP with a temporary
data folder, creating a name-only account, signing out and reopening it passed.
The empty-library Open chat action and Skills' direct Open now navigation also
passed in the live native app. Combined-filter recovery has headless coverage;
it still needs a live populated-library check.


Saved-work editing now checks for changed name/content before Back, Cancel or
opening another item. The inline choice keeps the original draft until the user
chooses Keep editing or Discard changes, and scrolls the choice into view.
Unchanged forms leave directly. Editing hides sharing, deletion and organization
actions that operate on the saved version. This follows the
[warn-before-leaving pattern](https://design-system.agriculture.gov.au/patterns/warn-before-leaving)
and W3C's guidance to [let users go back without losing work](https://www.w3.org/WAI/WCAG2/supplemental/patterns/o4p02-back-undo/).
The regression first reproduced the missing confirmation; targeted tests then
passed for keeping the draft, Cancel, changing items and unchanged forms, plus
minimum/wide/200% layouts in both themes. Live verification is pending a native
build; the first attempt failed on concurrent file-index API changes.
Save-failure recovery and closing the entire app still need review; this change
does not promise crash recovery or durable draft storage.

Latest editor verification: the three targeted artifact tests passed after the
content-control change. An earlier confirmation-layout run passed all six
size/theme combinations. Re-running that layout check against the newest shared
tree stopped at a concurrent ADK visibility error (`file_index::evidence::model`
is private), before executing tests. The latest full CI attempt stopped at fmt
in the concurrently added `sovra-ui/examples/site_shot.rs`. Native verification
remains unavailable: initial builds failed against inconsistent core APIs, and
the final launch was rejected because another live development session owns the
checkout. Do not start a second GUI or claim the new editor flow was tested live.

Saved-work save recovery now waits for a matching Saved event before closing the
form. A failed send or Error event leaves the draft visible with retry guidance.
Typing remains possible during IO; a matching save does not close newer edits.
Five targeted artifact tests passed, including failed-save retry and newer typing.
The extended error/retry layout check has not completed successfully yet.

Build investigation found a separate HarnessSite scratch checkout running Cargo
with `CARGO_TARGET_DIR` pointing at this checkout's `target`. Its baseline core
outputs explain the repeated missing-API errors despite current source and an
intervening passing rebuild. Do not infer a source regression from those stale
outputs. The latest full CI attempt stopped on a redundant clone in ADK test code
(subsequently removed in the shared tree); full CI still needs a clean verification
run. The existing native GUI belongs to another live session, so live editor/save
verification remains pending. Coordination was requested to stop cross-checkout
writes to the shared target; source review can continue independently.

Next source-review findings: the schedule form still relies on placeholders and
hides its cadence examples in compact layouts. `submit_new_schedule` also clears
all inputs and closes the form immediately after queuing Create; verify failure
recovery against the scheduler reducer/runtime before changing it. The first-use
setup strip recommends a free OpenRouter key without explaining what a key is;
review that wording and the actual connection flow for complete beginners.

Schedule review found a runtime persistence defect, reproduced by a failing test:
creating a task on unavailable storage still published it as active. The scheduler
now rolls back failed commands and publishes the prior state; runs dispatch only
after persistence. Automatic failures wait 30 seconds before retrying instead of
repeatedly hitting unavailable storage. All five scheduler integration tests pass,
including create/retry, pause/delete rollback and no dispatch after failed Run now.
The schedule form itself still clears inputs optimistically and ignores errors;
retaining that draft and adding visible labels remain required next steps.

The full gate then reached 868 workspace tests (867 passed, one failed) at an
outdated knowledge fixture. Its exact expectations now include the new source
header as well as the original content; the focused integration test passes.
The full-gate rerun passed: 1,416 tests passed, 14 skipped, and all static gates passed.
This includes the previously pending artifact error/retry layout checks.
Native schedule-storage failure verification remains
unavailable in the other session's running GUI; the controlled-store runtime test
is the current evidence for this failure path.

Schedule forms now validate without inserting an optimistic task. The runtime
emits a specific Created acknowledgement only after persistence; a Snapshot alone
does not dismiss the form. Errors retain the entries and offer retry guidance.
Hide form keeps the draft for this session, Continue task reopens it, and a new
form after confirmed creation starts blank. Field labels, timing examples and
blank-timing behavior remain visible in compact layouts; inputs disable during
saving to preserve the submitted draft.

Targeted evidence: all 22 shell tests, the five scheduler integration tests and
the minimum/wide/200% form/retry layout checks passed. Through Development MCP
in an isolated profile, empty-instructions validation kept the name, Hide/Continue
kept the task, and confirmed creation closed the form. The schedule file contained
the intended task. The next form was blank; the temporary task was deleted and
its removal verified on disk. An OS capture confirmed readable form labels and
examples. Storage-failure retry is covered by controlled runtime/headless tests,
not by deliberately breaking the live profile's filesystem.

The shared-target conflict was worked around for verification using Cargo's
[workspace-wrapper cache separation](https://doc.rust-lang.org/cargo/reference/environment-variables.html).
The temporary wrapper passes compiler arguments through unchanged; it changes
cache identity, not source or compiler policy. Only UI package build outputs were
cleaned to reclaim disk space while no app/build was active. Full CI passed after adapting legacy tests that assumed immediate schedule
creation: 1,421 tests passed, 14 skipped, and all static gates passed.

AI setup wording now distinguishes a saved key from a tested service. It explains
API key in plain words, removes the claim that OpenRouter is the easiest/free
route, and accurately describes local key storage in both development and release
builds without naming the wrong storage backend. Research:
[W3C clear words](https://www.w3.org/WAI/WCAG2/supplemental/patterns/o3p01-clear-words/)
and [OpenRouter free choices](https://openrouter.ai/collections/free-models/).
The existing save/replace/remove integration test first reproduced the old status.
Native MCP verified the environment-key status, Back and Chat navigation; an OS
capture confirmed readable explanatory copy. No credential was changed or chat
request sent. An environment key still shows new-key instructions; improving that
branch and exercising provider-error recovery remain pending.
Full CI passed with this copy: 1,421 tests passed, 14 skipped, all static gates
passed. An earlier run failed the corrected-memory lifecycle assertion (zero
memories instead of one); its isolated rerun and the subsequent full runs passed
without changing that test. This intermittent failure remains worth investigating.

The existing-system-key branch now skips key-creation instructions and presents
model selection as the next step. Replacement remains optional and the UI does
not offer to remove a system key it does not own. A regression first reproduced
the detour, then verified checking the existing key, retry after an error and
continuing without a SaveConnection command. Native MCP confirmed that route.

Error-path inspection also found that Connections bypassed the model page's
error/notice renderer. Online and compatible-service editors now display these
results with stable accessible IDs. The regression reproduced the missing error;
native testing against an unavailable loopback address then showed the actual
failure, kept the address editable and left the connection unsaved. An OS capture
confirmed visible error text. The temporary draft was cleared and the test app
closed. Dedicated minimum/wide/200% checks of this new feedback branch remain
pending, alongside the broader feature review.
Full CI passed after this fix: 1,422 tests passed, 14 skipped and all static
gates passed. The preceding run ended with SIGTERM before its last test completed;
the final run was performed with the native test app closed.

Dedicated connection-feedback layout coverage now passes for both online and
compatible-service editors at 640×420, 2560×1440 and 200% interface size, in both
themes. The test checks horizontal bounds, scroll reachability, actual retry
commands, successful-check feedback and return navigation. Loaded preferences
explicitly retain each test's theme and scale, so the event cannot reset the
test to default appearance.

Next Skills findings from source inspection: the UI's automatic shortcut builder
discards non-ASCII letters, so an entirely Arabic name produces an invalid empty
shortcut. Creating a skill whose shortcut already exists also reaches the store's
replacement path without distinguishing creation from editing. Regressions and
a safe automatic-name rule are still needed; no fix is claimed yet.

The full gate for the connection layout test is currently failing intermittently
at `memory_lifecycle.rs:541`: corrected-memory recall returned zero facts instead
of one. The layout test itself passes. The runtime invalidates recall before
acknowledging a correction, so a simple acknowledgement-before-invalidation race
is not established. Further investigation must distinguish recall cancellation,
store revision changes and temporal visibility before changing code or tests.

Investigation found an independently reproducible recall-invalidation defect:
completing extraction with zero fact writes still incremented the database fact
revision. Concurrent recall rejects results when that revision changes, even
though this completion changed no facts. A real-engine regression failed with
revision 1 instead of 0; completion now advances that revision only when writes
are present. A companion assertion verifies real writes still advance it.
All 107 store tests passed (three ignored), including correction/removal checks.
Twenty isolated correction-flow runs passed before the fix, so those runs alone
did not reproduce the timing-dependent full-suite failure or prove its cause.
The regression proves the empty-job defect; full-suite verification follows.
Live verification of concurrent correction and background completion has not
been performed: another development GUI is running, and the race is covered by
deterministic store assertions rather than claims based on a single native run.
Full CI passed after the fix: 1,423 tests passed, 14 skipped and all static gates
passed, including the connection-feedback layout test. The first full run passed
the corrected-memory test but timed out on group-history witnessing over the real
network; that unchanged network test passed alone and in the final full run.

Custom-skill naming now uses a pure catalogue rule to choose an unused shortcut.
The visible title stays unchanged, including Arabic. Automatic names avoid both
names and aliases of enabled or disabled skills and retain the 64-character
limit. The new-skill form rejects explicitly entered shortcuts already present
in its catalogue; ordinary editing still uses the existing shortcut. This is a
form safeguard, not a new atomic create-only storage operation.
The headless regression failed on an Arabic title before the fix and now saves
it without advanced options, then verifies that a conflicting explicit shortcut
sends no save command. Core regressions cover fallback, collisions and truncation.
Native verification of this form change remains pending while another development
GUI is running; no second GUI was launched and that session was not disturbed.
