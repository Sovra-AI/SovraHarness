# Decks

Shared presentation frames for Sovra pitches. Each deck is its own folder with
one `index.html`. Three frames exist; pick the one that matches the audience.

## Frames

| Frame | Stage | Use |
| --- | --- | --- |
| **Cinema** (`shared/cinema.css` + `cinema.js`) | 1920×1080 | Investor pitches. Sovra light theme (paper `#fbfaff`, deep purple ink, violet `#7c3aed`), soft aurora tints, white glass panels, inline-SVG diagrams, live canvas scenes and a three.js hero. |
| **Editorial** (`shared/sovra-deck.css`) | 1920×1200 | Investor and capability briefs. Kyndryl deck craft — paper, one accent, kicker, cards, stats — remapped to the Sovra light palette. |
| **Sparse** (`shared/reveal/` + `deck.css`) | 1920×1080 | One-idea leadership slides. Vendored reveal.js 4.6.1, light `white` theme. |

A deck opens by double-clicking its HTML file. Stylesheets are normal links.
Scripts are classic scripts, not modules, because `file://` would block a
module import. Fonts are stored next to the frame. Editorial and Sparse need no network
fetch; Cinema adds Google Fonts and three.js from a CDN and still works
without them.

## Cinema deck (investor)

1. Copy `investor/index.html`, `group/index.html` or `value/index.html` into a new folder.
2. Keep this shell: `../shared/cinema.css`, `../shared/cinema.js`, and the
   Google Fonts and three.js (cdnjs r128) links in the `<head>`.
3. Each `<section class="slide">` takes `data-section` (top right) and
   `data-note` (footer). The engine adds the logo, the chrome and the page
   number. Add `data-bare` to opt out.
4. `.rv` elements reveal in order when a slide opens. `.draw` SVG strokes
   draw themselves. `[data-count]` numbers count up. A
   `<canvas data-fx="globe|net|train|mesh">` animates only while its slide
   is open. `data-three` upgrades a globe to three.js when the CDN loads and
   falls back to the 2D globe offline.
5. Keys: arrows / Space / PageUp / PageDown, Home / End, `F` fullscreen,
   `O` overview grid, `#7` opens slide 7, swipe on touch, click the sides or
   the on-screen arrows. `prefers-reduced-motion: reduce` stops looping
   backdrop, flow packets and canvas so copy is fully visible at rest.
   Printing gives one 1920×1080 page per slide.
6. Offline, the slides fall back to the vendored IBM Plex fonts and the 2D
   globe. Product screenshots are linked from `docs/evaluations/`, not
   copied.

## Editorial deck

1. Start a new folder with one `index.html` that links the shell below.
   (The investor deck moved to the Cinema frame; the editorial CSS stays
   for capability briefs.)
2. Keep this shell:
   - `../shared/sovra-deck.css`
   - `../shared/sovra-deck.js`
3. Do not copy those files into the deck folder.
4. Open `index.html`. Arrow keys, Space and a click on the right half move
   one slide at a time. `F` is fullscreen. `#7` in the URL opens slide 7.
   Print (`?` or the browser print dialog) stacks every 1920×1200 page.

Colour comes from the product light theme: SRX paper `#faf5ff` / `#fbfaff`,
ink `#3b0764` / `#581c87`, accent `#7c3aed` (the same violet as
`sovra-ui` `design::palette`). Type is vendored IBM Plex Sans, Serif and
Mono.

## Sparse deck (one idea)

1. Copy `company-computers/index.html` into a new folder.
2. Replace the `<section>` slides. Keep this shell:
   - `../shared/reveal/dist/reveal.css`
   - `../shared/reveal/dist/theme/white.css`
   - `../shared/deck.css`
   - `../shared/reveal/dist/reveal.js` then `../shared/deck.js`
3. Do not copy those files into the deck folder.
4. Open `index.html`. Arrow keys and the on-screen arrows move one slide at
   a time. The stage is 1920×1080 and scales to the window at 16:9. The
   slide count sits along the bottom.

Sparse slides stay one idea: a short headline and one or two lines of type.
Headlines stay at least 40px and every other readable string at least 28px,
at the 1920×1080 stage.

## What a slide may say

- A line that says **Today** is only for what the desktop app already does:
  a helper on the computer, memory you can open, a correction that replaces
  instead of erasing, and helpers that answer together.
- A line that says **Next** or **The direction, not today** is not current
  behaviour. Use **Next** for the closer-in group of computers (join, lanes,
  helper sharing, work handoff). Use **The direction, not today** for the
  later company-fleet story.
- Do not invent a market size, revenue, headcount, or a fundraising ask.
- No tour of libraries.

The `pitch/`, `value/` and `uae/` decks add a deck-local layer on top of the
Cinema frame: deep-ink bars outside the 16:9 stage, larger type, **ink slides**
(`class="slide ink"`, a deep-purple stage for titles, dividers and the moments
that matter, with the particle-nation hero via `<canvas data-hero data-ink>`),
and **no boxes** (cards, chips, tiles and bars become typography on hairlines; diagrams
use dots, rings and glows). Copy that layer rather than re-inventing it.

Every frame uses the light paper theme, not a near-black canvas. The first
Cinema cut was dark; it is in git history (`e4142d1`) if it is ever wanted.

The mark on every slide is the product logo,
`crates/sovra-ui/assets/branding/sovra-logo.svg`, linked with `<img>` so
its colours stay. Do not copy it and do not redraw it.

## Decks

The written source behind the `value/` and `uae/` decks, with code evidence
for every Today claim, is [`docs/value-realisation/`](../value-realisation/README.md).

| Folder | Frame | Audience |
| --- | --- | --- |
| `investor/` | Cinema | Investors. 31 slides: sovereignty, what ships today (with real counts and a real screenshot), then the direction: the organisation as a ledger, fleet search, and proof-of-learning training. |
| `company-computers/` | Sparse | Company leaders. Keep the work on computers you already own. Fleet, ledger, and in-place training are the direction, not today. |
| `group/` | Cinema | A family or a 3–20 person business. [A few computers share helpers and memory](group/index.html). Chrome, cards and diagrams. Today is the helper on one computer; next is a group of people and their computers: join, lanes, helper sharing, work handoff. No central Sovra server. People-and-computers are a **group**; **team** stays the AI helpers. |
| `value/` | Cinema | Anyone judging whether Sovra is worth using. [Value you can count](value/index.html): value is work that is finished, kept with versions, and easier the next time. The first day, everyday value moments, why memory compounds, sovereignty as how the value stays yours, and where to count it (Artifacts, Knowledge, Schedules, approvals). No invented ROI; the compounding chart is labelled an illustration. |
| `uae/` | Cinema | UAE leaders (government, Chief AI Officers, regulated sectors). [AI-native, 100% sovereign, value you can count](uae/index.html). Non-technical value-realisation deck: the UAE's AI-native vision, five things that must stay sovereign (data, knowledge, the AI itself, decisions, learning/training) with Today/Direction status, a value map, staged time to value, a pilot with a baseline and stop criteria, a shared success plan, honest limits (training is direction; Arabic chrome ships today) and a sources slide. Outside figures are cited with dates; law summaries are not legal advice. |
| `pitch/` | Cinema | Investors, non-technical. [The investor pitch](pitch/index.html): 19 slides with a deliberate rhythm. Deep-ink slides (title, the question, the problem, proof, why Sovra wins, close) alternate with light poster slides where a scene fills the stage, one-giant-number slides, an editorial numbered list, a section divider and a giant-type ask. Scenes: the particle-nation hero (three.js), the border the work leaks across and then stays inside, the isometric sovereign stack (slide 4: owning that stack is the play), the orbit of five things that stay yours, the constellation, the sealed ledger, lessons flowing home, the market map. No Today/Next labels; limits live in `docs/value-realisation/`. The funding amount is for the founder to add. |
