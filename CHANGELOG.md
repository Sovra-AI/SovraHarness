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

- **Knowledge → About you:** Knowledge opens with the person's saved details,
  capture totals and recent activity. **Add a detail** saves an explicit note
  immediately, with a retained source and no AI interpretation. Failed message
  captures can be retried without reprocessing completed or paused-away work.
  Capture activity refreshes without discarding drafts, corrections or older pages;
  **Files** and **Guide** remain one click away.

- Persisted memory-capture summaries in the memory store: saved and historical
  fact totals, queued/failed work, and the latest eight capture receipts, independent
  of search pagination. These power the Knowledge → About you overview.
- Regression coverage for saved Google desktop-client precedence, credential
  redaction, restart persistence, both sign-in entry points, and Settings controls.

- **WhatsApp and Telegram from chat:** the orchestrator can call a shared
  messaging helper to check connections, inspect locally retained recent chats,
  and prepare messages. Chat shows the exact sender, recipient, and text with
  Send, Cancel, and Save. Sends require native review, expire after ten minutes,
  reject stale links and repeated approvals, and report service acceptance or
  uncertainty without automatic retries. Skills includes a plain-English example.
  Connected Google tools now equip the Google helper as well.
- **Google client setup in Settings:** when Google has no desktop client
  configured, Connections accepts a client id and optional secret. The saved
  client takes precedence over environment and build settings, is shared with
  Continue with Google, and survives signing out. Secrets use the existing
  credential store and are cleared from the input after saving.
- **Messaging privacy:** authenticated own chats retain personal capabilities;
  other phone senders cannot use the owner's memories, folders, connected account
  tools, private instructions, or outbound messaging tools.
- **Development mode:** An opt-in Settings switch and explicit top-bar indicator
  let Codex, Claude Code, and other local MCP clients inspect, click, type, press
  keys, scroll, and capture the running native app. It starts off and remembers
  the chosen setting across restarts. The authenticated stdio bridge survives
  normal app reloads; switching off revokes access. Includes bounded requests,
  credential rotation, stale-control rejection, Skills registration, and the
  required agent testing workflow. A second app cannot overwrite the active
  instance's saved preference when its activation fails. See the
  [connection instructions](docs/development-mcp.md).
  Settings scrolls its header with its controls so the off switch remains reachable
  at the minimum window size and 200% text size. The indicator shares the
  existing native title bar and remains visible with window-manager decorations.

- **Read & compare any document:** When files are attached, the document
  reader helper can now read them cover to cover, several at once, and answer
  with file, section and line citations. It lists what is attached, searches
  every file in one step, and reads long documents in parallel parts, telling
  you exactly which parts are still unread. It also looks at attached
  pictures, scanned pages and recordings directly and, with a picture-reading
  AI, at the actual pages of a PDF for charts, tables, signatures and stamps.
  Find it on the Skills page.
  Reading is bounded to four concurrent batches and reports unread or failed
  sections explicitly. PDF page previews are limited to the current turn's
  attachments; older saved PDFs need to be attached again for page previews.
  Helpers whose tools only read now run those reads at the same time.

- **Connect a service:** In Settings, paste a web address to connect a service
  you already use. Sign in in the browser if it asks. A connected service can
  help during chat, and the reply shows which service did the work. Remove
  stops later use. An optional secret stays on this computer and is not shown
  again.

- **Dev build cache monitoring:** `cargo xtask dev` checks the Cargo target
  directory before every rebuild and warns above 30 GiB (`SOVRA_TARGET_BUDGET_GIB`).
  Watcher cycles preserve live build/test outputs; cleanup is an explicit step
  after the watcher and other Cargo work have stopped.
  Shared build outputs now resolve the invoking checkout at runtime, so a
  verification worktree cannot redirect the normal development runner.
  Dependencies no longer carry debug info in dev builds.
  Parallel build jobs are capped at 8 to keep RAM free while rebuilding.

- **Live folder reading view:** while a folder is read, Knowledge shows "Finding your
  files…" then a determinate bar with "1,204 of 5,000 files read", a smoothed and rounded
  time estimate, the file being read now, **What I've found so far** chips and **Your files
  seem to be about** life areas. Problems are one line with **See why**. Finished folders
  summarise what was found ("Found 320 photos, 85 documents and 12 spreadsheets.").
- **File understanding taxonomy:** every indexed file is classified into a plain-English
  kind (15 SKOS-shaped concepts linked to DCMI Type, schema.org and NEPOMUK NFO/NMO IRIs)
  and an optional life area (11 concepts, IPTC Media Topic links where genuine). See
  `docs/file-ontology.md`.

- **Local sign-in foundations:** account/session types, email and password
  validation, typed commands/events and workspace state updates. Passwords use
  Argon2id PHC hashes; accounts and the active session are stored through
  `MemoryStore` in SurrealDB. Signing out retains the accounts.
- **Optional Google sign-in backend:** authorization-code exchange with PKCE,
  a loopback callback listener (RFC 8252: literal `127.0.0.1`, OS-assigned port,
  constant-time state check, single callback), cancellation and credential-store
  handling for refresh tokens. The app reads `SOVRA_GOOGLE_CLIENT_ID` and optional
  `SOVRA_GOOGLE_CLIENT_SECRET` at startup.
- **Sign-in gate in the UI:** the shell requests the saved session at startup and
  routes to the workspace or the Login screen; on a fresh install the first submit
  creates the account, afterwards it signs in, with inline runtime errors. The
  Google button opens the consent page in the browser and the Login screen shows
  a waiting panel (*Open the page again* / *Cancel*) until the loopback listener
  reports back. Remaining rollout notes:
  [development notes](docs/development.md#sign-in-integration-status).

- **Find files without exact capitalization:** filename and scan-report filters
  now use Unicode case folding and canonical normalization by default. Match case
  restores exact matching. Filter changes restart pagination; literal `%` and `_`
  remain literal characters. Model catalogue queries expose the same option and
  the Partial text extraction filter.
- **Search more than 16 selected folders:** text, catalogue, reviewed-knowledge
  and summary scopes no longer reject larger granted-folder selections. Result
  pages, graph traversal depth and explicit summary request budgets still apply.

- **Read beyond the first saved excerpt:** file details offer First, Previous,
  Next and Last part controls, a saved-part count, and retry after a read fails.
  Each request loads one bounded text part from the selected version. Changed
  or revoked versions cannot be mixed into the preview, and partial-read warnings
  remain visible. Catalogue results expose the number of saved text parts too.

- **Add several folders together:** the native picker accepts multiple folders as
  one queued request, including selections larger than the 16-operation queue.
  Folders are registered before background scans start; a failed addition does
  not discard the rest. A persistent result lists failed paths and offers Retry
  failed folders. Existing folders keep their settings, and waiting folders can
  be paused or removed while another scan runs.

- **Files-first Knowledge navigation:** Knowledge opens to folders, with separate
  Memories and How it works sections that stay reachable while scrolling. Browse
  files and Scan report open focused views with Back to folders. Folder settings
  show three notice examples and link directly to the complete scan report.

- **Folder-specific large-document budgets:** indexing no longer inherits chat's
  20 MiB input or 4 MiB text limits. Folder settings offer Lower memory
  (64/16 MiB input/text), Standard (256/64 MiB, the default) and Large documents
  (1 GiB/256 MiB), with matching unpacked-byte budgets. Scans report the selected
  limit and recovery action, and changing it refreshes the folder. Budget-aware
  version keys prevent reuse across different limits. Input reads report progress
  and check cancellation in blocks. Indexed archives no longer have a 100-entry
  cutoff; cumulative bytes, nesting and codec protections remain bounded.

- **Visible partial reads:** Knowledge's file browser and scan report now have a
  **Partial text** filter. Decoder warnings, missing slides, unread archive media
  and other known omissions remain visible beside usable searchable text. Notes
  stay separate from source passages and survive restart and unchanged-file reuse;
  repairing a source clears the warning after refresh. Schema 10 preserves older
  reports/history, and `extract-v5` refreshes old representations.

- **Cancel during document reading:** Cancel, Pause and shutdown now reach the
  decoder's cooperative checkpoints, including documents inside archives.
  Interrupted replacements preserve previous searchable text instead of reporting
  a failed file. Each decoder registration is released when its read finishes.

- **Document indexing independent of attachment page limits:** local text
  extraction accepts PDFs and presentations beyond 500 pages/slides. Indexing
  no longer rasterizes scanned PDFs or prepares unused media payloads; files
  without a searchable text layer keep metadata and explain how to use OCR.
  Short text layers stay searchable. Attachment limits and document byte/expansion
  budgets remain separate; older index snapshots refresh with `extract-v4`.
- **Recoverable folder scans:** inaccessible subfolders or directory entries no
  longer stop readable sibling folders from updating. Each failure has its own
  path and recovery guidance. Partial walks remain visibly incomplete and keep
  older entries until a complete scan can confirm removals. Knowledge explains
  older results and offers **Retry scan**; failure counts exclude duplicate summaries.
- **Large legacy text without attachment limits:** folder indexing streams detected
  Windows code pages and multibyte encodings as well as Unicode. Full-source
  inference handles long ASCII headers and Japanese escape sequences; encoding
  guesses are labelled. Strict decoding, bounded buffers and transactional writes
  preserve full text without replacement characters or partial current versions.
  Streaming starts before single-byte text can exceed the decoded-text budget.
- **Office packages no longer fail at 100 internal parts:** format detection
  recognizes large Word, Excel and PowerPoint packages even with a misleading
  extension. Office/OpenDocument/EPUB package part counts follow the bounded input
  size; expanded-byte, text, page and nesting budgets still apply. Refreshed
  indexes re-extract older snapshots with the corrected decoder revision.
- **Full scan reports without the 50-example ceiling:** each folder has a paged,
  filterable report of every recorded skip, unreadable file and failure from its
  latest scan. Reports survive reopening, distinguish incomplete scans and older
  indexes that need a refresh, and announce entries added after a page snapshot.
  The compact folder summary still shows at most 50 examples. Latest reports
  replace older diagnostic rows; source history remains intact.
- **Complete large Unicode sources:** folder indexing now streams BOM-marked
  UTF-16/32 and conservatively inferred UTF-16 as well as UTF-8, including beyond
  the 20 MB source and 4 MB decoded-text attachment budgets. Decoding is strict,
  preserves split Unicode sequences and labels inferred encodings. Cancellation
  rolls back new chunks while retaining the previous saved version.
- **Browse indexed files in Knowledge:** list all observed files or one folder,
  filter filenames and reading status, move through pages, and inspect extraction
  explanations and literal saved-text excerpts. Browsing works during scans.
  Request-specific failures and stale-response checks keep old results from
  replacing a new filter or reopening files after folder access changes.
- **Per-folder inclusion settings:** folder details can include hidden files and
  generated folders. Choices persist and refresh the index; switching them off
  removes matching files from current search immediately while keeping saved
  history. Credential folders, symbolic links and Sovra's index data stay excluded.
- **Large indexed text and clearer scan notices:** UTF-8 text/source files are
  indexed completely in bounded buffers instead of failing at the attachment
  text/file limits. Validation, content hashing and atomic replacement preserve
  earlier snapshots on cancellation and prevent partial new versions. Scan
  details expand all recorded notices, including exclusion and symlink reasons.
- **Folder indexing without fixed collection caps:** removed the 16-folder,
  100,000-entry, 1 GB scan/database and 64-level traversal ceilings. The walk is
  iterative and cancellable; per-codec limits, OS resources and available disk
  still apply. No source files are changed.

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

- **Telegram, the same way (`sovra-telegram` new, `sovra-core`, `sovra-runtime`, `sovra-ui`, `sovra-app`)**:
  - **Settings → Telegram** links a bot of your own over Telegram's official Bot API, built
    into the single binary (HTTPS long polling, no third-party client, no sidecar). Create
    the bot with `@BotFather`, paste its token, and Sovra shows a six-digit code; send that
    code to the bot in Telegram and you become its owner. From then on your messages to the
    bot are answered by Sovra's helpers, with the same model, memory and web search as the
    chat window, and long answers are split to Telegram's size limit.
  - Safe by default: only the owner is answered (a switch allows everyone); people who find
    the bot before it is linked are told how to link it; groups and queued-while-closed
    messages are never answered. **Disconnect Telegram** stops the bot and forgets the
    token. The link resumes on its own after a restart.
  - The token is an API key, so it lives with the model keys (OS keychain, or the
    `SOVRA_DEV` plaintext files in debug builds), never in the saved page state or logs.
  - Both links now share one runtime task and one `MessagingChannel` port; adapters that
    poll implement `ChannelSession::poll` and the runtime does the waiting. The Connections
    page lists recent messages from both services with their service name.
  - Tests: core policy for owner linking and strangers, adapter message splitting, a runtime
    end-to-end run with a scripted bot, the Settings flow headless, and an opt-in live check
    that Telegram refuses a made-up token in plain words.

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
  - Verified against WhatsApp's servers: the opt-in test
    `cargo test -p sovra-whatsapp --test live -- --ignored` completes the handshake and
    receives genuine rotating pairing codes (61×61 modules, 60 s then 20 s).
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

- **Google in Settings → Connections**: a new **Connections** section in Settings with one **Connect Google** button.
  - **Sign-in:** Google's own page opens in the browser (single-use `127.0.0.1` listener, PKCE S256, fresh `state`, five-minute timeout, Cancel / Open again / Copy link). The person comes back to their name, email and a switch per service: Email, Calendar, Files, Contacts and To-dos.
  - **Partial consent:** permissions unticked on Google's consent screen show as off. Turning one on asks Google again; switching one off keeps the sign-in.
  - **Agent tools:** the agent gets tools only for switched-on services.
    - Gmail: search, read, and **draft only** (never sends).
    - Calendar: list, and add events with no guests.
    - Drive: search and read; Docs, Sheets and Slides as text.
    - Contacts: search.
    - Tasks: list and add.
  - **Safety:** outside text is clipped and marked untrusted, and model-supplied ids and email headers are sanitised.
  - **Storage:** refresh tokens stay in the OS keychain (dev file store under `SOVRA_DEV=1`). Disconnect revokes them at Google, and an expired sign-in becomes **Sign in again**.
  - **Implementation:** a new `sovra-google` adapter crate behind the core `GoogleConnector` port, with hand-rolled bounded reqwest/rustls clients (no second HTTP stack).
  - **Setup:** the Desktop OAuth client is baked in at build time via `SOVRA_GOOGLE_CLIENT_ID`/`_SECRET`; the same variables override it at run time. Scopes, the Google verification and CASA requirements, and setup are in `docs/google.md`.

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

- WhatsApp and Telegram messages are prepared and reviewed in chat. Removed the
  test-message inputs and test-send buttons from Settings; pairing, connection
  status, and automatic-answer switches remain.

- **Search while changing the selected AI:** Change on the selected model has a
  search box. It matches the name, the everyday price, or the sentence about
  what the model is good for. A search with no match says so.
- **See the price and what a model is good for before you choose it:** On Models,
  the selected AI and each model you can switch to show an everyday price
  (free, or about how much one message costs) and a plain sentence about what
  it is good for. If the service did not list a price or an ability, the choice
  says so instead of guessing.
- **Change the selected AI on its card:** When a model is already selected, Models
  shows it with a Change button on that same card. Change lists the other models
  already loaded, and choosing one selects it. Browse all models stays in the header.
- **Parallel folder scanning:** one capability-safe walker feeds N reader threads (cores − 1,
  max 8) that read, hash and extract in parallel, with a 512 MiB in-flight byte cap; a single
  writer saves results in batched transactions with per-file savepoints. Unchanged versions
  are skipped by reader-side lookups, a decoder panic on one damaged file no longer ends the
  scan, and progress is sampled from atomics (every 32 files or 100 ms) instead of per entry.
- **Refreshes skip unchanged files:** a file whose path, identity, size and modification
  time match its saved reading (same extractor and document budget) is confirmed without
  reading it again; git-style racy-timestamp protection re-reads files edited in the same
  second as the previous scan.
- **Progress bar appears almost at once:** a listing-only count runs beside the reading
  walk, so the determinate "1,204 of 5,000 files read" bar no longer waits for reading to
  reach the end of the folder.
- **File groups and life areas understand more languages:** bills, forms and life areas
  recognise common French, German, Spanish, Italian, Dutch and Portuguese words, with or
  without accents; ambiguous words are deliberately excluded.
- Folder card statuses use everyday words ("Done — ready to search", "Stopped — press Refresh
  to carry on") and the **Connect folders** skill card describes the live view in plain terms.
- **Skills is a short list:** Skills opens with a title, one sentence, search, plain
  filters, and cards. Each card shows a plain title, one sentence, an example, and
  a button that fills chat. Try the example fills that example, including when the
  card also opens a shortcut. A skill you added can be edited, turned off, or removed.
  Leave the shortcut blank and the name comes from the title.
- **Clear knowledge capture results:** Memories shows whether the last check
  saved details, found nothing new, or failed, with saved details and original
  sources above optional search setup. Files puts reading progress first, moves
  optional meaning search into Folder settings, and reports search progress and
  empty results. Enter submits a file search. How it works labels its example
  separately from saved information and explains that meaning search is optional.
- **Faster memory lookups:** up to four independent keyword reads overlap, with
  stable result order, existing deadlines and cancellation. Capture reads recent
  context alongside keyword matches; model extraction and corrections stay ordered.
- Knowledge evaluation now includes synthetic native UI captures and a saved
  memory smoke-test record. These document observed behavior and limitations;
  they do not claim an automated memory-quality score.

- **Web search fixed**: it used to rely on a public SearXNG server (`searx.be`) that answers in HTML, not JSON, so every search returned no results. It now uses the built-in sources above. The orchestrator also asks the search helper whenever an answer depends on current facts, not only when the user typed an exact phrase. More plain phrases ("search online", "look it up online", "google it") count as an explicit request.
- **Orchestrator offers only working specialists**: it no longer offers itself, or specialists whose integrations do not exist yet (email, calendars, GitHub, to-dos, running code), so it cannot hand a question to a helper that would only guess. A missing ability is now reported plainly.

- **Plain-language Skills & Shortcuts explorer & mandatory agent registration rule (`sovra-core`, `sovra-ui`, `AGENTS.md`, `CLAUDE.md`, `.cursor/rules`)**:
  - Replaced the flat vertical dump on the **Skills** page (`skill_library.rs`) with a *"What Sovra Can Do for You"* header, 3-step *"How it works"* banner, category filter pills (`All skills`, `Quick actions`, `Files & folders`, `Web search`, `AI helpers`, `Tables & cards`, `Made by you`) with live counts, and categorized responsive glass cards (`Surface::Glass`) across `t.layout.page_width`.
  - Moved `Skill::friendly_card_copy` (`SkillCardCopy`) into `sovra-core::skills::builtin`: every skill and specialist card now leads with an icon, a warm grandma-friendly title (e.g., *"Polite letter & report writer"*, *"Household budget & table builder"*), `/shortcut` pill, plain-English kind badge (`Instant action`, `Guided helper`, `Made by you`), a relatable example box, a 1-click **`Try example`** button (which fills the complete example prompt in Chat), and a **`Try in chat`** / **`Open now`** primary CTA.
  - Simplified the **Create a skill** form with numbered plain-English prompts, automatic `/shortcut` slug generation from the title when left blank, and progressive disclosure (`Show optional tool settings`) for technical tool names.
  - Added mandatory instructions across `AGENTS.md`, `CLAUDE.md`, `crates/sovra-core/AGENTS.md`, `crates/sovra-ui/AGENTS.md`, `crates/sovra-adk/AGENTS.md`, and `.cursor/rules/skills-page-agents.mdc` so Claude, Codex, Grok, and Cursor always register new agents/capabilities on the Skills page, enforced by `every_specialist_and_builtin_skill_has_grandma_friendly_skills_page_copy`.
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
- `cargo deny` now allows the `Unicode-DFS-2016` licence, which `finl_unicode` (pulled in by `xberg` → `comrak`) carries. It is an OSI-approved Unicode data licence, and the check was already failing on this before the Google work.
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

- Conversation identities stay distinct across app restarts, including chats
  outside the loaded history page and retained deleted conversations. New chats
  can no longer overwrite an earlier transcript or inherit its pending actions.
  Each launch opens a fresh draft; previous conversations remain in the sidebar.

- Folder document readers retain their owning runtime handle when registering
  cancellation from parallel reader threads. This fixes the missing-runtime panic
  that prevented documents from being indexed. Refresh previously failed folders
  in Knowledge to retry them.

- **Knowledge access for helpers:** delegated specialists and team members now
  inherit the turn's granted file search, history, reviewed-knowledge and saved-memory
  readers. Delegates can also read current attachments; obsolete attachment IDs
  remain unavailable. Standing instructions reach each helper without copying the
  parent's selected procedure. Independent delegations are queued above four active
  calls per invocation, and cancellation drops active and queued work. Real-index,
  HTTP-provider and ADK regressions cover cited answers, grant removal, context
  scope and concurrent execution. Helpers with a mix of concurrent and ordered
  tools preserve requested call order; parallel tool execution requires the whole
  tool set to be safe for concurrency.

- `sovra-ui` login form compiles again (missing `Disableable`/`FluentBuilder` imports).
- Connecting a service speaks to it in the current way on the first try. Sovra
  uses the older handshake only when the service does not understand that.
- Accepted conversation-memory inputs drain to storage when Sovra closes instead
  of being dropped with the worker. Interrupted extraction remains recoverable.
- Failed or invalid file searches hide previous matches instead of showing a stale
  success. Empty memory guidance and the teaching-example notice have explicit
  accessible labels.
- Unicode filename-search fixtures remain distinct on case-insensitive filesystems,
  preserving normalization, pagination and exact-case coverage on macOS.
  Non-Unicode path escaping is checked without filesystem IO on macOS, where
  APFS rejects the invalid-byte fixture names; the raw-filename scan remains
  covered on Unix platforms that support those names.

- **Reliable memory restart checks:** the lifecycle fixture uses the model
  restored at startup instead of reselecting it and racing the first recall.
  Failure cleanup shuts down the runtime before joining its event reader, so
  an assertion failure no longer leaves the test hanging.
- Removed duplicate sign-in dispatch/store attachment and unused account-form
  scaffolding that prevented CI from passing. Local `.claude/worktrees/`
  checkouts are ignored by the parent repository.

- **Sign-in runtime integration:** sign-in commands reach the account worker,
  Google configuration and storage are passed through, and shutdown drains
  accepted commands and cancels browser waits. Sign-in events stay outside
  conversation-memory capture and messaging replies. The existing sign-in form
  waits for runtime confirmation instead of entering the workspace immediately;
  signing out also notifies the runtime.

- **Show newly captured memories:** the open Memories list refreshes when a
  background capture finishes. Captures arriving during a load trigger a follow-up
  refresh; hidden lists refresh when opened. Corrections, source review, typed
  searches and older pages stay in place with a notice that new details are saved.

- Folder summaries now show skipped or unread-content notices without opening
  Folder settings. A completed scan with notices says to review them and points
  directly to Scan report; clearing the notices removes the prompt.

- SQLite and binary property-list indexing now use the folder's Document reading
  budget. Indexing removes SQLite's attachment caps of 100 tables, 10,000 rows,
  256 columns and 4 MiB of output, and the property-list attachment event cap.
  SQLite keeps readable rows when a budget is reached and reports Partial text
  with recovery guidance. Unread binary cells and invalid text are also disclosed.
  Both readers observe cancellation; refresh replaces older capped representations.

- Binary files with text-like names no longer become searchable partial text
  through the document decoder's permissive text fallback. The inventory keeps
  their metadata and explains how to inspect or convert them. Refresh replaces
  affected old representations; chat attachments retain labelled byte inspection.

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
- **No more empty bar at the top of the window**: when the window manager
  already frames the window (i3, X11, KDE and most Linux desktops), Sovra no
  longer draws its own blank title strip underneath. On macOS, Windows and
  Linux desktops where Sovra draws the frame, the bar now uses the sidebar's
  chrome colour and shows the open page. The window title now reads
  "<page> — Sovra" in taskbars and Alt-Tab, where it was previously blank.

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
