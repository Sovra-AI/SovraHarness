# Changelog

All notable changes to SovraHarness are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). Releases are cut by pushing a
`vX.Y.Z` tag (see [README → Shipping](README.md#shipping)); installed apps
can use the generated Velopack release feed once update checking is integrated.

Add entries under **Unreleased** as part of the change that introduces them;
move them under a version heading when the tag is cut.

## [Unreleased]

### Added

- **Built-in web search, no setup**: the search helper asks free, open services directly from the app (Mwmbl's open web index and Wikipedia's MediaWiki API) with no key, account or server, and merges their results by reciprocal rank with tracking parameters removed. A new read-only `read_web_page` tool opens chosen results and returns their readable text (scripts, menus and page chrome dropped, 2 MB and 12,000-character limits). It refuses addresses on this computer or a private network before the request, on every redirect, and after connecting. Sources that answer "busy" rest for a minute. The helper is told today's date, answers in plain words and lists only sources it actually found. `SOVRA_SEARXNG_URL` now optionally adds a self-hosted SearXNG beside the built-in sources.
- **Capability-aware orchestrator**: specialists declare the capabilities they need (`sovra_core::Capability`), and `available_specialists` offers only those whose needs are met. Capability providers in `sovra-adk` supply the tools, procedure and routing rule; web search is the first. Research, stock market, housing market and scholar specialists now get real web search.
- **Tool calls written as text now run**: small local models (for example Qwen 2.5 Coder on Ollama) often print a tool call as JSON instead of making it, so nothing ran and the chat showed raw JSON. Every orchestrated agent's model is wrapped once: a reply that ends with one JSON call to a tool actually offered, after at most a short lead-in sentence, becomes a real call. Prose, inline braces, code blocks and calls to tools that were not offered stay text and keep streaming. An opt-in live test (`live_orchestrator`) drives the real orchestrator, a local Ollama model and live search end to end.
- **Search works with small models too**: an explicit request ("search the web for…", or a procedure naming a helper) now always reaches that helper, and a web-backed helper always searches before answering, even if its model would answer from memory (`sovra_core::required_delegate`; capability providers declare their required first call). Each search also opens the top two results and returns their opening text, so answers rest on current pages, not stale index snippets. When the orchestrator's final reply drops the helper's links, the links the helper returned are listed under it (`sovra_core::sources_footer`). With `qwen2.5-coder:7b`, 8 of 8 live runs searched, answered correctly and cited pages the search returned.
- **Questions about current facts search automatically**: "What is the latest…", news, weather, prices, scores and opening hours now always go to the search helper (`sovra_core::needs_current_facts`), so a small model cannot answer them from stale memory; messages with code are left to the orchestrator. A new opt-in runtime test drives the app's real chat path (Ollama connection, model activation, `RunPrompt`, streamed events) and proves a search ran in every turn.

- **Semantic recall measurements:** an opt-in diagnostic retains live rankings and
  latency for 116 synthetic facts and 24 queries with two embedding models. Results
  expose missed paraphrases, unsupported high-similarity matches and remote latency;
  the provisional cutoff and fallback deadline remain unchanged.
- **Verified license metadata:** the exact `finl_unicode 1.5.0` dependency is clarified
  against all three bundled license hashes. Its Unicode file matches SPDX Unicode
  3.0 despite the older manifest identifier; the permitted-license list is unchanged.

- **OpenRouter search helpers:** meaning-based memory and folder search can use
  explicitly advertised OpenRouter embedding models through the existing bounded
  transport. Choosing a helper still requires separate destination consent; chat
  models and unknown capabilities are not treated as embedding models.

- **Duration-aware recording admission:** PCM WAV and ordinary MP4/QuickTime timing
  headers now increase the input allowance for long recordings, including reopened
  attachments. Duration hints are removed before provider requests. Unsupported,
  fragmented or malformed timing keeps the existing fallback and explains that the
  recording length could not be checked. No external media process or dependency
  was added; container hints and reserve rates are not exact provider token counts.

- **Context-budget calibration:** a repeatable, opt-in diagnostic records native
  token usage for synthetic text and images through the selected OpenRouter model.
  Offline tests compare the admission estimate with the retained nine measurements.
  A local long-video fixture documents why compressed size and fixed media allowances
  do not bound duration-dependent usage; audio/video calibration remains incomplete.

- **Temporal memory evaluation:** the opt-in synthetic suite now has a focused
  date mode covering dated state changes, unrelated dates, undated preferences,
  tentative plans and historical answers. Extraction guidance explicitly carries
  a transition date onto its resulting state while keeping unrelated claims
  separate. Both baseline and revised live observations are retained; they do not
  establish that intermittent model date errors are eliminated.

- **Open saved reply sources:** completed ordinary and team replies expose bounded,
  deduplicated source buttons for memory references. They open the original saved
  evidence and current/previous/stopped status, with a way back to the conversation.
  Missing sources show an error without retaining the previously opened source;
  viewing retained history does not reactivate it for recall.

- **Related memory context:** a matched detail can bring up to two supported details
  about the same named person or project into the reply context, helping distinguish
  a colleague's preference from the user's. Extra reads have a 200 ms limit within
  the existing recall deadline and preserve direct matches on failure. Source,
  date, pause and withdrawal checks also apply to this context.

- **Structured remembering output:** models advertising schema support receive a
  constrained extraction format through OpenRouter and compatible transports.
  Unknown capabilities retain the strict local JSON parser; ordinary chat is
  unaffected. Source validation remains local, and schema bytes count toward the
  extraction input limit. Guidance now explicitly includes useful details about
  named people and projects, separates independently changeable attributes and
  preserves source-supported unchanged details during partial corrections. Live
  evaluation also checks separate people's preferences and project schedule changes.
  OpenRouter requests require compatible endpoints; nullable reference schemas
  were checked against the configured live model as well as pinned transports.

- **Bounded memory consolidation:** extraction can reuse a supplied current detail
  expressed in different words while retaining each message's evidence. Related
  keyword matches precede recent details, so relevant older facts can be considered.
  Transactional checks reject stale references, incompatible people/dates and
  conflicting reuse/correction batches. Recognized wordings remain suppressed after
  stopping use and reopening storage. Exact matching now distinguishes people and
  explicit dates; returning to an earlier value creates a new correction version.
  Live-model matching accuracy remains under evaluation.

- **Repeatable memory evaluation:** an opt-in debug test records synthetic live-model
  answers, extraction retries, version history and timings in an isolated database
  with 100 distractors. Normal tests remain offline. Live findings prompted clearer
  background-saving and correction guidance; validation failures now identify the
  failed rule without displaying model response text. Extraction and linking errors
  found during evaluation remain documented rather than counted as quality passes.

- **WhatsApp really links now (`sovra-whatsapp` new, `sovra-core`, `sovra-runtime`, `sovra-ui`, `sovra-app`)**:
  - The earlier Connect WhatsApp screen was a mock: it drew a locally invented code that
    no phone could accept and needed a manual "I've scanned" click. It is replaced by a
    real WhatsApp Web multi-device client, the open-source MIT `whatsapp-rust` 0.7, compiled
    into the single binary (pure Rust, rustls; no Node sidecar, server or business account).
  - **Settings → WhatsApp → Connect WhatsApp** opens a window with the QR code WhatsApp's
    servers issue, painted black on white at a size a phone camera reads. Scan it from
    *WhatsApp → Linked devices → Link a device*; the link completes on its own, the window
    closes, and the Settings badge shows **Connected**. "Can't scan?" asks WhatsApp for a
    real 8-character code for your phone number. Codes rotate automatically; **New code**
    starts over. Errors (expired code, outdated client, link removed on the phone) are
    shown in plain words with a **Try again** button.
  - The link survives restarts: credentials and Signal keys live in one SQLite file under
    the app data directory (`whatsapp/session.sqlite`) and reconnect in the background.
    **Unlink WhatsApp** removes the link on the phone and deletes that file.
  - Messages you send yourself on WhatsApp ("Message yourself") are answered by Sovra's
    helpers through the same prompt path as the chat window, with the same model, memory
    and web search, and the answer is sent back to the phone. Two switches: *Only answer
    in my own chat* (on by default; off answers other people's direct messages too) and
    *Answer WhatsApp messages automatically*. Groups, broadcasts, channels, your own
    outgoing messages mirrored from the phone, and messages that queued while the app was
    closed are never answered. **Send to my WhatsApp** sends a test message to your own chat.
  - Pure policy and the `MessagingChannel`/`ChannelSession` ports live in
    `sovra-core::connections` (unit-tested); `sovra-runtime` drives the link in one task and
    diverts `whatsapp:` sessions' events from the chat window back to the phone.
  - `rusqlite` moves to 0.39 so the whole binary links one SQLite (the WhatsApp store pins
    libsqlite3-sys 0.37); `xtask/src/deps.rs` gains `sovra-whatsapp` and confines
    `whatsapp-rust` and `qrcode` to it.

- **Restored conversation context:** new ordinary and attachment turns carry a
  bounded text excerpt from the visible saved chat, preserving user/reply roles
  after restart and model changes. Fresh per-turn sessions prevent duplicated
  provider history and replay of earlier tools, widgets or reasoning. Team prompts
  also bound earlier text and always include the complete current request, even
  when no history is supplied. Same-chat context remains available with cross-chat
  remembering off; omitted/shortened history is identified to the model.

- **Ordered chat persistence:** chat snapshots, soft deletions and reloads now run
  in command order on one owned background worker. A slow older save cannot
  overwrite a newer snapshot or resurrect a subsequently deleted thread. Graceful
  runtime shutdown drains accepted chat operations before reporting completion.

- **Saved memory controls:** Knowledge now lists current memories with search,
  bounded pages, retained history and original sources. Corrections atomically
  save a user-authored note and replacement while retaining the previous version.
  Stop using asks for confirmation and excludes that version from current and historical recall
  without deleting its history or original chat. Explicit review remains available
  while automatic remembering is paused; Knowledge also has the remembering toggle.

- **Shared context budget:** model requests now account for instructions, tool
  schemas/results, restored conversation, recalled evidence and media reservations
  together, with room reserved for the reply. Older optional context yields first;
  the current question and live tool exchanges are never silently cut. Requests
  that still cannot fit fail before contacting the chat model, while saved history
  remains intact. Admission uses conservative estimates; provider-tokenizer and
  media calibration remain unfinished.

- **Historical conversation queries:** ordinary and team helpers can request
  date/entity-scoped evidence, distinguishing when a claim applied from what was
  known by a date. Four query receipts per turn contain no remembered facts;
  evidence is refreshed at each model request and remains revocable. ISO dates
  and offset-bearing date-times are validated, and ambiguous dates require
  clarification. Explicit Stop using now records a withdrawal separate from
  temporal correction history, excluding that version even from past queries.
  Schema v5 conservatively migrates earlier stops without erasing records.

- **Optional semantic memory recall:** Knowledge can use an explicitly chosen
  embedding destination for saved details and future questions, independently of
  folder permissions. Incremental batches reuse existing provider transports;
  dimensions are pinned per selection and old indexes never mix with a new model.
  Pausing, disabling or changing destinations cancels outstanding work. Keyword
  recall remains available on semantic failure or timeout. Migration/reopen,
  consent, cancellation and responsive controls have deterministic tests;
  relevance calibration and broader quality evaluation remain unfinished.

- **Conversation recall (in progress):** ordinary, attachment and team model
  requests receive bounded keyword-matched evidence with original user quotes,
  conversation IDs, dates and citation identifiers. Retrieved evidence stays out
  of saved provider sessions; pausing revokes in-flight reads. Unsupported,
  assistant-authored, future and superseded evidence is excluded from current
  recall. Storage supports explicit valid-time and known-time filters.
  Models with tools can now request explicit dates and entities through bounded
  per-turn queries; models without tools disclose that limitation.

- **Automatic remembering (in progress):** an owned background worker extracts
  supported details from user messages with the selected AI, without tools or
  assistant history. Complete structured responses must cite exact source text;
  bounded jobs retain provenance and report failures. Turning remembering off
  cancels pending work and model requests, and storage rejects late writes. Model
  changes cancel old extraction for retry. Knowledge shows live status and explains
  which data reaches the selected AI service.

- **Recoverable memory ingestion (storage API, in progress):** user evidence and
  its pending job are recorded together under a stable conversation/message key.
  Expiring leases recover interrupted work with bounded retries; a whole validated
  extraction commits atomically with its job. Exact duplicates retain one fact and
  every job's evidence, and do not reactivate a stopped fact. Pausing cancels pending
  and leased work across restarts.

- **Conversation-memory storage foundation (in progress):** fact, source, entity and
  replacement writes commit atomically; stale corrections fail without partial
  records. Recall exposes source and replacement references, episode reads are
  bounded, and embedding identity persists across disk reopens. Schema migration
  rejects incompatible vectors and newer schemas.

- **Summary model transport (in progress)**: a single-request adapter supports Ollama, Anthropic, OpenRouter and compatible chat endpoints without tools. It revalidates model destinations when constructed, validates prepared source scope and size, uses conservative context admission, and accepts only completed JSON with valid input citations. Embeddings and summaries share bounded HTTP response handling. Scheduler, consent UI and recursive generation remain pending.

- **RAPTOR clustering engine (in progress)**: global/local UMAP reductions feed full-covariance Gaussian mixtures selected by BIC. Soft assignments retain overlap and source coverage; recursive grouping fits request budgets and identifies fallback partitions explicitly. Computation runs on one CPU worker with cancellation and finite work limits. Background generation and retrieval integration remain pending.

- **Summary storage foundation (in progress)**: immutable summaries retain exact source inputs, model provenance and claim citations. Source changes and derived corrections invalidate dependent summaries; revoked folder grants remain revoked after re-adding a folder. Request reservations persist across restarts, enforce a chosen budget, and reject results from paused or changed work. Background generation, RAPTOR clustering and compiled maps are not yet exposed.

- **Reviewed file knowledge**: `/map-files` proposes typed relationships between logical documents, sections, topics, people, organizations, projects, tasks and exact source versions. Knowledge shows the structured preview, full cited excerpts, extraction caveats, confidence estimates, dates and replacements before Save or Reject. Only native review can approve; saved claims remain model inferences. `/file-knowledge` supports bounded traversal and evidence reads with independent recorded/world dates and supersession history. Changed sources block approval and require explicit historical retrieval; revoked grants cannot be restored by re-adding a folder. Original files remain unchanged.

- **Opt-in semantic file search**: each indexed folder can use an explicitly chosen Ollama or OpenAI-compatible embedding model. Knowledge names the destination before enabling background passage indexing. Search combines keyword and cosine rankings with source-version citations; incomplete coverage and embedding failures are disclosed to both the UI and agent. Changing the configured server/model requires a new choice, and disabling/removing the folder cancels outstanding work and removes its vectors. No model downloads or embedding requests occur automatically for unconfigured folders.

- **File history and provenance**: `/file-history` (also `/file-provenance` and `/file-versions`) inspects an append-only observation log, including observed renames, metadata/content changes, scan status and extraction revisions. Historical passages retain their original observed paths and version citations, support bounded continuation, and become inaccessible when a folder grant is removed. Missing eligible paths are distinguished from proven deletions; legacy index state is labelled as imported, with unknown provenance and real-world validity left explicit.

- **File catalogue skill**: `/file-catalog` (also `/query-files` and `/list-files`) discovers indexed scopes, filters observed files by literal name/path, MIME type, size, filesystem modification time, extraction status and text keywords, and groups counts by folder, directory, extension or MIME type. Results include scan coverage, unknown-size counts and revision-bound pagination. Large, unsupported and blank files, and files whose text extraction fails, retain metadata without claiming readable content; existing indexes migrate transactionally and preserve citations. Queries over 32 keywords return a clear error instead of silently dropping terms.

- **Local folder indexing (in progress)**: `/index-folder` opens the native folder chooser; Knowledge shows measured progress, cancellation, refresh, removal and local text search. A persistent SQLite FTS index reuses content-defined chunks, keeps source-version citations, skips symlinks/hidden/generated inputs, and preserves unvisited records when a scan is cancelled. Sources remain read-only. `/search-files` lets the agent retrieve bounded source passages and cite extracted-text lines through the orchestrator. Bounded native watches coalesce changes with periodic reconciliation, visible fallback, and persisted pause/resume controls. Derived knowledge remains in progress.

- **AI team and Artifacts (`sovra-core`, `sovra-adk`, `sovra-runtime`, `sovra-store-surreal`, `sovra-ui`)**:
  - **Team, ported from SovraSRX team chats.**
    - Eight seeded helpers, with a moderating Conductor (Alex).
    - The first speakers are chosen by `@mention`, then name, then "everyone" words,
      then expertise.
    - Members may pass (`<pass/>`), which is never shown.
    - The Conductor's verdict tail (`CONTINUE` / `YIELD_TO_USER` / `ASK_USER`) is hidden.
    - A turn has at most 3 rounds and 8 replies.
  - **Streamed as AG-UI events.** Team turns use `RUN_*`, an `ACTIVITY_SNAPSHOT`
    live team card, `SUBAGENT_STARTED/FINISHED` and `TEXT_MESSAGE_*`, through the
    new `Event::AgUi`.
  - **Team page.** Switch helpers on or off, edit them, add ready-made helpers or make
    your own, and restore originals.
    - "Ask my team" and the composer's team button switch chat into team mode.
    - `/team` and `@handle` also send a message to the team.
  - **Artifacts.**
    - Nine kinds, one table row each, so adding a kind is one variant plus one row.
    - **Save** on every reply and code block. The main block becomes the artifact;
      otherwise the whole reply is saved as a document.
    - A `save_artifact` tool for agents, with an in-chat AG-UI card.
    - A library page with search, kind filters, preview, copy, **Save to my computer**
      (tables as CSV) and **Open in browser** for HTML, SVG and Mermaid.
    - Edits keep earlier versions; delete is a soft delete and asks for a second click.
  - **Persistence.** The roster and artifacts are saved in the embedded SurrealDB store.
  - See `docs/teams-and-artifacts.md`.

- **Skill workflows (in progress)**: one validated command catalogue backs native slash
  suggestions, explicit model procedures and the Skills page. Local user skills use
  bounded SKILL.md packages with private, atomic writes, retained resources, enable/disable,
  import and removal. File indexing and the broader knowledge pipeline remain tracked in
  `docs/skills-and-file-knowledge.md` and are not yet complete.

- **File-agent retrieval and citations**: small prompt budgets now prioritize matching
  passages, including short identifiers and numbers, with extracted-text line ranges.
  Text search and binary search have explicit continuation cursors; binary search also
  accepts hexadecimal byte patterns. Prompt excerpts and tools share one cached index
  built on a runtime-owned blocking task, and malformed tool offsets are rejected.
  Rebuilding a reader replaces its old attachment tool and honors disabled tools.

- **Extension-independent file reading**: fixes `.rs` and other readable source/config
  files being rejected by the document parser. Preserves UTF-8 text and supports
  UTF-16/32 plus detected legacy encodings, signature-aware renamed containers,
  bounded XZ/LZMA/BZip2/Zstandard reading, SVG rendering, SQLite tables, binary
  property lists and executable metadata. Unknown, empty, encrypted or damaged
  formats remain available as explicitly labelled inspection with retained bytes
  and bounded byte tools; inspection never implies complete semantic decoding.
  Text sections borrow source slices, repeated tool searches cache case folding,
  and binary search uses `memchr`. See `docs/files.md` for concrete limits.

- **Responsive, scalable layout (`sovra-core`, `sovra-runtime`, `sovra-ui`, `xtask`)**:
  every design-token size is now in rems, so the whole UI follows a remembered
  **Text size** (75–200%) in **Settings → Appearance**, **View → Zoom In/Out/Actual Size**
  and ⌘= / ⌘- / ⌘0 (`ModelCommand::SetInterfaceScale`). Panes adapt by Material 3 width
  classes measured in rems: the sidebar becomes a rail below 840 reference px, docked
  inspector/detail panes need 1200, and the models view measures its own pane rather
  than the window. Chat prose is capped at a centred ~80-character column on wide
  monitors, Markdown headings scale with the text, and the first window is fitted to
  the display's visible area. `cargo xtask check-styles` now rejects literal `px`/`rems`
  sizes in views and window-width checks outside the shell; rules live in `AGENTS.md`
  and `docs/styling.md` → *Responsive layout*.

- **Files in chat and task-specific models**: native multi-file picker, preparation/error
  cards, image thumbnails, remove controls, saved transcript attachments, and draft-preserving
  model setup for pictures, audio and video. Document models default to the existing chat
  selection; media choices persist separately and filter by known input/transport capabilities.
- **Local file reader (`sovra-files`)**: Xberg 1.3.10 for documents, pure Rust PDF reading,
  normalized raster images and scanned-PDF page inputs, bounded ZIP/TAR/GZIP traversal,
  private managed storage, and attached-file reading with actual multimodal parts plus a read-only
  section/search tool for long documents. See `docs/files.md` for supported families and limits.

- **Keyless web search (`sovra-core`, `sovra-adk`, `sovra-runtime`, `sovra-app`)**:
  Chat turns are answered by `OrchestratorAgent`, built from the Sovra SRX roster so independent specialists can run at the same time. Asking to "search the web for …" delegates to `WebSearchAgent`, even when the topic contains words such as "rust" or "code", and even when that chat already has files attached. `/web-search <question>` requests that same delegate: after the slash command resolves to the question alone, the active procedure tells the orchestrator to call `WebSearchAgent`. Only that delegate receives a read-only `SearXNG` search (`GET /search?format=json`) with no API key. Several queries in one call run together, and a failed query does not drop the others. The answer cites titles and URLs from that response only. Settings **Web search** removes `WebSearchAgent` and the tool. `SOVRA_SEARXNG_URL` overrides the default origin (`https://searx.be`).
- **Unified Single Left Sidebar & AI-Generated Conversation Titles (`sovra-core`, `sovra-adk`, `sovra-runtime`, `sovra-ui`)**:
  - Consolidated the navigation sidebar and conversation history rail into a single unified left sidebar (`sidebar.rs` + `chat_history_view.rs`), eliminating the double-sidebar layout.
  - Placed a prominent **`+ New Chat`** button at the top of the sidebar above primary navigation, followed by search, `Chats | Scheduled` filter pills, and date-grouped conversation history (`Pinned`, `Today`, `Yesterday`, `Previous 7 Days`, `Previous 30 Days`, `Older`) that remains accessible across all workspace tabs and collapses cleanly into a compact icon rail on narrow viewports.
  - Added automatic LLM-powered conversation title generation (`Command::GenerateChatTitle` / `Event::ChatTitleGenerated` and `AdkHarness::generate_title`): after the first exchange in a chat, the active model generates a concise 3–6 word title (sanitized via `sanitize_generated_title` with smart filler-stripping fallback in `derive_chat_title`), plus a 1-click **`AI Title`** button on each history row while preserving manual user renames.
- **State-Aware Guided AI Setup, Curated Model Recommendations & 5-Step Interactive System Tutorial (`sovra-core`, `sovra-runtime`, `sovra-ui`)**:
  - Deterministic `SetupStage` progression (`NeedsConnection` → `NeedsModelSelection` → `NeedsModelKey` → `Ready`) so Chat setup cards, the top Guided Setup Strip, and `Models` setup home **only** display what is still needed (when an API key is already saved, Step 1 connection options are hidden and the user is guided straight to Step 2: picking a model).
  - Model suggestions for free trials, everyday chat and step-by-step reasoning use live catalog prices and advertised capabilities, without claiming a quality ranking. Direct **Use Model** / **✓ Active** actions are available in the catalog.
  - One-click **Open openrouter.ai/keys in Browser ↗** helpers on both the Setup Home screen and the Settings OpenRouter key editor.
  - Interactive 5-step System Tutorial (`TourStep::WelcomeChat` → `AutomaticRouting` → `KnowledgeMemory` → `AutomatedSchedules` → `PersonalizeSettings`) with a top spotlight coach bar (`onboarding_bar.rs`), sidebar step badges, interactive try-it actions, and persistent `onboarding_completed` state.
- The Knowledge tab explains how Sovra remembers, in everyday words: a record of
  what happened, a separate current belief, who or what that belief is about, and
  the older belief kept as history when a newer one replaces it. A worked example
  shows that whole life in order, including that finding a memory uses both meaning
  and the words themselves. The page is a fixed picture, so it still appears when
  no memory store is open. The times Sovra keeps stay behind a labeled control.

### Changed

- **Web search fixed**: it used to rely on a public SearXNG server (`searx.be`) that answers in HTML, not JSON, so every search returned no results. It now uses the built-in sources above. The orchestrator also asks the search helper whenever an answer depends on current facts, not only when the user typed an exact phrase. More plain phrases ("search online", "look it up online", "google it") count as an explicit request.
- **Orchestrator offers only working specialists**: it no longer offers itself, or specialists whose integrations do not exist yet (email, calendars, GitHub, to-dos, running code), so it cannot hand a question to a helper that would only guess. A missing ability is now reported plainly.

- **Plain-language Skills & Shortcuts explorer (`sovra-ui`)**:
  - Replaced the flat vertical dump on the **Skills** page (`skill_library.rs`) with a 3-step *"How it works"* banner, category filter pills (`All skills`, `Quick actions`, `Files & folders`, `Web search`, `AI helpers`, `Tables & cards`, `Made by you`) with live counts, and categorized responsive glass cards (`Surface::Glass`) across `t.layout.page_width`.
  - Every skill card now leads with its human title, `/shortcut` pill, plain-English kind badge (`Instant action`, `Guided recipe`, `Made by you`), stripped of internal agent boilerplate, plus a concrete real-world example prompt and a primary **`Try in chat`** / **`Run now`** CTA.
  - Simplified the **Create a skill** form with numbered plain-English prompts, automatic `/shortcut` slug generation from the title when left blank, and progressive disclosure (`Show optional tool settings`) for technical tool names.
- **Grouped, plain-language sidebar & streamlined conversation history (`sovra-ui`)**:
  - Primary navigation is now chunked into everyday workspace destinations (`Chat`, `Team`, `Artifacts`, `Schedules`) and a labeled **Your AI & Setup** section (`Models`, `My AI's`, `Knowledge`, `Skills`, `Settings`) to reduce cognitive load.
  - Promoted **`+ New Chat`** to a primary brand-gradient CTA, added plain-English tooltips to every sidebar tab and collapsed rail button, surfaced live context badges (`Setup` on `Models` when unconfigured, saved item counts on `Artifacts` and `Schedules`), and anchored **`Take a guided tour`** in the sidebar footer above the account switcher.
  - Eliminated double-padded nested borders in conversation history rows, added pin badges and compact icon quick-actions (`Pin`, `AI Title`, `Rename`, `Delete`) with tooltips, and upgraded empty states with direct action buttons.
- **Setup is two steps and ends in the chat.** Models shows *1 · Connect* → *2 · Pick
  your AI*. **Use this model** goes straight to the conversation once the runtime
  confirms the choice, and saving a key from a setup step continues to the next step
  (to Chat when a model is already chosen). A message typed before setup finished is
  sent automatically as soon as a model is ready. Chat setup cards and the top setup
  bar carry one action each, in plain words. Per-file-type AI choices appear once
  setup is complete.
- **Picking a chat model is plain.** After the key is saved, step 2 reads *Pick a chat
  model* and every suggestion shows its price as an estimated cost per message ("Free",
  "About 1¢ a message") instead of per-million-token rates; the model list does the
  same, with exact rates kept in model details. The everyday suggestion is now the
  newest affordable paid model that supports Sovra's tools, rather than the cheapest
  model listed, and Settings points straight to *Next: pick a chat model* once a key is
  saved.
- **The model library shows chat models by default.** Every model is labelled by what
  it makes (chat, pictures, video, voices, speech to text, search helpers, other tools).
  A **Show** picker beside the search box lists the other types; it replaces the
  "What it creates" filter.
- Refine the native UI throughout: neutral charcoal/paper palettes, restrained
  violet accents, consistent typography and spacing, flatter cards, quieter chat
  bubbles and a more comfortable composer. Unify page headers and simplify setup
  and settings copy. Improve navigation targets and selected-conversation actions.
- Use native keyboard-operable buttons for routing choices and conversation/profile
  actions. Keep Knowledge readable by hiding its secondary inspector at compact
  widths. Document the research and native styling decisions in `docs/styling.md`.

- Start Models with a guided connection → model choice → Chat flow. Offer an
  explicitly free, text-capable online model when available; wait for runtime
  confirmation before showing the Chat action. Keep the full virtualized catalog
  one click away with filters closed initially, and stop showing an unavailable
  local server error while browsing unrelated online models.
- `cargo xtask dev` stores model API keys in 0600 files under the app data dir
  (`SOVRA_DEV`) instead of the keychain, so hot reloads never prompt for the macOS
  password. Release builds still use the keychain.
- Restyle the standard chat after SovraSRX: bare assistant replies beside a Sovra
  avatar (ringed while writing), gradient user bubbles with a tail corner and initials
  avatar, responsive 720/768/880px reading column, 16px/1.7 reply text, a glass
  composer with violet focus glow, round send and red stop buttons, a glass "Jump to
  latest" pill and quiet copy actions.
- Show model reasoning: a shimmering "Sovra is thinking…" pill with an elapsed timer
  before output, then a collapsible reasoning panel that streams open, folds when the
  answer starts and reads "Thought for Ns" afterwards (saved with the conversation).
- Make streaming cheaper: history is synced once per turn instead of copying the whole
  transcript on every token; Markdown views receive only appended bytes (parsed off
  the UI thread); thinking chunks are coalesced like answer chunks; collapsed reasoning
  is never built; the per-token fade animation is off and the only continuous
  animation is the pre-answer shimmer.
- Streamed tokens are coalesced per frame (8ms) before they reach the UI, so each
  frame costs at most one state update and one Markdown parse. Measured on a
  20,000-chunk reply after 40 long messages: the core reducer went from 327ms to 0.7ms.
- Render the conversation as its own cached GPUI view: composer cursor blinks,
  typing, the sidebar and the elapsed timer reuse its last frame instead of laying out
  every visible message. Measured with a long conversation on screen: idle CPU after
  a reply fell from ~20% to ~7% of a core (the cost of the cursor blink alone).
  GPUI frame-time overlay, release builds, 40 earlier replies plus a 3,400-token
  streamed reply: frames after the reply drop from 72.8 ms to 3.6 ms, and the slowest
  10% / 1% of frames late in the stream from 71.1 / 74.7 ms to 40.9 / 48.7 ms.
- Long replies render as several virtualized rows, split only at safe Markdown block
  boundaries (never inside code fences), so a streaming update lays out what is on
  screen instead of the whole reply. GPUI frame-time overlay, same workload: the
  slowest 10% of frames late in a 3,400-token reply fell from 70.5 ms to 10.6 ms
  (inside the 16 ms budget for 60 fps), the slowest 1% from 75.0 ms to 24.2 ms.
- Very long paragraphs (walls of prose) are split softly between sentences after
  ~3 KB, never inside inline code or emphasis, and drawn without a gap. Final load
  matrix (frame-time overlay plus app CPU over the same window, old → new): slowest
  10% of frames short 12.1 → 5.4 ms, code 7.2 → 4.5 ms, reasoning + answer ~104 →
  14.4 ms, long 66.7 → 10.2 ms (all within the 16 ms budget); frames drawn in heavy
  streams up to 1.9× more for 7–23% less total CPU, i.e. CPU per frame 72 → 33 ms and
  55 → 34 ms on the virtual display (whose software copy is the ~30 ms floor).
- Reasoning is segmented like answers; while it streams, the pinned panel builds only
  its last segments. Replies containing link reference definitions stay one block.
  Load matrix (frame-time overlay, slowest 10% at end of stream, old → new): short
  12.2 → 5.7 ms, code-heavy 7.5 → 4.3 ms, long 65.6 → 10.2 ms, reasoning + answer
  ~102 → 33.7 ms; CPU after any reply ≤ 7% of a core (was up to 27%).
- Highlight code blocks with `gpui-kit`'s `syntect` feature (pure-Rust regexes,
  cached per unchanged block).
- New chats open on a `SovraSRX`-style welcome: a greeting, a plain-language subtitle and
  starter chips that fill the message box without sending.

- Clean up the title bar: it no longer shows the product name or a light/dark
  button. Appearance now lives in **Settings → Appearance**.
- Rework Settings for people new to AI: three plain-language sections (Appearance,
  AI connection, Advanced), step-by-step OpenRouter key instructions, a folded-away
  Advanced section for other services, and technical connection fields (service
  type, key header, wait time, organization/project IDs) hidden behind
  *Show technical options*. Connection, key and error messages use everyday wording.
- Simplify **My AI's** for people new to AI: three plain choices (Automatic,
  One AI for everything, Choose per task) with one-line explanations; effort per task
  (Quick / Balanced / Thorough) shows only when choosing per task, and the decision
  engine, routing tree and prompt tester sit behind *How automatic choice works*.
- Rewrite the **Models** library in everyday words: Online / On my computer / Other
  services tabs, "Made by", "Reads / Creates", "Not for chat" markers, "Memory size", "Can use
  tools", "Thinks step by step", plain prices and clearer Ollama download help.

### Fixed

- **Release storage during shutdown:** keep the background executor alive for up
  to five seconds of adapter cleanup after owned workers finish, then emit
  `Stopped` after executor teardown. This lets the embedded database finish its
  asynchronous close instead of leaving its directory locked on an immediate
  runtime restart. A combined real-disk/runtime/HTTP regression covers remembering,
  recall, corrections, withdrawal, paused restart and restored chat context.
  The desktop quit hook now requests shutdown, and the composition root waits for
  a dedicated completion signal after the UI loop exits, so accepted saves are
  drained before normal process exit without competing for UI events.

- **A key OpenRouter rejects is no longer saved as "connected".** OpenRouter lists its
  models without a key, so a mistyped or deleted key used to look ready and then fail
  the first message with `model.unauthorized: User not found.` Saving a new OpenRouter
  key now checks it first and explains how to copy it again; a key refused later shows
  "OpenRouter didn't accept your key…" with the fix instead of the raw error code.
- **Chat no longer asks you to set up again after setup said "ready".** If the
  settings file records a saved key the credential store does not hold (a reset
  keychain, or a `SOVRA_DEV` build reading its separate dev store), the app now asks
  for the key instead of claiming setup is complete. The runtime also reports when
  the chosen model cannot run (`ModelEvent::ActiveModelStatus`), so Models and Chat
  agree and ask for another model. A locked keychain no longer stops other settings
  loading, and the chosen model is re-activated before a saved key is announced.
- Embedding model choices now use catalogue metadata before incomplete manual
  entries, matching execution and excluding models known to produce only chat text.

- Turning folder auto-refresh on or pausing it applies on the next indexer turn.
  Those edits no longer wait for a one-second tick that can be skipped while
  other work is running.
- Keep login controls within the minimum window and make schedule actions
  keyboard operable.
- Keep untouched chat drafts out of conversation history, avoiding misleading
  epoch-based ages before the first message.

- Send missing-key setup to the selected model's own service, including custom
  connections; avoid repeating setup prompts inside Models and Settings.
- Keep development credentials separate from release keys, enforce owner-only
  Unix permissions when saving, and ignore the development flag in release builds.
- Describe the tour's routing simulation, memory example and manual task runs
  accurately instead of promising background automation.
- Make `cargo xtask dist` fail early without the pinned packaging tool and verify
  that exactly one nonempty native installer was produced. Each build stages only
  the app in fresh directories, with an explicit OS/architecture and bundle identity.
- Use the `.entitlements` filename required by Velopack in both local and release
  packaging, and pin the release CLI to the SDK version. Clarify that update
  lifecycle hooks exist but automatic update checking is not yet implemented.
- Correct documentation lint and Settings view visibility errors that prevented
  the live development build and CI from compiling.
- Streaming a code-heavy reply could crash the app: a segment split was decided only
  after a line completed, behind text already rendered. Splits are now decided at a
  line's first byte, with a defensive one-block fallback, covered by an invariant test
  and a mid-line streaming test.
- Sending in a conversation restored at startup no longer blanks earlier replies or
  shows one reply's text in place of another: new message ids skip ids already in
  the conversation, and chat rendering state is keyed by position, so duplicate ids
  saved by older builds still render correctly.
- Code blocks use the dark highlight theme in dark mode (they used the light one),
  and reasoning text is muted.
- Memory recall's "newest first" order no longer flips when two facts are saved in
  the same millisecond: each store hands out strictly increasing fact timestamps.
- Ensure every clickable button (`design::button`, `design::primary_button`), checkbox (`Checkbox`), dropdown (`Select`), and custom interactive element across `sovra-ui` displays a pointer cursor (`CursorStyle::PointingHand`), enforced by `xtask check-styles`.
- Normal chat answers conversationally in Markdown; native UI tools are optional,
  and ordinary turns no longer invoke the specialist routing simulation.
- Preserve streamed text exactly and deduplicate ADK's final response snapshots.
- Render selectable Markdown with code copying in a centered reading column;
  virtualized messages follow streamed output, pause when scrolled up, and offer
  Jump to latest. Enter sends, Shift+Enter inserts a line, and Stop clears queued work.

- Keep OpenRouter key management visible after saving, with a persistent masked
  saved status and explicit replace, test and remove controls.

- Enable keyboard navigation immediately when a window opens, before an input has been clicked.
- Size the title bar to fit the appearance control without clipping and verify keyboard theme switching.
- Align model names with their metadata and preserve usable Models controls at the minimum window size.
- Settings now opens provider configuration, with a masked OpenRouter key editor,
  connection testing and OS keychain save/replace/removal.
- Left sidebar icons and labels share a left-aligned content edge.
- Chat starts empty; removed seeded messages, hero banner and mock build inspector.
- ADK-UI tool responses now deliver actual AG-UI component payloads to native GPUI
  cards, tables, alerts, progress and action buttons. Surface revisions update in
  place; button actions carry their surface identity back to the agent.
### Security

- Record the approved `RUSTSEC-2026-0387` exception for SurrealDB's WebAssembly-only
  cryptography dependency, which is absent from all three desktop release targets.

### Added

- **Settings → Appearance** remembers Light or Dark across restarts (saved with model
  preferences; older preference files load as Dark).
- Show streamed reasoning in a collapsible panel with an elapsed-time indicator,
  and refresh chat typography, avatars, user bubbles and composer styling.
- Feed Markdown incrementally and save transcript snapshots at turn boundaries
  to reduce work during long streamed replies.
- Assistant replies show a compact model and connection pill with the exact model ID
  on hover. Execution identity stays with each reply across model changes and saved history.

- **Chat History Rail & Schedules Integration (`sovra-core`, `sovra-store-surreal`, `sovra-runtime`, `sovra-ui`)**:
  - Multi-session conversation history rail (`RecentConversationsRail` from `SovraSRXProduction`) with collapsible strip, instant search, segmented `Chats | Scheduled` radio-group filter pills, `Pinned` partitioning, recency buckets (`Today`, `Yesterday`, `Previous 7 Days`, `Previous 30 Days`, `Older`), semantic topic icons, auto-titling from first prompt, inline rename, pin/unpin, and soft-deletion.
  - Persistent `chat_thread` storage in `sovra-store-surreal` and automatic startup history hydration via `sovra-runtime`.
  - Schedules placeholder empty state (`"No scheduled runs yet"` in the history rail and `"No schedules yet"` with schedule creation form in the Schedules Command Center).
- **Main Chat, Login & Personal AI Builder (`sovra-ui`, `sovra-core`)**:
  - 3-column post-login workspace (`Sidebar`, `ChatView` with `"Built around you."` hero banner and prompt timeline counter, and `My Personal AI` right inspector with 6 configuration cards and `"Preview my AI"` workflow) plus a native glass Login screen.
  - Native AG-UI (`adk-ui = "2.2.0"`) surface rendering (`BuildProgress` live checklist card, `ClarifyChips`, and `SurfaceCard`) and `SovraSRX` type-ahead prompt queue strip.
  - `SovraSRX` **Schedules Command Center** (`NavTab::Schedules`) with KPI cards (`Active Schedules`, `Runs Today`, `30d Success Rate`), Pause/Resume controls, and one-click `"Run in Chat"` execution.
- **Multi-Provider Model Library & Discovery (`sovra-providers`, `sovra-adk`, `sovra-ui`)**:
  - Live catalog discovery, caching, and OS keychain credential storage (`keyring`) for **OpenRouter** (all models & modalities), **Ollama** (local tags, metadata enrichment, and streaming model pulls), **OpenAI-compatible** servers (`LM Studio`, `vLLM`, `llama.cpp`, custom `/v1` endpoints), **Anthropic**, and **Google Gemini**.
  - Virtualized **Models** view with search, capability/modality/context/price filters, side-by-side model comparison, and connection management.
- **22 `SovraSRX` Specialist Agents & `CogniRouteSLM` Dynamic Tree Router (`sovra-core`, `sovra-adk`, `sovra-ui`)**:
  - Full 22-specialist `SovraSRX` roster (`OrchestratorAgent`, `WebSearchAgent`, `CodingAgent`, `DeepResearchAgent`, `StockMarketAgent`, `HousingMarketAgent`, `ScholarAgent`, `GitHubAgent`, etc.) with per-agent model and tier (`Tier 0 · Fast`, `Tier 1 · Standard`, `Tier 2 · Deep`) overrides.
  - Three selectable model routing modes in **My AI's & Routing Tree**: `Single Global Model`, `Per-Agent Models (22 Specialists)`, and `Dynamic Tree Router`.
  - System-1 decision model client (`sovra_adk::decision_client`) supporting **Laya** (`/v1/systemone`), **OpenJev / TypeSafe Jev** (`/v1/systemone` and OpenRouter `/api/alpha/decisions`), and an **Embedded Rust Heuristic** engine (`0ms` offline fallback) with `heuristic_floor`, auditable node-by-node traces, and `rc1-<tree>-<path>` route codes.

## [0.1.0] — 2026-10-24

Initial import of the native Rust desktop harness.

### Added

- Workspace of layered crates with machine-enforced dependencies
  (`app → ui → runtime → core`, table in `xtask/src/deps.rs`):
  - `sovra-core` — pure domain: types, `Command`/`Event` protocol, state
    reducers, ports (`MemoryStore`). No IO, async runtime, or GUI.
  - `sovra-adk` — ADK agent harness adapter.
  - `sovra-providers` — LLM provider adapters.
  - `sovra-runtime` — tokio on a background thread, serving the UI over
    channels (`Client`).
  - `sovra-store-surreal` — embedded SurrealDB adapter for `MemoryStore`
    with bi-temporal, supersede-don't-edit facts.
  - `sovra-ui` — GPUI views via `gpui-kit`, with the native Sovra design
    system under `src/design/` (dark + light themes).
  - `sovra-app` — binary and composition root: Velopack hook, paths,
    logging, wiring.
- `xtask` automation: `dev` (watch + rebuild + relaunch), `run`, `ci`,
  `fix`, `check-deps` (layering), `check-styles` (styling contract and
  contrast), `setup`, `dist`, and the editor `hook` (rustfmt + clippy on
  every agent edit).
- CI (`.github/workflows/ci.yml`) running every gate, and release
  packaging (`.github/workflows/release.yml`) producing signed Velopack
  installers for macOS, Windows, and Linux from a `vX.Y.Z` tag.
- Dependency policy: `deny.toml` (license allowlist, no OpenSSL, no git
  deps), `clippy.toml`, workspace-level lints, pinned Rust 1.99.1.
- Documentation: `docs/architecture.md`, `docs/development.md`,
  `docs/styling.md`, agent rules in `AGENTS.md` (plus per-crate files).
