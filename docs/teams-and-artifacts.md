# Teams and artifacts

How a group of helpers answers in chat, and how anything worth keeping is saved
as an artifact.

Two features that work together:

- **Teams:** a group of AI helpers, each with a different speciality, who answer
  together in chat.
- **Artifacts:** a library of everything the user kept from chats — documents,
  checklists, tables, code, diagrams, web pages, pictures, data and notes.

Both run inside the single desktop binary, on whichever AI the user picked in Models,
and are designed for someone with no technical experience.

## Ask a team and save work

1. Open **Team**. Each card is a person: a face, a name, a job, and one line
   about how they sound. Open a card to chat with that helper, switch them on
   or off, edit them, remove a helper you added, or add them to a group. The
   opened card says what they focus on and where they step back. **Ask my team**
   and **Start a group chat** stay on the page. **Ask my team**, or the helpers
   button in the chat box, opens a new chat and leaves the one you were in
   alone. Ask everyone stays off until you turn it on. A new team starts with
   Alex, Morgan, Priya, Jamie, Sam, Robin, Noor, Kai, Scout (looks things up on
   the web), Quinn (prices and trade-offs), and Mara (the right words on the
   right channel). **Add a helper** also offers Engineer, Designer, Sales,
   Operations, Skeptic, Visionary, Diplomat, and Hustler, plus the helpers that
   were already there. A team you already saved keeps your edits and anyone you
   switched off, and gains any of these helpers that was missing.
2. A question can ask one helper or several. The same helper can answer two
   parts at the same time. A stray word does not pull in an extra helper when
   someone else is the stronger match. Turn on **Ask everyone** when you want
   the whole group to answer. **Ask Sovra only** returns to a normal chat.
   Each chat keeps its own helpers. A later round only calls helpers who are
   already in that chat.
3. When several helpers speak, Alex joins. Alex stops if the next round repeats
   the last one, adds little, or they are unsure. Watch the **Waiting / Working /
   Done** board. A helper with nothing to add is named on their card. Use Stop
   to interrupt a run.
4. Choose **Save** on a reply or code block to keep it in Artifacts.

Helpers use the model selected in Models. Members selected for a round work in
parallel, then Alex decides whether another round is needed. From the second
round, a continue that repeats the previous round, adds little new wording, or
is unsure hands the chat back. A turn stays within three rounds and eight
replies. A helper can pass without showing protocol text. Team replies retain
their authorship; see [team memory](memory-capture-and-recall.md#one-team-turn).

In **Artifacts**, use folders, favorites, search, and sorting to find saved work.
Editing keeps versions. Back and Cancel ask before discarding unsaved edits;
a failed save keeps the draft. Deletion is soft and asks for confirmation.
See the website section below for browser previews and build requirements.

**Artifacts → Decisions** keeps a company decision as its own record, with
criteria, cited wording, a second person's acceptance and retained versions.
Chat replies also offer **Record a decision**. See [Decision records](decisions.md).

## Everything streams as AG-UI

A team turn is a multi-agent run, so it uses the AG-UI protocol's own events
(<https://docs.ag-ui.com/concepts/events>). They are typed in
`sovra_core::AgUiEvent` and sent as `Event::AgUi { session_id, event }`:

| AG-UI event | Meaning in a team turn |
| --- | --- |
| `RUN_STARTED` / `RUN_FINISHED` / `RUN_ERROR` | The whole turn starts, finishes or fails; the composer unlocks. |
| `ACTIVITY_SNAPSHOT` (`activityType: "sovra.team"`) | The live team board: the round, a plain-words status, and each member's card in a **Waiting / Working / Done** lane, with its latest step ("Searching the web for …") and step count. |
| `SUBAGENT_STARTED` / `SUBAGENT_FINISHED` | One member's invocation. `outcome` is `success`, `passed` or `failed`; `speaker` carries the name, emoji and role shown in chat. |
| `TEXT_MESSAGE_START` / `CONTENT` / `END` | The member's reply, attributed through `subagent_run_id`. |

Members chosen for the same round **answer side by side**: each sees the
conversation as it stood when the round began, their replies stream at the same
time, and a member's card moves to Done as soon as it finishes. The Conductor
then reads all of them. Reply slots and ids are assigned before the round
starts, so the reply cap and the event ids do not depend on who finishes first.
Every member loop has the same guard rails as the rest of Sovra; see
[The agent loop](agent-loop.md).

Two kinds of text never reach the screen:

- **A member's pass.** `teams::VisibleStream` holds back anything that could still
  turn into `<pass/>`, so a pass never flashes up.
- **The Conductor's verdict tail.** The same filter holds back a possible `[[CONDUCTOR:`
  prefix. Once the tail starts, the stream is hidden, and the tail is parsed when the
  reply ends.

When an agent saves an artifact through the `save_artifact` tool, the chat shows an
AG-UI surface widget, `AgUiWidget::ArtifactSaved`, as a card with an **Open** button.

## Layers

The feature follows the repo's layering rules (`AGENTS.md` → Architecture):

| Crate | Teams | Artifacts |
| --- | --- | --- |
| `sovra-core` (pure) | `teams.rs`: members, presets, speaker selection, Conductor parsing, `VisibleStream`, `TeamActivity`, `TeamRoster` reducer | `artifacts.rs`: `ArtifactKind` + one `KINDS` row per kind, extraction (`find_pieces`, `draft_from_reply`), versions, CSV export, browser page, `ArtifactLibrary` reducer |
| `sovra-adk` | `team.rs`: `AdkHarness::run_team`; each member is a plain single agent in a throwaway session | `artifacts.rs`: the `save_artifact` tool and `ArtifactSaver` (injected, like `SearchClient`) |
| `sovra-runtime` | `teams.rs`: roster persistence, starting team turns | `artifacts.rs`: save, edit, delete, export, browser view |
| `sovra-store-surreal` | `team:roster` record | `artifact` table, soft delete |
| `sovra-ui` | `team_view.rs`, the team toggle in the composer, speaker avatars in chat | `artifacts_view.rs`, the **Save** buttons on replies and code blocks |

### Adding a new artifact kind

1. Add a variant to `ArtifactKind`.
2. Add a row to `KINDS` with its label, plain explanation, extension, MIME type, and
   whether it opens in the browser.
3. Map languages to it in `ArtifactKind::for_language`.
4. Pick an icon in `artifacts_view::kind_icon`.

The compiler lists every `match` that needs the new variant.

## UX rules (plain words for non-technical people)

- **Plain words, no jargon.**
  - "Your team", "Ask my team", "Taking a break" — never "persona", "agent" or "subagent".
  - "Save" and "Save to my computer" — never "export".
  - "Open in browser".
  - The live card says "Priya is answering…" and "Nothing to add".
- **One obvious action per place.**
  - The Team page leads with the helpers. **Ask my team** and **Start a group chat** stay on the page. Chat, on/off, edit, remove, and group pick are on the opened helper.
  - Each reply has **Save**.
  - Each artifact has **Copy**, **Save to my computer** and, for web pages, diagrams
    and pictures, **Open in browser**.
- **Never lose work.**
  - Editing an artifact keeps the old content as a version.
  - Deleting is soft in the store and needs a second click ("Click again to delete").
  - Built-in helpers are switched off rather than deleted, and **Restore original**
    brings back their settings, including how they sound and where they step back.
  - A saved team gains any missing shipped helper. It does not overwrite an edit
    or switch someone back on.
- **Team mode is never a surprise.** While it is on, a note above the chat box says
  so and offers **Ask Sovra only**.
- **Nothing is executed inside the app.**
  - HTML, SVG and Mermaid are previewed as source.
  - **Open in browser** serves a throwaway preview at a loopback HTTP address.
  - Mermaid diagrams load `mermaid@11.4.1` from jsDelivr, so they need an internet
    connection to render.

## Tests

- **`sovra-core` unit tests:**
  - speaker selection;
  - Conductor parsing;
  - the stream filter (passes, the tail, Unicode);
  - instructions;
  - the reducers;
  - artifact extraction, title, CSV and browser-page rules.
- **`sovra-adk`:** a scripted model checks a moderated round. It covers the AG-UI event
  order, a hidden pass and hidden tail, member outcomes and the final team card, plus a
  single-member turn.
- **`sovra-store-surreal`:** the roster and artifacts round-trip, and soft delete works.
- **`sovra-runtime`:** through `Client`, the roster is seeded, edited, switched off and
  reset. A saved roster that already edited Alex and switched Morgan off loads again
  with those changes and with Scout, Quinn, and Mara. Artifacts are saved, edited,
  exported as CSV, opened and deleted. A team turn with no AI connected explains how
  to connect one.
- **`sovra-ui`:** headless tests cover:
  - the Team page at 640×420, at 2560×1440 and at 200% text size, in both themes;
  - "Ask my team" sending a team run;
  - routing by `@mention`;
  - AG-UI events rendering member replies and the team card;
  - **Save** on a reply;
  - library filters, open, open-in-browser and delete;
  - the Artifacts page fitting at every size.

## Writing and running websites locally

The coding helper is instructed to deliver complete source using semantic HTML,
labelled controls, keyboard focus, responsive layouts, reduced-motion support,
and working interactions. The parent saves substantial code and websites through
`save_artifact` before reporting completion. Completed fenced source is also saved after a successful chat turn when the model
did not call the save tool; unfinished fences are left as previews. Storage
failures are shown as errors, never as successful saves. These are generation instructions,
not a claim that generated code has passed automated browser tests.

Visible fenced code streams as `ACTIVITY_SNAPSHOT` with activity type `sovra.code`.
The native source panel displays up to 32,768 characters, and never executes the
source. Saved content is not truncated to that preview limit. Wide windows dock
the panel; narrow windows show it over chat with **Back to chat**.

After saving, ask to run the website, use **Run website** in the panel or Artifacts,
or try the **Run your website on this computer** skill. `deploy_artifact` uses a
saved artifact ID, or the latest saved web page from the same chat. The runtime
serves an immutable HTML snapshot from a new ephemeral IPv4 loopback port. It
opens that address in the system browser. **View website** reopens it; **Stop
website** closes hosting. Editing saved source does not change an already running
snapshot; run it again to serve the new version. Closing Sovra stops hosting.

Self-contained HTML is served directly. `project` artifacts store JSON with
`files` (relative paths to complete text source) and `output_dir` (default `dist`).
Include `package.json` with dependencies and a static `build` script. React/Vite,
Vue, Svelte and other static frontend builds use **Build and open**. Sources stay
in one artifact, with file selection in its detail view; whole-project edits
retain versions. JSON copies preserve the project when saved to the computer.
Builds require installed Node.js/npm and enabled terminal access. `npm install`
(or `npm ci` with a supplied lockfile) and `npm run build` go through the existing
terminal approval flow, with lifecycle scripts disabled except the requested
build script. Commands and failures appear in chat. Source is written into a fresh
private temporary folder, and bounded static output is loaded into memory before
that folder is removed. Backends and server-side rendering are not hosted.

**Open in browser** also serves HTML/SVG/diagram previews over HTTP, avoiding
browser sandbox restrictions on temporary file URLs. Folders and favorites are
persisted as artifact metadata (older saves default to Unfiled); filters and sort
order are pure core rules, and the UI shows 24 results per page.

The browser server serves only its in-memory static snapshot. It exposes no shell, filesystem
routes, or development-control endpoints. Host headers must match the exact local
address. Requests, connections, page size, and simultaneous sites are bounded.
Websites can still make network requests from the browser if their source does so.
Local hosting is not public publishing.
