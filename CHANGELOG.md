# Changelog

All notable changes to SovraHarness are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). Releases are cut by pushing a
`vX.Y.Z` tag (see [README → Shipping](README.md#shipping)); installed apps
can use the generated Velopack release feed once update checking is integrated.

Add entries under **Unreleased** as part of the change that introduces them;
move them under a version heading when the tag is cut.

## [Unreleased]

- Answers from a free computer in your group now stream in as they are
  written, **Stop** stops the helping computer's AI within a second, and if
  the helping computer goes away before answering, the next free one takes
  over.
- **Let a free computer in your group answer.** Settings → Groups → *Share
  this computer's AI*: on a computer with an AI, *Let my group use this
  computer's AI*; on one without, *Use a free computer's AI when this one has
  none*. Chats on the second computer are then answered by the first, which
  receives only the conversation's words and never uses its tools, files or
  memory for someone else. Both start off; jobs are size- and rate-limited and
  listed for the helping computer's owner. Groups also shows which computers
  are online now. Skills has a “Let a free computer in your group do the
  answering” card.
- Same-room codes now work when the code was already showing before the other
  person pressed *Join with code*: each computer keeps a live list of nearby
  Sovra computers instead of listening only after Join. Found by driving two
  real Sovra windows through Development mode. Groups' text boxes now have
  stable ids and spoken labels, and group problems are announced as status
  messages.
- **Share helpers, skills and schedules with your group.** Helpers you made
  (Team page), your own skills (Skills page) and schedules (Schedules page)
  have *Share with my group*. Groups → *Shared helpers and schedules* shows
  each one's full words first; *Use this helper*, *Use this skill* or *Add to
  my schedules* makes your own copy. Shared helpers and skills never bring
  tools (each person grants tools on their own computer), shared skills are
  not applied automatically, schedules that send messages are never shared,
  and a shared schedule only runs on the computer that adds it. Sharing an
  update replaces the old version; the sharer or an admin can stop sharing.
- **Join a group in the same room with a six-digit code.** Groups → *Show a
  code* displays six digits; on the other computer, on the same Wi-Fi, type
  them under *Join with code*. Sovra finds the computer showing the code on
  the local network (mDNS, `iroh-mdns-address-lookup`), and both screens then
  show four pictures with their names and six digits. The computer joins only
  when both people press *They match*. A code allows one try within two
  minutes, and a wrong try burns it. Invite links now also show a QR code.
  Skills has an “Add another computer to my group” card.
- **Share with just one person.** In a group, details, memories and
  artifacts can go to *Just Dad* instead of everyone. Each pair of people has
  its own space with its own keys; its history only ever travels between
  those two people's computers, and their helpers are the only ones that can
  use it. Private items say “Just you and Dad”. Skills has a “Share
  something with just one person” card.
- **Group sharing is encrypted with keys that change when someone leaves.**
  Everything shared in a group (details, artifacts, agreements, corrections)
  is sealed with a group key before it is saved or sent; each computer in the
  group gets its own sealed copy of the key. Removing someone retires every
  key they held, so the next share makes a fresh key only the remaining
  computers can open. Newcomers receive the older keys so they can read the
  group's history; people who were removed keep only what they already had.
  Keys use the computers' existing device keys (X25519 from Ed25519) and
  AES-256-GCM, the same primitives iroh already ships.
- **Ask each other's memories (federated recall).** Settings → Groups can
  ask the group's computers a question (“Who is our dentist?”). Each computer
  whose owner turned on *Let my group's helpers ask my memory* searches its own
  memory and sends back up to four short answers with the owner's name; nothing
  else leaves it. *Ask my group when I chat* lets helpers do the same during
  chat (2.5 s deadline, quiet computers skipped). Both start off, owners see
  every question they were asked, and asks are rate-limited. Skills has an
  “Ask your family or team's memories” card. Direction for memory, compute and
  learning across desktops: `docs/roadmap/05-collective-intelligence.md`.
- **Share artifacts with your group.** Saved documents, lists, tables and
  pictures get **Share with my group** on the Artifacts page; everything the
  group shared appears under **Shared in <group>**, where **Keep a copy**
  saves it to your library. Sharing an edited copy makes it that artifact's
  newest version on every computer; two edits made at the same time are both
  kept (“two versions”) until someone shares one that settles them. **Stop
  sharing** works for the sharer or an admin. Each shared artifact is checked
  on every computer (title, up to 512 KB, plain text, never run). Helpers
  that read memory also find matching shared artifacts (“what's on our
  shopping list?”) and say who shared them.
- **Share a memory with your group.** A saved detail on the Knowledge page
  has **Share with <group>**: Sovra rewords “User's dentist…” as
  “Mum's dentist…”, you check the words, and everyone's helpers can use it.
  Skills has two new cards for these.
- Groups sync large histories in rounds (at most 6 MB per message), so a
  computer that joins a group with many shared artifacts still receives all of
  them. Viewers can read but no longer add to the group, on every computer.
- `rmcp` 3.8.0 → 3.9.0 (RUSTSEC-2026-0462: unbounded HTTP response
  buffering in its Streamable HTTP clients).
- Debug builds started with `SOVRA_DEV=1` may set `SOVRA_DATA_DIR` to run a
  second Sovra (its own device key and group) beside the first; release builds
  always use the system data folder.

- Fix large-folder embedding writes repeatedly rebuilding the full current-chunk
  catalogue per vector; use indexed membership checks while retaining permissions,
  source-version validation and the five-second transaction limit.

- Add `just free-cache` as an alias for the guarded Rust build-cache cleanup;
  it accepts the same flags as `just clear-cache`.

- Add an opt-in, hash-pinned LongMemEval-S source-retrieval diagnostic and a
  knowledge-capture tool assessment. Reports distinguish user-source recall from
  extraction, assistant-memory coverage and answer correctness.
- Add an opt-in disk file-index benchmark for 100/1,000/10,000 synthetic files,
  unchanged-file reuse, lexical retrieval and 768-dimensional vector scans,
  including replacement, reopening and access-removal checks.

- Google connects with one click: the Desktop OAuth client is baked into builds
  from a gitignored `google-client.env` (or build environment variables), and
  the client-id form only appears behind **Use my own Google Cloud client** in
  copies built without one.

- Settings → Groups offers **Create and copy invite link** and copies a new
  one-time invitation only after the runtime confirms it; **Copy invite link**
  lets you copy it again.

- Add first-use, knowledge review, schedules, groups, configuration/local-data and release guides;
  expand the docs index and reconcile credential, update, group-roadmap and
  network-isolation descriptions with the implementation.
- Expand the groups guide with the identities, signed membership log, invite
  links, peer sync and shared-knowledge rules the app runs, and keep the
  later lane and handoff design in the roadmap.

- Investor pitch slides no longer share one paper fill. Four light washes
  alternate down the deck, and nine slides carry a Bootstrap Icon from
  jsDelivr. The deck also loads the classic Lucide script from that host.
  The headlines stay in the page if that artwork does not load.
- Investor pitch slide 9 shows the permit history as three readable receipts
  — 12 March, 2 October, and what is current — then the question and the
  memo. The old timeline no longer crashes the lines together.
- Investor pitch slide 4 draws the sovereign stack as one building, with
  the desk under the roof and national compute in the base. Slide 6 draws
  three offices inside a solid border. Slide 15 draws the market as four
  filled quadrants. The words on those slides stay.
- Investor pitch beats no longer share one side-by-side spread. The answer
  is a full-width border under the claim, 100% sovereign is five columns,
  the path is a centered constellation, learning puts the diagram over the
  type, and the market map sits under its question. Only the product and the
  ledger keep the headline beside the picture. Slide 4 stays the play.

- Knowledge map: clearer guidance, readable clickable labels, keyboard-accessible
  connection filters, and named zoom and reading controls. Labels try several
  positions to avoid covering other details; the map has more room to explore.
  Focus on a detail to see only its direct connections, then use Show all to
  return to the complete map.

- Every deck and the decks site use the light colour scheme only. The
  deep-ink slides are gone from the pitch, value and UAE decks, the bars
  around the stage are pale lavender, and the site's list and viewer bar are
  light.

- The decks are one site: [`docs/decks/index.html`](docs/decks/index.html)
  lists every deck with a live preview, routes with the URL hash (`#/pitch`,
  `#/pitch/7`), shows a deck inside the site with a bar to switch decks, and
  works from `file://`. The pitch, value and UAE decks carry an **All decks**
  pill (`shared/decknav.js`) that returns to the site.

- Pitch slide 4 (`docs/decks/pitch/index.html#4`) states the play in type —
  own the sovereign stack — then four readable layers: the desk / Sovra,
  talent and policy, national models, and national compute.
- Record deny exceptions for `RUSTSEC-2026-0454` (reblessive `Send`/`Sync` in
  `surrealdb-core` 3.3.4) and `RUSTSEC-2026-0452` (short-key read in
  `surrealkv` 0.21.4). Both are patched only in SurrealDB 4.0.0; the embedded
  engine stays on 3.3.4. Remove when we move to `surrealdb` >= 4.0.0.

- The value and UAE decks take the pitch deck's rhythm: deep-ink slides for
  the title, section dividers, the problem and the honest-limits slide, with
  the particle-nation hero on the title and close; light scene and number
  slides in between.

- The investor pitch has a visual rhythm instead of one repeated layout:
  deep-ink slides for the title, the question, the problem, the proof, why
  Sovra wins and the close; light poster slides where the scene fills the
  stage; a one-giant-number slide for why now; an editorial numbered list; a
  section divider; and a giant-type ask. The particle-nation hero now has an
  ink variant.

- The value and UAE decks share the pitch deck's look: dark cinema bars
  around the stage, larger type, and no boxes. Cards, tiles, steps, phases,
  pills, bars and table panels are typography on hairlines; diagrams use
  dots, rings and glows instead of rectangles.

- Investor pitch at [`docs/decks/pitch/`](docs/decks/pitch/index.html)
  rebuilt as an 18-slide vision deck with a scene on every slide: the
  national border the work leaks across (and then stays inside), the
  sovereign stack with Sovra as the desk layer, the orbit of five
  things that stay yours, a constellation from one desk to a nation, the
  sealed national ledger with a residency report, lessons flowing into the
  nation's own AI, and three.js globes. Dark cinema bars frame the 16:9
  stage in any window. It tells the full vision without Today/Next labels;
  limits stay in `docs/value-realisation/`.

- Non-technical investor brief at
  [`docs/value-realisation/10-investor-brief.md`](docs/value-realisation/10-investor-brief.md):
  the UAE's AI-native goal, sovereignty as five things that stay yours,
  value realisation through measured pilots, a 10-minute talk track,
  investor questions and a plain-English glossary. Business model and ask
  are left for the founder.

- New [`docs/value-realisation/`](docs/value-realisation/README.md): the
  enterprise case for Sovra. Sovereignty in seven layers (data, knowledge,
  model, decisions, learning, operations, jurisdiction) with a scorecard; an
  evidence register citing the code for every Today claim, including every
  network destination and a fully local setup; value drivers mapped to
  measures; a value-measurement and pilot method; the person → group →
  company → regulated vision; regulated and UAE markets (not legal advice);
  limits and objections; dated sources. The UAE deck now says a fully local
  setup also needs web search off.

- New UAE deck at [`docs/decks/uae/`](docs/decks/uae/index.html) (Cinema
  1920×1080, plain words): AI-native and 100% sovereign, with value you can
  count. Five things stay sovereign (data, knowledge, the AI itself,
  decisions, learning), each marked Today or Direction; a value map, staged
  time to value, a measured pilot, a shared success plan, honest limits and
  dated sources for every outside figure.

- New value-realisation deck at
  [`docs/decks/value/`](docs/decks/value/index.html) (Cinema 1920×1080):
  value is work that is finished, kept with versions, and easier the next
  time. It covers the first day, everyday value moments, why memory
  compounds, sovereignty as how the value stays yours, and where to count it
  in the app. No invented ROI figures.

- Arabic is the default language for chrome (login, navigation, Settings, chat
  welcome, starters, composer) and for the helper's replies. Settings has a
  world-language picker; the choice is saved like appearance and restored after
  restart. Right-to-left layout is used for Arabic. A language without its own
  chrome catalog still changes the helper's reply language and never shows empty
  labels or raw keys.

- The family and small-business Cinema deck
  ([`docs/decks/group/`](docs/decks/group/index.html)) is a light 1920×1080
  stage: paper `#fbfaff`, dark ink, product violet, Today/Next pills, on-stage
  arrows, and reduced-motion that holds copy at rest. Today is the helper on
  one computer; next is a group of people and their computers sharing helpers
  and memory, with no central Sovra server.

- Group recall guidance is only added when a turn actually has shared group
  details, so personal sources still fit in the helper's request.

- **Groups** (people and their computers sharing helpers and memory; see
  `docs/roadmap/02-a-team.md`). Settings → Groups starts or joins a group
  with a `sovra://join` link, shows people and computers, invites, removes
  with a confirm step, and shares details everyone's helpers can use. Skills
  has “Share with your family or team”. Helpers that already read personal
  memory also see ranked group details with who shared them. Membership is a
  signed hash-linked log every computer folds the same way (strong removal,
  remove-wins, never-zero-admins, fork detection, pairing codes). Computers
  talk over `sovra-mesh` (iroh QUIC, internet by default,
  `SOVRA_GROUP_REACH=local` for the same network).

- Linked Devices on your phone now names this computer **Sovra connection**
  (instead of "Unknown device"). An already-linked phone keeps its old label
  until you unlink and Connect WhatsApp again.

- After a linked WhatsApp Web session is online, contacts the phone shares
  (including hidden-id rows that still carry a phone number) are stored in the
  people book, and groups you are in are stored with their id and name without
  waiting for a group message. An empty or failed pull does not wipe anyone
  already saved. A unique saved person or a unique stored group can be prepared
  as a review; nothing is sent until you press Send. Groups are listed separately
  on Knowledge → People and are never answered automatically. The phone's
  contact/chat bootstrap is processed instead of being acknowledged and discarded.

- README is a short front door: quick start, command table, mermaid graphs of
  crate layering, `Command`/`Event`, and one chat turn through memory, then a
  map of the guides. Top-level docs each open with a purpose sentence.
  `cargo xtask check-docs` fails if those graphs or relative markdown links
  go missing.

- GitHub CI now splits lint, tests and a Windows compile, keeps going on `main`
  when a new push arrives, and asks every pull request to update the changelog.
  Contribution workflow, PR template, Dependabot and CODEOWNERS are in the repo
  so several people can work on it at once. See
  [CONTRIBUTING.md](CONTRIBUTING.md).

- Terminal: while a command is still running, its stdout and stderr show on the
  card and go into the helper's result — including programs that only flush when
  they think they are talking to a terminal. Stdin stays closed, so a program
  waiting for a keyboard still fails fast instead of hanging.

- Terminal: if the helper's reply times out while a command card is still waiting
  for your OK, **Allow once** and **Always allow** still run it. The card no
  longer becomes "Not answered, so it didn't run" just because the model
  connection timed out. The assistant is told to call `run_terminal_command` and
  do the work, not paste a command in chat for you to copy.

- Rolling a dice with the sample MCP server now shows a native card (the roll and
  a **Roll again** button), not only the sentence in chat. Other MCP servers can
  send the same AG-UI envelope in `structuredContent`.

- When Sovra needs to run a command or change files, it calls the terminal tool
  so you see **Allow once** / **Always allow** / **Don't allow** on a card. It no
  longer asks "Should I go ahead?" in the chat. **Always allow** still remembers
  that program (for example `mkdir`) so later similar commands can run without
  asking.

- Timed WhatsApp and Telegram jobs now need a concrete recipient when you create
  them: the international number starting with +, me, an exact chat id, or
  `person:<id>` from a people lookup. A first name such as Ryan is no longer
  stored as the send target, so the job cannot sit until due and then fail
  instantly. If a timed send still does not go out, Schedules shows why it
  didn't finish.

- Connections now has an **MCP servers** card, renamed from "A service you already use"
  and opened with **Add MCP server**. Every label is in plain words: "From a web
  address", "From a program on this computer", "Paste setup from instructions", "Server
  address (URL)", "Key or token (optional)".
- New **Try the sample MCP server** button: a safe sample server built into Sovra
  (`sovra --sample-mcp-server`) with four simple tools. It connects in one click, so
  anyone can see what MCP servers do, and developers have a server to test against.

- Connected services can now be managed at scale. Add a **web address**, a **program
  on this computer** (local MCP servers such as `npx …`/`uvx …`), or **paste
  settings** in the `mcpServers` format used by Claude Desktop and Cursor (also VS
  Code's `servers` and Zed's `context_servers`). Each service has its own card with:
  - a status;
  - a pause switch;
  - per-tool switches (switched-off tools are never offered to the model);
  - Reconnect/Restart;
  - Rename;
  - Remove, which asks first.

  Search covers names, addresses and tools, and services that need you come first.
  Programs never run until you've seen the exact command with plain warnings and
  chosen **Run it**; any change to the command asks again. Programs run in their own
  process group with your login shell's `PATH`, and a crash shows what the program
  said. Private settings and extra headers are kept in the keychain. The saved list
  moved from the keychain to a JSON file (migrated automatically), because Windows
  limits keychain entries to about 2.5 KB. Services reconnect at start-up, and tool
  calls and sign-in waits no longer block other services. Tool names are capped at
  64 characters. See [docs/connected-services.md](docs/connected-services.md).

- Terminal access: Sovra can run commands on your computer with
  `run_terminal_command`, approved the way Claude Code does it. Read-only
  commands run straight away. Everything else shows a chat card with the exact
  command, the reason, plain-English risk notes and **Allow once** / **Always
  allow “<command>”** / **Don't allow**. Compound commands are split, and every
  part must be allowed. Wrappers (`timeout`, `nice`) and safe environment
  prefixes are stripped before matching. Commands that hide inner commands
  (`$(…)`, here-docs), launch other code (`bash -c`, `sudo`, `xargs`,
  `find -exec`) or are destructive (`rm`, `git push --force`) are asked every
  time and never remembered. The tool tells the model the OS, its version,
  architecture, shell and starting folder. Commands run in the login shell's
  environment with stdin closed, in their own process group, with live output,
  **Stop**, a time limit and whole-tree cleanup on timeout, Stop or an
  interrupted chat. Output for the model is bounded. Settings → Terminal
  switches access off and removes saved rules. The terminal is never offered to
  WhatsApp or Telegram senders. Registered on the Skills page as “Do things on
  your computer for you”. See [docs/terminal.md](docs/terminal.md).

- Clicking a button on an in-chat card now sends that button's label as your
  next message, so the helper continues the flow. It no longer dumps a JSON
  blob such as `surface_ref` / `button_click` into the transcript.

- A confirmed timed WhatsApp or Telegram text now goes out at the scheduled
  time. After you confirm who, which app, the exact words, and when, the job
  sends on its own — you do not have to be there to press Send. Live chat
  still prepares a review and waits for Send. Ask in chat, or use
  **Set a one-time or repeating task, including a timed text** in Skills.

- Knowledge and the other workspace pages follow the pane. About you, People,
  Files, Guide, and a source opened from chat shrink on a small window, stay
  centred inside the reading width on a wide one, and scroll inside the page.
  The memory picture matches that column and shortens when there is no room
  for a side column. Action rows wrap, and a 200% text size enlarges the
  type and the controls together. Chat, My AI's, Schedules, Skills, Settings,
  Connections, Models, and sign-in follow the same fit.

- Repeating tasks on the Schedules page are real: a new workspace starts empty,
  you describe how often in ordinary words (in 1 minute, just once, every
  weekday at 9, Fridays at 4, every few hours), and Sovra runs them in the
  background, pauses them, and skips a tick if one is already running. A
  one-shot such as “in 1 minute” or “just once” fires once and then stops —
  it is never saved as every day at a clock time. A confirmed WhatsApp or
  Telegram text on that job goes out at the due time without pressing Send.
  A swimlane shows past runs and the next one, with a line for now. Ask in
  chat to set one up — the helper is always available, and Skills lists it as
  **Set a one-time or repeating task, including a timed text**. “Fridays at
  4pm” and “every day at 12:00” keep the afternoon and noon you asked for.
  Stopping a run or restarting Sovra closes a leftover in-flight job so the
  next due time can fire.

- Dragging a text selection across a chat now highlights both your message and
  Sovra's reply, instead of only theirs.

- Skills now apply on their own when a request clearly fits, so you no longer
  need to type `/`. Sovra sees a short list of skill names and when each helps.
  It loads a skill's steps only for a real match, picks the single most
  specific one, and leaves greetings and quick questions alone. The list holds
  your own skills plus the built-in file and folder procedures. Web search,
  helpers, and tables and cards already run without a skill, so they are not
  listed. Untrusted message senders never see the list. A new **Use it on its
  own when a request fits** checkbox in the skill editor, and
  `disable-model-invocation: true` or `sovra-auto-apply: false` in `SKILL.md`,
  keep a skill to the `/` menu. Typing `/` still forces a skill.

- Chat recall listings no longer show `[memory:fact:…]` / `[source:episode:…]`
  hashes in the bubble, labels or tooltips. Saved details appear as an in-chat
  card with Open on each row; the original tokens stay in stored text so
  checking a source still works.

- Knowledge memories now draw a picture of real links between saved details:
  an earlier version, the same original message, a shared subject, or the chat
  a detail came from. Each chat uses the conversation's saved title, and two
  conversations stay separate when the titles match. A note typed on the memory
  screen is not shown as a chat. The words stay beside the picture. A check on
  the same screen finds exact repeats and close wording without a model or a
  network call. Exact repeats, and close wording you confirm, are stored as an
  earlier version. Both details stay on the page after a refresh. Close wording
  is not listed again once that link is stored. Nothing is deleted.

- Compatible providers that omit a tool-call ID now receive a distinct local ID
  before execution, keeping memory lookup calls and results paired on the next
  model round. Existing provider IDs are preserved.

- New ordinary and team replies can be recalled across chats when all visible
  history and saved-memory inputs have durable source links. Provider requests
  and recalled evidence are tracked independently of model-written citations;
  withdrawing an ancestor also hides dependent replies. Unknown history, files,
  external tool results and opaque provider history keep replies review-only.
  Source receipts persist against the exact originating message even after chat
  navigation, without changing its activity date. Schema 11 preserves older chat
  JSON and defaults missing source links to unknown; no automatic backfill runs.
  Fact provenance loading now applies explicit source permissions to direct,
  background and related evidence, excluding legacy unknown grants from model
  input while preserving their review records. Schema 12 distinguishes a removed
  detail's raw source wording from whole-source revocation, keeping unrelated
  grounded details usable; existing revocations remain in force.

- Memory storage now verifies bounded, immutable dependencies before returning
  helper wording as recalled evidence. Source withdrawal blocks dependent replies
  transitively; corrections label changed support. Unknown, missing or oversized
  dependency chains remain reviewable but unavailable to recall. Schema 10 keeps
  existing IDs and leaves legacy helper dependencies unknown. Runtime integration for conversation and saved-memory inputs is described above.

- Switching chats now keeps each conversation's live reply, helper attribution,
  Stop state and queued follow-ups together. Background completions save the
  originating conversation, and follow-ups carry that conversation's context.
  Leaving a live chat or stopping it saves the partial transcript; completed
  ordinary and team replies survive reopening the local database.

- Team replies now retain original wording, helper name and run/member identity
  in saved-source review, independently of personal-fact extraction. Named replies
  show who wrote them. Capture uses bounded buffers and a bounded event channel;
  storage failures remain visible. A pause invalidates in-flight normal and team
  replies even after remembering resumes, including at transactional admission.
  Completed team messages and final run state now also save to chat history,
  fixing replies that disappeared from the conversation after restarting.

- Conversation history now uses real event times immediately, including before a
  save completes. Continuing a saved chat advances its activity date while keeping
  its creation date; switching chats leaves those dates alone. History groups use
  the current clock, so an old newest conversation no longer appears under Today.
  Storage preserves supplied dates instead of guessing from small numbers.

- Background remembering now fills available extraction slots when evidence arrives,
  a job finishes, or interactive AI work ends. The 250 ms poll remains for delayed
  retries and lease recovery; it no longer limits ready work to one start per tick.
  Pause and foreground requests retain priority, with four process-wide slots by default.
  Existing job updates use their cancellation row as the transaction fence instead
  of rewriting shared policy metadata; pinned-engine tests verify pause and withdrawal
  still reject stale writes.

- New private chat replies retain explicit helper authorship, producer identity,
  version and admission time in saved-source review. Durable save receipts follow
  persistence; reply-storage failures are visible. Assistant text is not extracted
  as a personal fact. Cross-chat recall requires the complete input tracking
  described above. Withdrawing an admitted
  source revokes all its immutable versions and their pending work. Schema 9 adds
  source-family identity without inventing legacy authorship or permissions.

- **Deep research always runs as an in-chat research team:** `/deep-research`
  (and the DeepResearchAgent skill) opens a kanban board in the chat transcript
  with Waiting / Working / Done lanes, researches independent goals in parallel,
  and finishes by saving a Document artifact with Open — intermediate goal notes
  stay off the transcript. `DeepResearchAgent` is no longer offered as a solo
  orchestrator AgentTool (that path could dump prose into chat); Skills / My AIs
  still list it, and the dedicated board path owns every run. No separate
  Team/Goals window.
- Memory tool rounds now retrieve every requested query, up to four in parallel,
  instead of discarding all but the last. Results retain separate question/date/
  person labels under one evidence budget, and final permission checks cover the
  whole batch, including earlier finished reads. Focused lookups omit unrelated
  recent background details; initial automatic recall retains its small working set.
- Original-evidence search now combines separately ranked search terms using
  bounded reciprocal-rank fusion, preserving distinctive terms when newer questions
  share a common word. Scope, withdrawal and temporal filters remain enforced.
- Added an opt-in synthetic runtime memory benchmark at 1k/10k/100k sources and
  1/4/16 concurrent requests, with zero-cost loopback responses and explicit metric
  limitations; real-model recall reports retain the observed citation defect and repeat.
- Fixed incomplete research-board widget rendering and research test integration
  that prevented the shared workspace from compiling.

- **`just clear-cache`:** discoverable recipe that reclaims reclaimable Cargo
  build caches via `cargo xtask clean-cache reclaim`. Explicit reclaim always
  runs profile-scoped `cargo clean` for `dev` and `release` (still refuses while
  `cargo xtask dev` holds `target/dev/watch.lock`, and still reports target size
  before that refusal). Status remains `just clean-cache` /
  `cargo xtask clean-cache`.

- **Remove from memory** now withdraws a detail's whole correction history and
  original/repeated source records after confirmation. Concurrent corrections
  invalidate stale removals; pending source chunks cannot restore removed details.
  Stopped history remains reviewable, and unrelated details and chats are kept.
  **Edit detail** continues to save a new version with its original wording.
- Independent memory searches now retain successful evidence when another search
  fails or exceeds its deadline, disclose incomplete coverage, and reserve time
  for provenance and final permission checks. Removal during an in-flight search
  still invalidates the whole result.

- Private chat turns now receive a small, source-grounded set of current saved
  details alongside targeted retrieval, so basic context does not require exact
  keyword overlap or an embedding service. Historical lookups exclude that current
  working set. Memory lookup instructions now ask for alternative wording before
  concluding that personal information is unavailable. Broader recall, edit/remove
  and performance acceptance remains in progress.

- **`cargo xtask dev` is one-shot only:** no Watchexec / save-triggered reload.
  Re-run `cargo xtask dev` yourself after edits. Setup no longer installs
  Watchexec. Agent and development docs (`AGENTS.md`, `docs/development.md`,
  README) now require manual reload and forbid starting an auto-rebuild watcher.
- Saved-memory details now open **Browse supporting records**, including repeated
  statements from other conversations. Search and paging stay scoped to that
  detail, stopped evidence remains explicitly selectable, and returning reloads
  the detail while preserving its exploration trail. Repeated records now report
  their linked-detail count correctly and can open those details.

- **Knowledge → Files → Browse saved relationships** opens reviewed file
  connections without a model call. Search, page through older results, follow
  subjects, and read cited passages with inference, date and source-freshness
  labels. Outdated sources and replaced relationships use explicit filters;
  removing folder access clears open records and invalidates pending reads.
  Empty results link to a relevant helper in Skills without sending a message.

- **Browse original records** now searches and pages through retained memory
  evidence independently of extraction. Readers show recorded origin, processing
  and withdrawal status, and link back to saved details and history. Stopped
  records require an explicit filter; reviewing them does not restore recall.
  Legacy metadata stays unknown, and newly written personal notes now record
  their explicit user origin and private scope. Processing-status reads are
  batched and use an engine-verified source index.
  The reader keeps the original wording in focus with compact metadata and
  wrapping actions, without repeating the overview header.

- Made the file-index same-second edit regression independent of wall-clock
  timing. It now also verifies that a same-size edit with a preserved timestamp
  replaces the searchable text.

- **Browse all subjects** now opens a searchable, alphabetical directory of the
  people, places and projects in current saved knowledge. Pages keep stable
  name boundaries; selecting a subject opens its details. Enter submits searches,
  and explicit navigation starts at the top while background updates keep the
  reader's scroll position.

- Updated the locked Markdown parser (`pulldown-cmark` 0.13.6) to address the
  nested-image wikilink parsing advisory reported by the CI security gate.

- Knowledge → People lists everyone you can message. While WhatsApp is linked, contacts the phone has synced are collected in the background, so a saved person can be prepared for review before they have written. The same person can also be reached by phone number, Telegram, and email. Each card shows those details. **Message on WhatsApp** opens the chat, where you press Send. A cancelled, sent, or expired review can be prepared again. A shared first name does not join two people, and nothing is sent until you press Send.
- Saved-memory cards now show readable dates and selectable subjects. **Explore
  detail** opens the original evidence and **Connected knowledge**, with reasons
  for shared sources, shared subjects and corrections. Follow links in either
  direction through version history, return along the exploration trail, and
  filter the library by an exact subject without treating co-occurrence as proof.
  The overview surfaces the most-mentioned subjects across current knowledge;
  recent activity opens original messages even when extraction found nothing.
  Long sources expand on demand, and background-save notices remain visible
  while a correction is being written.

- **`cargo xtask clean-cache`:** report and reclaim Cargo target disk use against
  the existing 30 GiB budget (`SOVRA_TARGET_BUDGET_GIB`). Reclaim refuses while
  `cargo xtask dev` holds `target/dev/watch.lock`, prefers profile-scoped
  `cargo clean`, and can optionally prune recreatable `~/.cargo` registry/git
  caches without touching binaries, config or credentials. Reclaim only when no
  `cargo xtask dev` session is running.
- Watched debug builds now start with visible Development mode enabled, per the
  workspace policy. Ordinary and release binaries start with it disabled, even
  when an earlier development preference was on; Settings still revokes access.
- Fixed Connections refresh retaining its own inbox and delaying runtime shutdown.
- Checking links no longer marks a reconnecting WhatsApp link, or a Telegram link still waiting for its owner, as linked when the check did not reach the service.

- Conversation memory now keeps durable pending evidence beyond the old queue
  capacity, processes bounded source chunks with persisted offsets, and overlaps
  extraction with a configurable process-wide limit of four. Consolidation stays
  serial and checks the current fact revision before committing.
- Extraction candidates survive retries and restarts before consolidation. New
  background model calls yield to interactive turns; preceding context excludes
  withdrawn and unadmitted sources. Original-source review includes a confirmed
  **Stop using this message** action that cancels pending work and suppresses recall.
- Recall can cite original admitted private messages even when extraction omitted
  their details. Source-only citations open the original message; unknown legacy
  permissions, paused memory and withdrawn source evidence are excluded.
- The broader all-source memory/backfill plan remains in progress; no benchmark
  speedup or model-quality improvement is claimed by these changes.

### Current progress — 2026-11-03

- **Knowledge and remembering:** About you, browse original/supporting records,
  subject directory, connected knowledge, remove-from-memory, durable reply
  source links, dependency-checked recall, and parallel memory-tool batches are
  in this tree. Live-model answer quality still needs broader evaluation; see
  [docs/agent-memory.md](docs/agent-memory.md) and the evaluation notes under
  `docs/evaluations/`.
- **People and messaging:** Knowledge → People lists messageable contacts with
  WhatsApp sync prep; reviewed send flows remain gated on explicit Send.
- **Deep research:** `/deep-research` runs as an in-chat kanban team and saves a
  Document artifact; it is no longer offered as a solo AgentTool dump path.
- **File knowledge:** Browse saved relationships and related indexing work are
  included; background summary generation and compiled maps remain unfinished.
  See [docs/skills-and-file-knowledge.md](docs/skills-and-file-knowledge.md).
- **Development tooling:** `cargo xtask dev` is one-shot (no Watchexec).
  `just clear-cache` / `cargo xtask clean-cache reclaim` reclaim build caches
  when idle. Development mode still starts on for `SOVRA_DEV=1` debug launches.
- **Verification:** xtask `clean_cache` unit tests passed (7). Full
  `cargo xtask ci` was not re-run for this entire unreleased batch at push time;
  run it after pull if you need the full gate.
- **Integration scope:** current uncommitted work is consolidated on `main`.
  The older `memory/base`, `memory/base-2` and `memory/living` prototypes remain
  separate from the current memory implementation.

### Added

- **Connections tab:** the sidebar lists every shipped link in everyday words
  (an online AI, an AI on this computer, another compatible service, WhatsApp,
  Telegram, Google, and a service added from a web address). Each row shows
  whether it is not linked, working, linked, or needs you, plus one action.
  Settings keeps preferences and opens this tab. Checking links looks at
  installed connections at the same time, with a bound, so one stalled link
  cannot delay another. Linking and unlinking stay on the page; a helper that
  can prepare a message still runs its tools in order.

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

- Development credential files accept dotted names such as
  `connection.google.client` and `connection.google`, fixing saved Google setup
  and account credentials under `SOVRA_DEV=1`. Each name stays a single file;
  empty components, traversal and directory separators remain rejected. Signing
  out removes account credentials while keeping the desktop client setup.
  Regression tests cover file permissions, name validation, restart persistence
  and both Google entry points. Release builds continue to use the OS keychain.
- Development-mode, memory-history and partial-summary runtime tests use longer
  bounded event waits to tolerate scheduling contention during the full suite.

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
