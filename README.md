# SovraHarness

> **Closed source — proprietary to Sovra.** Copyright © 2026 Sovra. All rights
> reserved. Do not copy, distribute, or disclose outside Sovra. See [LICENSE](LICENSE).

Native Rust desktop app for the Sovra agent harness. GPUI (via `gpui-kit`) UI, tokio runtime,
embedded SurrealDB memory (vector + graph + full-text in one engine, behind a swappable
`MemoryStore` port), Velopack installers and update hooks. macOS, Windows, Linux from one
codebase.

## Quick start

```sh
just dev
```

Or without `just`:

```sh
cargo xtask setup
cargo xtask dev
```

On Linux, install system packages once first with `./scripts/install-linux-deps.sh`.
That's it — no Node, no Python, no system packages on macOS/Windows.

## Test with Codex or Claude Code

Enable **Settings → Development mode**. The top bar explicitly shows when local
automation is enabled. Add the command copied from Settings as a **stdio MCP**
server in your coding assistant. It can inspect and operate native controls and
capture screenshots; switching off revokes access. The existing development
watcher keeps running. See [connection instructions and testing workflow](docs/development-mcp.md).

## Everyday commands

### Skills in chat

Use `/index-folder` or **Knowledge → Files → Add folders** to select one or more
folders for local indexing. The selection is registered together, then scanned in
the background. Failed additions show their paths and reasons with **Retry failed
folders**; successful folders stay connected. Existing folders keep their settings.
**Knowledge** puts a live folder summary first. While a folder is read it shows
"Finding your files…" while it counts, then a progress bar with "1,204 of 5,000 files
read", a rounded time estimate once it is reliable, the file being read now, and **What
I've found so far** — files sorted into plain groups (Photos & pictures, Documents &
letters, Bills, receipts & statements, …) and everyday life areas (Money, Health,
Travel, …) guessed from folder names. Problems appear as one line with **See why**. Each
group is linked to open standards (DCMI, schema.org, NEPOMUK, IPTC); see
[docs/file-ontology.md](docs/file-ontology.md). Files are read in parallel on all but one
core with bounded memory. The card provides Cancel, Refresh, Pause/Resume updates,
Remove, and text search. Enter or Search files searches saved text, with a clear result
message when nothing matches. Optional meaning search is under **Folder settings**.
While Sovra is open, native folder notifications trigger updates and a full check
every 15 minutes catches missed changes. Large scopes or unavailable notifications
fall back to periodic checks, shown in Knowledge. Cancel pauses automatic updates;
Refresh performs a manual scan. Cancel and Pause also signal a document being read;
it stops at the reader's cancellation checkpoints and keeps the previous indexed
version. Removing an index
leaves the original files intact. Each folder's **Scan report** pages through every
recorded skip, unreadable file and failure from its latest scan, with path/reason
filters and incomplete-scan labels. The compact settings summary shows at most 50
examples; older indexes need a refresh to create a full report.
**Partial text** in Browse files or Scan report finds files with known reading
problems, such as a missing presentation slide. Usable text stays searchable;
reader details explain the gap in previews and citations. Repair the original and
refresh to replace the partial version.
Unreadable subfolders do not stop accessible folders from updating. An incomplete
scan keeps older entries because it cannot confirm removals. Check its report,
restore access, then use **Retry scan**; a successful retry reconciles missing files.
Large UTF-8, BOM-marked UTF-16/32 and detected legacy text/source files
are indexed completely with fixed-size buffers, including beyond the attachment
limits. UTF-16 without a BOM is inferred conservatively. Inferred encodings are
labelled; the saved-text preview lets you check the decoded characters. Folder indexing
has no fixed 16-folder, 100,000-entry, 1 GB scan/database or 64-level traversal cap;
available storage and OS resources still apply. Office documents can contain more
than 100 internal package parts; their format is recognized regardless of extension.
Indexed PDFs and presentations have no fixed 500-page/slide cutoff. Indexing reads
their text layer; scanned pages need text recognition (OCR). Files without text
remain in Browse files with an explanation. **Folder settings → Document reading**
lets you choose a memory budget: 64 MiB, 256 MiB (default), or 1 GiB per source
file, with extracted-text budgets of 16, 64, or 256 MiB. Larger choices use more
memory and refresh the folder. Indexed archives have no fixed 100-entry cutoff;
total unpacked bytes, nesting and codec-specific protections still apply. Chat
attachments keep their separate limits.
Folder details let you include hidden files and
generated folders, which are excluded by default. Switching these options off
removes matching files from current search while preserving saved history.
Credential folders and symlinks stay excluded.
Use **Browse all files** or a folder's **Browse files** in Knowledge to inspect the
saved inventory without a keyword search. Filter by filename or reading status,
page through results, and use **View details** for extraction explanations and
the saved text. **First part**, **Previous part**, **Next part** and **Last part**
let you read every saved part without loading the whole document. Parts are
sections of extracted text, not original document pages; partial-read warnings
still apply. A changed source version requires reloading the list before reading
its replacement. Excluded folders and links are explained in folder
notices; they are not inventoried as readable files.
Filename and scan-report filters ignore capitalization by default, including
Unicode letters and equivalent accented spellings. Select **Match case** for
exact capitalization. Filters use literal text, so `%` and `_` are not wildcards.
Searches and catalogue queries can include more than 16 connected folders; result
pages remain bounded and retain the selected scope.
Binary files that cannot be decoded keep their filename and size in the inventory,
with guidance in View details and Scan report. A text-like extension does not make
binary fragments searchable document text. Attach the file in chat for byte
inspection, or export a readable copy and refresh the folder.
SQLite databases and binary property lists use the folder's document reading
budget too. Database indexing can read beyond 100 tables and 10,000 rows; if it
stops at a budget, saved rows remain available with **Partial text** status and a
scan-report explanation. Increasing the document budget and refreshing can read
more; database work time and native format limits still apply.
Folder summaries flag notices even when Folder settings is closed, so a completed
scan does not hide skipped or unread content. Open **Scan report** for the reasons
and recovery steps.
See [scope and current limits](docs/skills-and-file-knowledge.md).

Chat helpers and team members can search connected folders, read cited passages,
inspect recorded file history and follow reviewed file connections. They also
receive your standing instructions and can look up saved memories when remembering
is enabled. Delegated helpers can read files attached to the current message;
old attachments are not silently carried forward. Removing a folder stops later
reads. Independent delegated tasks overlap, with at most four running per turn;
team discussions stay ordered so each reply can build on the previous one. See the
[context and concurrency audit](docs/agent-memory.md#helper-context-and-concurrency-audit-2026-11-01).

Type `/` to browse skills. Arrow keys select, Tab or Enter chooses, and Escape
closes suggestions. Choosing a model procedure fills the draft so you can add a
request before sending. `/files` (also `/file-system`) opens the attachment picker;
`/read-files <question>` searches attached content; `/search-files <question>` retrieves
cited snapshots from indexed folders; `/file-catalog [question]` lists, filters and
groups indexed files by folder, name, type, size or modification date. With no
question it shows available folders and counts. `/file-history <file or question>`
inspects recorded changes and reads historical text with source-version citations.
History starts with observations Sovra actually recorded; it is not a complete
filesystem journal or a backup of original file bytes. `/web-search <question>` requests
web research. Native presentation tools also have commands such as `/table` and `/card`.

`/map-files <project or relationships>` proposes cited document, section, topic,
person, organization, project and task relationships. Review the exact relationships,
source excerpts, confidence estimates and dates in **Knowledge → Review file knowledge**,
then choose **Save inferences** or **Reject**. Approval keeps their model-inference
label; it does not verify their truth or edit source files. `/file-knowledge [question]`
explores saved relationships and reads their evidence. Changed source versions are
excluded by default; historical queries must request and label them explicitly.
Removing a folder revokes dependent knowledge permanently, even after re-adding it.
The inaccessible review history remains in the local database.

Source-linked summary storage is under development. It retains provenance,
invalidates dependent summaries and enforces generation budgets, but background
summary generation, recursive clustering and compiled maps are not available yet.

For meaning-based file search, open **Knowledge → Set up semantic search** on a folder.
Choose an installed Ollama embedding model (inspect it in Models to load its
capabilities), an OpenRouter search helper, or an embedding model on an
OpenAI-compatible connection. The choice
shows which server receives extracted text and future searches; usage may incur
provider charges. Background indexing resumes while the app is open, and both
Knowledge search and `/search-files` combine semantic and keyword results. Progress,
incomplete coverage and failures remain visible. Disable semantic search to stop
requests and remove that folder's vectors. Requests already received by a server
cannot be recalled. No model is downloaded by this feature.

Open **Skills** or use `/skills` to inspect built-ins, create a personal procedure,
import a folder containing `SKILL.md`, or enable, disable and remove user skills.
Skill names and aliases are unique. A skill cannot enable unavailable tools, and
importing a package does not execute its scripts. Keyword indexing stays local; chat retrieval
sends requested file metadata and matching passages to the AI service you selected. Installed packages live in the
app's local data directory. File indexing and semantic file knowledge are still
being implemented; [the scope record](docs/skills-and-file-knowledge.md) tracks them.

### Development

| Command                  | What it does                                                 |
| ------------------------ | ------------------------------------------------------------ |
| `cargo xtask dev`        | Persistent live reload: watch, rebuild/relaunch, stream logs  |
| `cargo xtask run`        | Launch the app (debug)                                       |
| `cargo xtask ci`         | Every gate: fmt, clippy, layering, cargo-deny, unused deps, tests |
| `cargo xtask fix`        | Auto-apply fmt + clippy fixes                                |
| `cargo xtask check-styles` | Check native styling boundaries and the bundled SVG logo  |
| `cargo xtask setup`      | Install pinned CI tools and Watchexec                       |
| `cargo xtask dist`       | Release build and verified native installer; requires Velopack `vpk` 1.2.161 |

Logs stream directly in the dev pane, with files under the OS data dir
(`~/Library/Application Support/com.Sovra.SovraHarness/logs` on macOS). The runner
preserves an existing `RUST_LOG` filter and defaults to focused application debug logs.

For development, use the existing **`sovra:sovraRust`** tmux window: the watcher and
live logs stay running there. Save Rust or embedded asset changes to rebuild and
relaunch automatically; no manual start/stop is needed. This resets in-memory
state rather than hot-swapping Rust code. Setup and operating rules:
[`docs/development.md`](docs/development.md).

Development builds use a separate plaintext key store in the app data directory
when `SOVRA_DEV=1` is set by the runner. Release builds always use the OS keychain.
See the development guide for storage and permission details.

Signing in is built in. At startup the app restores the saved session and opens
the workspace, or shows the sign-in screen. On a fresh install the first submit
creates the account (email + password, protected with Argon2id on this computer);
afterwards the same form signs in, with friendly inline errors from the runtime.
**Continue with Google** opens Google's own consent page in your browser and the
screen waits, with **Open the page again** and **Cancel**, until the sign-in
finishes by itself. It uses the desktop client saved in Settings, or the optional
`SOVRA_GOOGLE_CLIENT_ID` and `SOVRA_GOOGLE_CLIENT_SECRET` configuration. Signing out
keeps accounts but clears the session. Remaining rollout work:
[sign-in implementation status](docs/development.md#sign-in-integration-status).

The **Models** tab guides you through two steps: connect a service, then pick a chat
model. It shows only the next needed setup action, with suggestions labeled **Free**,
**Everyday**, and **Thinks longer** from the live catalog, each priced in plain words
("About 1¢ a message"). **Get an API key ↗** opens OpenRouter when a connection is
needed; Sovra checks the key with OpenRouter before saving it, and saving moves
straight on to picking a model. **Use this model** takes you straight into the
chat, and a message you typed before setup finished is sent for you.
When a model is already selected, **Change** on that card lists the other
models already loaded. Each one shows an everyday price and a plain sentence
about what it is good for, taken from what the service listed. If a price or
an ability is not listed, the choice says so. Choosing one switches to it and
stays on Models. The search box on that list finds a model by its name, its
price, or what it is good for.
**Browse all models** lists chat models by default, each labelled by what it makes;
the **Show** picker lists pictures, video, voices and other types. Every row has a
1-click **Use Model** / **✓ Active** button, plus optional Ollama connections,
favorites and comparison; filters stay closed until needed. Setup, authentication and capability details: [Model library](docs/models.md).

A single **Unified Left Sidebar** (`sidebar.rs` + `chat_history_view.rs`) houses **New chat** at the top, primary navigation (`Chat`, `My AI's`, `Knowledge`, `Models`, `Schedules`, `Settings`), a **Take a tour** launcher, instant chat search, `Chats | Scheduled` filter pills, and date-grouped conversation history (`Pinned`, `Today`, `Yesterday`, `Previous 7 Days`, `Previous 30 Days`, `Older`). After the first exchange in any conversation, the active LLM automatically generates a concise 3–6 word title (`Command::GenerateChatTitle` / `Event::ChatTitleGenerated`), and the selected row exposes named icon actions for generating a title, pinning, renaming, and deleting.

The **My AI's** tab lets anyone choose how Sovra picks an AI, in plain language:
- **Automatic:** a fast decision engine (**Laya**, **OpenJev / TypeSafe Jev**,
  or the **Embedded Rust Heuristic**, with offline fail-open and auditable
  `rc1-...` route codes) simulates which model to choose for each task. Chat
  currently uses the model selected in **Models**.
- **One AI for everything:** use the model picked in **Models** for every task.
- **Choose per task:** set effort (Quick / Balanced / Thorough) for each of the 22
  `SovraSRX` specialists.

The engine choice, routing tree and prompt tester are folded under
*How automatic choice works → Show details*.

The **Knowledge** tab opens to **Files**, with **Add folders**, local search,
folder controls, and focused file browsing and scan reports. **Memories** holds
saved-memory controls. **How it works** explains how Sovra remembers, with one worked example a
new reader can follow: something happens, a record is kept, a current belief is
formed, a later belief replaces it, and the earlier belief stays as history.
The explanation names who or what the belief is about. Finding a memory uses
matching words, with optional meaning search that you can enable in Memories. It is an explanation, not the
chat, and it does not need a memory store to be open. The times Sovra keeps are
under *Show the times Sovra keeps*.

Conversation memory is being built separately from file knowledge. Its embedded
store preserves source episodes and correction history, commits fact changes
atomically, and checks embedding compatibility across restarts. Its persistent
ingestion API supports stable input keys, recoverable leases, bounded retries,
atomic batches, and cancellation when paused; it retains model/evidence attribution
and suppresses exact duplicates. The selected model can also recognize different
wordings of an earlier detail, keeping the original fact and each message's evidence.
Matching distinguishes people and explicit dates; corrections keep a new version
even when returning to an earlier value. Previously recognized wordings stay
suppressed after Stop using, including after restart. New paraphrases may still
be missed, and matching accuracy remains under evaluation.
An owned background worker now uses the selected
chat model to extract supported details from user messages, including team prompts.
Accepted messages are written to the memory queue before normal shutdown finishes;
interrupted extraction can resume when Sovra reopens. Independent keyword lookups
run up to four at a time, and capture reads recent context alongside those matches.
Extraction and corrections stay ordered so later messages see earlier updates.
Models advertising structured-output support receive an extraction schema where
the provider adapter supports it; every result still needs an exact source quote
and local validation. Useful stated details can concern named people and projects
as well as the user.
Extraction guidance keeps a stated change date with the resulting saved state,
without applying it to unrelated details or treating tentative plans as confirmed
changes. Date interpretation still depends on the selected model.
Remember across conversations in Settings or Knowledge pauses new work and cancels pending extraction; saved history
is kept. Knowledge shows live status and failures. Your selected AI service receives
the message and up to eight earlier memories; the resulting records stay locally stored.
Keyword-matched saved details can now inform ordinary, attachment and team replies.
Matches can include a little related context about the same named person or project,
such as identifying whose preference was saved. This lookup is bounded and may miss
older relationships; it still respects dates and Stop using.
Each request receives bounded quotes and source references, outside saved provider
history. Completed cited replies offer **Saved source** buttons to inspect the
original message and whether that saved version is current, previous or stopped.
Missing references show an error; a reference by itself does not verify the reply.
Pausing remembering also disables automatic recall. Explicit source review remains
available. Reopened chats and model
changes use a bounded excerpt of the visible conversation, preserving user/reply
roles without replaying earlier tool actions, widgets or reasoning. This same-chat
context stays available when remembering is off. Knowledge → Memories → What I remember lists
your saved details with search, older pages, retained history and original sources.
Newly captured details refresh the open list automatically. While you are reviewing
a source, editing, typing a search or browsing older pages, a notice announces new
details so your work stays in place. Use Search / refresh when ready.
The last capture result says whether details were saved, nothing new was saved,
or remembering failed. Saved details and their sources appear before optional
search settings. The example in How it works is clearly separate from your data.
Correct saves a new version and its source note; Stop using asks for confirmation
and excludes that version from current and historical recall. Both keep previous versions and the
original conversation, which still supplies its own same-chat context. Reviewing
and correcting saved details remains available while automatic remembering is off.
Optional **Find memories by meaning → Choose a search helper** uses an existing
Ollama, OpenRouter or OpenAI-compatible embedding model. Enabling explicitly permits sending
saved details, including retained versions, and future questions to the displayed
service, which may charge. Similarity is a candidate filter rather than proof of
relevance; remote helpers can exceed the short query deadline and fall back to
matching words. [Measured examples](docs/agent-memory.md#live-semantic-calibration-2026-10-27)
cover two models, including these limitations.
Folder search permission is separate. No model is downloaded automatically. Progress
and failures stay visible; matching words still work when this option is off or
unavailable. Switching destinations builds a separate index, and pausing remembering
also pauses embeddings. Models with tools can request saved details for a date or
an exact person, using
separate filters for when a claim applied and when it became known. Ambiguous
dates need clarification. Models without tools explain that date-specific lookup
is unavailable. Relevance calibration and broader quality evaluation remain
unfinished. Opt-in synthetic live-model and disk-growth evaluations now retain
answers, fact history, failures and timings; results include missed facts and
inconsistent correction linking, so passing pipeline tests is not a quality guarantee.
The worked example below the live controls remains an illustration.
Before each model call, Sovra checks the combined input against a conservative
budget based on the model's advertised capacity. Older conversation excerpts and
saved evidence can be omitted to leave room for the answer. The current question
and live tool results remain intact; if they cannot fit, the request reports an
error instead of silently cutting them. Your saved conversation stays unchanged.
Synthetic text/image measurements for the configured model are recorded in the
implementation contract. Readable PCM WAV and ordinary MP4/QuickTime timing now
increases the allowance for longer recordings. Other recording formats retain an
estimate and explain when the length could not be checked. These allowances are
not guaranteed provider token bounds.
Chat saves, deletions and reloads run in order; graceful runtime shutdown waits for
accepted chat saves to finish.
After workers finish, the runtime allows up to five seconds for storage and other
adapter cleanup before tearing down its executor.
Normal desktop quit requests this shutdown and waits after the UI loop exits.
See the [implementation contract](docs/agent-memory.md) for scope and validation.

**Settings** is the one place to set Sovra up, written for people new to AI:
**Appearance** (Light or Dark, and **Text size** from 75% to 200%; also
**View → Zoom In/Out**, ⌘= / ⌘- / ⌘0), **AI connection** (follow the three steps to paste an
OpenRouter key; it is kept in the OS keychain and can be checked, replaced or removed)
and a folded-away **Advanced** section for local or other services, whose technical
fields stay hidden until you choose *Show technical options*. Then select a chat
model in **Models**.

**Chat** starts with a normal conversation using the selected model: quiet user
bubbles, bare replies beside the Sovra avatar and a rounded composer. Each turn
is answered by `OrchestratorAgent`, which offers only the specialists that can
actually do their job this turn and can run independent ones together. Web search
works out of the box: no account, key, server or setup. Ask a question that needs
current facts (news, prices, opening hours, recent events), say "search the web
for…" or "look it up online", or send `/web-search <question>`, and it delegates to
`WebSearchAgent`. That helper searches free, open services built into the app
([Mwmbl](https://mwmbl.org), an open-source non-profit web index, and Wikipedia),
opens the most useful pages to read them, and answers in plain words with source
links taken only from what it found. Research, market, housing and scholar helpers
use the same search. Pages are read as text, never run, and nothing on this
computer or the home network can be opened.
Settings → **Web search** turns it off, along with every helper that needs it.
Optionally, `SOVRA_SEARXNG_URL` adds your own self-hosted SearXNG as one more
source. While Sovra works, a "Sovra is thinking…"
pill shows the elapsed time; models that reason
stream their thinking into a collapsible panel that folds when the answer starts. Replies render
as selectable Markdown with code copying. Each new reply includes a model pill
with its connection and exact model ID on hover; this identity is saved with the
conversation and stays accurate after switching models. Streaming follows the latest output;
scrolling up pauses following until you return to the bottom or use **Jump to latest**.
Visible model reasoning appears in a collapsible panel with an elapsed-time indicator.
Enter sends and Shift+Enter adds a line. **Stop** also discards queued prompts.
Native AG-UI cards and buttons are optional tools for interactive requests, and
their actions return to the same session with the surface ID and revision.
Routing simulation remains in **My AI's**. The **Schedules** tab groups recurring prompts and manual run actions; background
scheduling is not active yet.

## Your team and Artifacts

**Team** is a group of AI helpers with different strengths, ported from SovraSRX:

- Alex (team lead), Morgan (strategist), Priya (researcher), Jamie (editor),
  Sam (wellbeing), Robin (home organiser), Noor (curator) and Kai (devil's advocate).
- Press **Ask my team**, or the team button in the chat box, and the right helpers
  answer in turn.
- Helpers who have nothing new to add stay quiet. Alex sums up and suggests next
  steps, or asks you one question.
- Type `@priya` (or any handle) to ask one helper, or use `/team` to choose team mode and prepare your message before sending.
- A live card shows who is answering.
- On the Team page you can switch helpers on or off, edit them, add ready-made
  helpers (tech helper, coach, storyteller, paperwork helper) or make your own.
- Every helper uses the AI you picked in **Models**.

**Artifacts** keeps anything worth saving:

- Press **Save** under a reply or on a code block.
- You can also ask Sovra or a helper to save something; models that support tools
  call `save_artifact`.
- Artifacts are documents, checklists, tables, code, diagrams, web pages, pictures,
  data and notes.
- Search them, filter by kind, copy them, or **Save to my computer** (tables save as CSV).
- Web pages, SVG pictures and Mermaid diagrams also have **Open in browser**.
  Mermaid diagrams load mermaid.js from jsDelivr, so they need internet to render.
- Edits keep earlier versions.

Everything streams through AG-UI events. Design, sources and the module layout:
[teams and artifacts](docs/teams-and-artifacts.md).

## Connections: Google

Open **Settings → Connections** and press **Connect Google**:

- Google's own sign-in page opens in your browser. Sovra never sees your password.
- Tick what Sovra may use, then come back to the app.
- Each service has its own switch: **Email**, **Calendar**, **Files**, **Contacts**
  and **To-dos**. Switch any of them off without signing out.

Once connected, just ask, for example "What's on my calendar this week?" or
"Add 'buy milk' to my to-dos". Here is what Sovra does with each service:

- **Email:** reads it, but only writes **drafts**. You press Send in Gmail yourself.
- **Calendar:** adds events without inviting anyone.
- **Files:** finds and reads your Docs, Sheets and Slides; it never changes them.
- **Contacts:** finds email addresses and phone numbers.
- **To-dos:** reads your list and adds to it.

**Disconnect Google** signs out and asks Google to forget Sovra's access.

If the card says **Needs a client id**, paste your Google Cloud desktop client
id and optional secret, then choose **Save**. Sovra keeps them in its credential
store; signing out keeps the client setup. Both **Connect Google** and
**Continue with Google** use that saved setup.

A saved client takes precedence over `SOVRA_GOOGLE_CLIENT_ID` and
`SOVRA_GOOGLE_CLIENT_SECRET` in the environment, then values baked into the build. Setup,
scopes, Google's verification requirements and the safety design are in
[Google in Sovra](docs/google.md).

## Files in chat

Use **Add files** (the paperclip beside the message) to attach up to eight files,
20 MB each. Ask a question, or send the files alone for an explanation. Files are
prepared locally; extracted document text, pictures, and recordings are shared with
your selected AI when you send the message. Attached files appear in the conversation, and follow-up questions
can use them again, including after restarting Sovra.

**Models → An AI for each kind of file** keeps separate choices for documents,
pictures, audio, and video. Documents use your chat AI by default. Pictures require
an explicit picture-reading model; chat offers **Help me choose an AI**, keeps your
files and message in place, and shows compatible models. Choosing one returns you
to your message without sending it. Audio and video use the same guided flow.

Files are accepted regardless of extension, including `.rs`, other source code,
extensionless files, and unfamiliar binary formats. Text is decoded directly;
document readers extract supported formats; pictures and recordings use compatible
models. Binary formats without a decoder offer labelled format/byte inspection,
not a claim that every proprietary or encrypted format can be fully understood.
Readers include Office/PDF, email, notebooks, SQLite, binary property lists, SVG,
raster images and bounded ZIP/TAR/GZIP/XZ/LZMA/BZip2/Zstandard archives. Scanned PDFs
can be read as pictures (up to 12 pages). See [file reading and limits](docs/files.md)
for coverage, extraction limitations, model compatibility, and research sources.
The file agent selects relevant passages from long files, cites extracted-text line
ranges, and can page through further text or binary search results when tools are enabled.
For summaries, comparisons and "find every mention" questions it hands the files to the
document reader helper (**Read & compare any document** on the Skills page). The helper
searches every file at once and reads long documents, scanned pages, pictures and
recordings cover to cover in parallel parts. With a picture-reading AI it also looks at
the actual PDF pages, so charts, tables, signatures and stamps are not lost.
It says exactly which parts it has not read yet. See
[the document reader helper](docs/files.md#the-document-reader-helper).

## WhatsApp on your phone

**Settings → WhatsApp → Connect WhatsApp** shows a code. On your phone open WhatsApp →
Settings (or ⋮) → **Linked devices** → **Link a device** and point the camera at it. The
link completes by itself and the badge turns **Connected**. If you can't scan, type your
number under "Can't scan?" and WhatsApp gives you an 8-character code to enter on the phone.

Then message yourself on WhatsApp (the chat with your own number, "Message yourself").
Sovra answers there with the same helpers, memory and web search as the chat window.
By default it only answers your own chat; a switch in Settings lets it answer other
people's direct messages too, and another turns automatic answers off. Groups, channels
and messages that queued while Sovra was closed are never answered. **Unlink WhatsApp**
removes the link on the phone and deletes the saved credentials.

You can also ask in the desktop chat: **“Send me a WhatsApp reminder to bring my
shopping list.”** Sovra's messaging helper checks the connection and prepares the
message in chat. Review the exact recipient and text, then choose **Send** or
**Cancel**. A receipt reports service acceptance; it does not claim the person
has received or read it. The same helper works with Telegram. Find **Send a
WhatsApp or Telegram message** in Skills.

For someone else on WhatsApp, supply their international phone number or choose
an exact recent chat. Telegram can use chats that have already contacted the bot.
Recent-message lookup covers only the messages Sovra has retained locally, not
your whole phone history or address book. These controls replace the test-message
fields in Settings. See [messaging behavior and safeguards](docs/communications.md).

The client is compiled into the single binary (the open-source `whatsapp-rust` crate,
WhatsApp Web multi-device protocol); nothing else runs and no business account is needed.
Credentials and encryption keys are stored only in the app data directory under `whatsapp/`.
WhatsApp does not officially support third-party clients, so use it with your own account.

## Telegram on your phone

Telegram works through a bot of your own, using Telegram's official Bot API. In Telegram,
open **@BotFather**, send `/newbot`, follow its two questions, and copy the token it gives
you. In Sovra, **Settings → Telegram**, paste the token and click **Connect Telegram**.
Sovra shows a six-digit code: open your new bot in Telegram, press **Start**, and send it
that code. The badge turns **Connected** and from then on anything you write to the bot is
answered by Sovra.

By default only you are answered; a switch lets other people who find the bot talk to it
too, and another turns automatic answers off. **Disconnect Telegram** stops the bot and
forgets the token. The token is stored with your other API keys (the keychain), never in
plain text.

Your authenticated own WhatsApp chat and linked Telegram owner chat retain your
personal tools and memory. Other senders get general answers without access to
your saved memories, connected folders, account tools, or private instructions.
Incoming messages from other people cannot create memories about you or prepare
outbound messages in your desktop chat.

## Connect a service you already use

**Settings → Connect a service**. Paste the web address the service gave you.
Add a name if you want one. If the service gave you a secret, paste that too.
It stays on this computer and is not shown again. Choose **Connect**.

If the service needs you to sign in, choose **Sign in** and finish in your browser.
When the status says **Connected**, you can see what the service can do, in
everyday words. Ask in chat as usual. A reply that came from the service shows
that service's name. **Remove** stops Sovra from using it later.

## Shipping

For a local installer, install the build tool with
`dotnet tool install -g vpk --version 1.2.161`, then run `cargo xtask dist`.
The command prints the installer path under `target/dist/` and fails if packaging
is unavailable or no installer is produced. `SOVRA_VPK` can point to an isolated
tool installation. Only the native app is packaged: users do not need Rust,
.NET, Node, Python or a separate database to run it. Online AI requires an account
with the chosen service; optional local servers such as Ollama are configured
separately. Local packages must be signed/notarized before wider distribution.

Push a `vX.Y.Z` tag. `.github/workflows/release.yml` builds each OS, signs + notarizes (when the
secrets listed in that file are set), produces Velopack installers (`.pkg` on macOS,
`Setup.exe` on Windows, AppImage on Linux) and publishes a GitHub Release with update
feeds. The app handles Velopack lifecycle hooks; automatic update checking and an
in-app update action are not implemented yet.

## For contributors and AI agents

Workflow and best practices: [`CONTRIBUTING.md`](CONTRIBUTING.md). Canonical rules:
[`AGENTS.md`](AGENTS.md) (read natively by Codex and Cursor; imported by `CLAUDE.md`
for Claude Code), plus a short `AGENTS.md` per crate. Reasoning: [`docs/architecture.md`](docs/architecture.md).
Edits are checked automatically by `cargo xtask hook` (Claude Code + Cursor hooks) and by CI.
Every commit uses gitmoji and must update [`CHANGELOG.md`](CHANGELOG.md) under **Unreleased**
before pushing to `main` (enforced by [`AGENTS.md`](AGENTS.md) and [`CONTRIBUTING.md`](CONTRIBUTING.md));
security and secrets policy lives in [`SECURITY.md`](SECURITY.md).

The native Sovra dark/light design system, embedded SVG branding, usage examples and
research sources are documented in [`docs/styling.md`](docs/styling.md). The UI uses
neutral charcoal/paper palettes, restrained violet accents, shared page typography
and native keyboard controls. Switch
between both modes in **Settings → Appearance**; the choice is remembered. Layouts adapt
to any window and display: sizes are rem-based and follow the remembered text size, the
sidebar and inspector yield to the main pane at Material width classes, and reading
columns stay capped and centred on large monitors (see *Responsive layout* in the styling guide).
