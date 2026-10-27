# Teams and artifacts

Two features that work together:

- **Teams:** a group of AI helpers, each with a different speciality, who answer
  together in chat.
- **Artifacts:** a library of everything the user kept from chats — documents,
  checklists, tables, code, diagrams, web pages, pictures, data and notes.

Both run inside the single desktop binary, on whichever AI the user picked in Models,
and are designed for someone with no technical experience.

## Where it comes from

Teams are ported from `SovraSRXProduction` team chats (the "Team" pages and
`backend/src/app/services/team_chats`). There, a team member is a *persona*: name,
icon, tagline, profile and instructions. A **Conductor** (Alex) moderates group chats
in rounds and ends each reply with a machine-readable verdict tail:

```
[[CONDUCTOR: {"verdict":"CONTINUE|YIELD_TO_USER|ASK_USER","next_speakers":[...]}]]
```

In moderated chats, members may reply exactly `<pass/>` when they have nothing to add.

What this app keeps from that design:

- The seeded teammates, minus the ones that drive tools this app does not ship yet.
- The order in which the first speakers are chosen:
  1. `@handle` mentions,
  2. names,
  3. "everyone" words,
  4. otherwise the members whose expertise best matches the prompt (no classifier call).
- The Conductor protocol and passes.
- A cap on rounds and replies per turn.

What changes:

- `MAX_ROUNDS` is 3, not 6, because the user pays for every reply on their own key.
- Every member uses the user's selected model.
- Memory for each member is not ported yet.

Artifacts follow what AI clients have converged on:

| Source | Artifact kinds |
| --- | --- |
| Claude artifacts | Markdown, code, HTML, SVG, Mermaid |
| ChatGPT / Gemini Canvas | Documents and code |
| `SovraSRX` `user_artifacts` | Note, document, code, data, spreadsheet, link |
| A2A protocol | Artifacts are typed parts attached to a task |

## Everything streams as AG-UI

A team turn is a multi-agent run, so it uses the AG-UI protocol's own events
(<https://docs.ag-ui.com/concepts/events>). They are typed in
`sovra_core::AgUiEvent` and sent as `Event::AgUi { session_id, event }`:

| AG-UI event | Meaning in a team turn |
| --- | --- |
| `RUN_STARTED` / `RUN_FINISHED` / `RUN_ERROR` | The whole turn starts, finishes or fails; the composer unlocks. |
| `ACTIVITY_SNAPSHOT` (`activityType: "sovra.team"`) | The live team card: the round, a plain-words status, and each member's state (waiting, answering, done, nothing to add, couldn't answer). |
| `SUBAGENT_STARTED` / `SUBAGENT_FINISHED` | One member's invocation. `outcome` is `success`, `passed` or `failed`; `speaker` carries the name, emoji and role shown in chat. |
| `TEXT_MESSAGE_START` / `CONTENT` / `END` | The member's reply, attributed through `subagent_run_id`. |

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
  - The Team page has one primary button, **Ask my team**.
  - Each reply has **Save**.
  - Each artifact has **Copy**, **Save to my computer** and, for web pages, diagrams
    and pictures, **Open in browser**.
- **Never lose work.**
  - Editing an artifact keeps the old content as a version.
  - Deleting is soft in the store and needs a second click ("Click again to delete").
  - Built-in helpers are switched off rather than deleted, and **Restore original**
    brings back their settings.
- **Team mode is never a surprise.** While it is on, a note above the chat box says
  so and offers **Ask Sovra only**.
- **Nothing is executed inside the app.**
  - HTML, SVG and Mermaid are previewed as source.
  - **Open in browser** writes a throwaway page to the OS temp directory.
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
  reset. Artifacts are saved, edited, exported as CSV, opened and deleted. A team turn
  with no AI connected explains how to connect one.
- **`sovra-ui`:** headless tests cover:
  - the Team page at 640×420, at 2560×1440 and at 200% text size, in both themes;
  - "Ask my team" sending a team run;
  - routing by `@mention`;
  - AG-UI events rendering member replies and the team card;
  - **Save** on a reply;
  - library filters, open, open-in-browser and delete;
  - the Artifacts page fitting at every size.
