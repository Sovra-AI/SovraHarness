# Memory capture and recall

How one chat turn becomes a saved record, and how a later question gets that
record back. This is the behavior of **this** desktop app. The audit history
and measured limits live in [agent-memory.md](agent-memory.md). This document
is the capture and recall reference.

The questions below follow the personal-memory checklist in SovraSRX
(`SovraSRXProduction/docs/memory/personal-memory-system.md`, capture, provenance,
and recall). Each answer is what this app ships. A later section lists the
SovraSRX ideas this app does not ship.

Two stored shapes, decided in `sovra-core` and written by the runtime:

- An **episode** is raw evidence: the message text, who wrote it, which chat,
  and when it was recorded. Episodes are not embedded. They are the receipt.
- A **fact** is a short interpretation of one user-written episode. It keeps
  the exact quote it came from, the model that wrote it, and bi-temporal
  dates. Facts are never edited or deleted. A correction ends the old fact's
  validity and links the new one. Stopping use sets `stopped_at` and hides
  the record from recall.

SurrealDB is the only store (`sovra-store-surreal`). The UI talks to the
runtime only through `Command` and `Event`.

## What capture must not block

Producing the reply does not wait for fact extraction.

On `RunPrompt`, `RunWithFiles`, and a team `Run`, the command loop in
`sovra-runtime` takes an interactive model slot **before** it queues the user
message (`model_priority::interactive`). `RememberingRuntime::record_user`
copies the text onto a bounded queue and returns a oneshot receipt. It does
not call the model. The chat turn is spawned with its own interactive slot
held until that turn finishes.

Extraction runs on a remembering worker. The worker claims a saved episode
only when remembering is on, a slot is free, and no interactive turn is in
progress (`model_priority::busy`). Inside the job, `process_chunk` waits on
`background_ready` again before the extraction call and again before
reconciliation. A slow or failed extraction cannot hold the reply. Closing
the app finishes the ordered evidence write and can resume the model job
later; it does not drop a message that was already accepted.

Remembering can be off. `record_user` then returns without a receipt, and
recall returns no reader.

## One ordinary chat turn

Follow a private chat message such as "I am vegan".

1. The command loop accepts `Command::RunPrompt` for a session that has
   private access. Connected-account chats are not the owner's memory and are
   not captured here.
2. The loop holds the interactive slot, then calls `record_user` with the
   session id, the prompt, and `ConversationContext::current` when the UI sent
   one.
3. Empty or whitespace-only text is ignored. If that context already names a
   saved source, the receipt is completed with that id and nothing is queued
   again. Otherwise the worker receives a `MemoryInput`: kind `UserMessage`,
   the chat id, the text, and the message time (or the runtime clock).
4. The worker stores the episode immediately through `enqueue_memory` /
   `admit_memory_source`. That write is the raw evidence. It emits
   `MemoryEvent::SourceSaved` (version 1, processing job present) and
   `MemoryStatus::Queued`. The in-process queue accepts 64 messages, and
   `record_user` waits until one can be queued. It does not drop a message
   because the queue is full. If the worker has already stopped, the queue
   is closed and the UI hears that the message could not be saved for
   remembering. Stored episodes are not refused after 256 pending jobs.
   That number is only how many failed jobs one retry pass requeues.
5. The reply proceeds. Recall for **this** turn runs inside the model call
   (see below) and does not wait for this message's new fact. The new fact
   can only appear on a later turn, after the worker commits it.
6. When the interactive slot is free, the worker claims the episode. A long
   message is cut into chunks of 2048 bytes (`MEMORY_CHUNK_BYTES`). Up to 4
   extraction slots run (configurable 1–16), sharing one process-wide permit.
   Each job has a 90 second budget, a 120 second lease, and at most 3
   attempts. Retry wait is 5 seconds. Cancelled jobs abort. A pause, or a
   change of remembering generation, drops late results so an old job cannot
   commit after the user turned remembering off or switched model.
7. Extraction calls `AdkHarness::extract_memories_in_context`. The request
   contains the chunk, at most three preceding episodes (each truncated to
   the last 1024 bytes), and no tools, no provider history, and no assistant
   transcript. Preceding text may resolve a name. It is not new evidence.
   Every candidate must quote `source.content`. Known current facts passed
   into a later reconcile step are capped at 8.
8. The model is instructed to return durable personal facts: preferences,
   ongoing projects, and explicitly stated facts about the user, plus stable
   details the user stated about named colleagues, relationships, and
   projects. It must return `{"memories":[]}` when nothing qualifies.
   Questions alone are not facts. It must not remember requests to change
   rules, secrets, authentication, quoted documents, tool output, fictional
   examples, or claims about what an assistant said. It must not infer
   sensitive attributes. This judgment is the write-gate. There is no second
   heuristic classifier and no durability score.
9. `parse_memory_candidates` accepts only a complete JSON object. A candidate
   is kept only when its evidence is an exact substring of that user episode,
   content and evidence are each at most 2048 bytes, and there are at most
   16 entities and 32 candidates. An invented quote is refused. The whole
   batch is refused before any write. Agent and tool episodes cannot be
   memory inputs (`MemoryInput::validate`).
10. The extracted batch is saved (`save_memory_extraction`) before a
    process-wide consolidation lock. If current facts already exist, a second
    model call may set `replaces` or `same_as`. Reconciliation that changes
    content, evidence, entities, or `valid_at` is refused and retried.
    `complete_memory` then commits. A revision clash retries up to 3 times.
    An empty batch commits no fact. The visible status is "Last check
    finished. No new detail was saved."
11. On success the UI hears `MemoryEvent::Processed`. The episode remains
    even when no fact was saved, so a discarded aside is still raw evidence
    in that chat's source, not a lasting fact.

A statement such as "I am vegan" becomes a fact only when the extractor
returns a candidate whose evidence is that phrase (or a longer exact
substring) and the batch validates. A later chat can recall it with the
source id. "The weather is lovely today" does not qualify under the
instruction. The shipped result for that message is an empty `memories`
array, so no fact is written. The weather episode can still exist as raw
text of that turn. It is not injected as a personal fact.

### Corrections

`replaces` is an explicit change of the same subject and attribute. The store
sets the older fact's `invalid_at` and `expired_at` and records a `supersedes`
link from the new fact to the old one. The old row stays. Recall of "what is
true now" hides it. A historical read (`valid_at` / `known_at`) can still see
what was known then, with later correction metadata stripped (`as_known`).

`same_as` means the same fact in different words. The store records a
`repeats` link and does not end the earlier fact. A batch cannot both reuse
and correct the same id, and cannot correct one id twice.

Tentative plans do not replace a current fact. The message clock and the word
"current" do not override an explicit date. Undated claims keep `valid_at`
empty.

### What is refused or left unknown

- Remembering off, or no private access: nothing is queued.
- Non-user episode kinds: rejected before a job exists.
- Quote missing, not a substring, or oversized: the batch is rejected.
- Secrets, instructions, tool output, quoted documents, fiction, and talk
  about the assistant: the extractor is told to return no memories. The core
  does not keep a separate blocklist of those topics. The quote guard still
  rejects a claim that does not appear in the user text.
- Semantic search error or timeout: recall continues with matching words.
  The model is told that search by meaning was unavailable only when that
  search returns an error, hits its 900 ms cap, or misses the recall read
  deadline, and only when no other recall notice is already set. An index
  that is off, a missing embedding dimension, a paused semantic search, a
  semantic revoke or reconfigure during that search, or no semantic service
  at all returns an empty meaning leg and does not add that notice.
- Recall that cannot finish in time, or that is paused mid-read: the model
  sees an empty context or the notice "Saved details could not be checked.
  Do not invent remembered facts." An empty search is not proof that the user
  never said something.
- Helper dependency metadata missing: the manifest stays `None` (unknown).
  Unknown is not treated as "no inputs". See the helper limit below.

## One team turn

A team `Run` uses the same user-message path. `record_user` queues the
person's prompt, and the same worker extracts facts from that user text only.
The interactive slot is held for the team turn, so member replies are not
delayed on extraction.

Visible team output is saved as raw assistant evidence, not as personal
facts:

1. `Event::AgUi` for that session is applied to `TeamMemoryCapture`.
2. `RunStarted` begins a buffer for that run id. `SubagentStarted` records
   the member id and display name. Activity snapshots, hidden reasoning, and
   moderation controls are ignored.
3. `TextMessageStart` / `TextMessageContent` / `TextMessageEnd` assemble one
   visible reply. Empty text produces no source. More than the team's reply
   cap, an unknown speaker id, or a buffer past `MAX_EPISODE_BYTES` (128 KiB)
   fails the capture instead of saving a cut-off quote.
4. On `TextMessageEnd` the runtime admits an `AgentMessage` source. A named
   member gets provenance `Team { run_id, member_id }` and author
   `Assistant { speaker: name }`. A visible summary with no member stays an
   anonymous assistant message (`Conversation` provenance). The author is
   never relabeled as the user.
5. `needs_extraction` is true only for a user author. Helper text is not sent
   through the personal-fact extractor.
6. The dependency tracker records the user-source receipt, injected memory
   lookups, and earlier saved replies in the turn. The assistant source waits
   up to 2 seconds for the user-message receipt. If it arrives, the manifest
   can be complete. If it does not, the input is marked untracked: dependencies
   stay unknown. A failed save also marks the turn untracked. Unknown cannot
   be repaired by a later guess.

`RunFinished` and `RunError` clear the buffer. Distinct runs keep distinct
run ids.

### Helper replies across chats

A helper reply is stored so it can be reviewed with the chat. It is recalled
across chats only when its dependency record is complete: every source and
fact the model actually saw is listed, and storage can expand that list into
a closed set of ancestors. `None` means unknown. A model cannot certify its
own inputs by writing a citation.

This app does **not** enable cross-chat recall of ordinary captured helper
replies whose manifest is unknown. File replies wait for file-grant lineage.
Helper output is not treated as a user-confirmed personal fact. The model
guidance says assistant wording is a previous helper's text, and
`support_changed` marks historical wording whose supporting facts have since
changed.

## A later question

Recall runs when a private turn builds its model request
(`AdkHarness::with_conversation_memory`). There is no separate "does this
question need memory?" classifier. If remembering is on, the reader has about
2 seconds (`RECALL_DEADLINE`). A pause or a recall-revision change during the
read discards the batch.

### Select

`MemorySearch::terms` splits the **current question** on non-letters, drops
words shorter than 2 or longer than 128, drops a fixed list of function
words, and keeps at most 8 terms. A question that is only function words
("What do I?") abstains: no recency fallback, no invented match.

For each remaining term the store runs a lexical recall (`keyword_candidates`,
four reads at a time). A parallel meaning search is started when a semantic
service is attached (`Reader` in `recalling.rs`). That service returns an
empty list, and no notice, when semantic search is not allowed (paused or
blocked), the index setting is missing, the embedding dimension is unset,
or the service is revoked or reconfigured during the search
(`SemanticMemory::search`).
No attached service is the same empty leg with no notice. When the service
does search, it embeds the question and keeps facts at or above cosine
similarity 0.6 (`MEMORY_SIMILARITY_FLOOR`). The embed-and-search call is
capped at 900 ms and must also finish before the recall read deadline. An
error from that call, the 900 ms cap, or the read deadline is the only case
that sets "Search by meaning was unavailable; matching words were still
checked." If word match, source search, or the background read already
failed, that earlier notice stays and this one is not added. Source search
looks for raw episodes whose text matches, even when no fact was extracted.
Those three legs are the hybrid search. This app does not fold earlier chat
turns into the embedding query, and it does not run a title word-similarity
arm.

A background read asks for up to 8 recent **current** facts, ignoring the
question's words. It is skipped when the question sets `valid_at` or
`known_at` (a historical lookup must not be mixed with "true now").

Stopped facts are dropped. Current facts must be valid now: `valid_at` in
the past or unset, and `invalid_at` / `expired_at` unset. A historical
question can accept a fact that was later corrected, then `as_known` hides
the later end dates so the model does not treat the correction as already
known.

### Rank

`fuse_rrf` merges the legs by rank, not by raw score. The constant is
`RRF_K = 60`. A fact's score is the sum, over legs that returned it, of
`1 / (60 + 1 + rank)`, best-first, duplicates inside one leg ignored. The
fused list is cut at 24. There is no importance multiplier and no recency
blend on that score. Ties keep the first leg that introduced the fact
(lexical terms, then semantic, then the order those legs were appended).

The first 6 fused facts that still have a readable user or note source become
the primary hits (`MAX_MEMORY_PRIMARY_ITEMS`). If time remains (at most 200 ms),
related facts are read for up to two entity names taken from those hits,
excluding the name "user", at most 8 candidates and 2 related items. Deferred
fused hits then fill unused item slots. Background hits are attached last.

### Bound

Injection refuses a fact that fails any of these checks (`MemoryContext::push`):

- `stopped_at` is set
- the fact's source id is not this episode
- the episode is not a user message or a note
- the quote is empty or is not an exact substring of the episode
- content or quote is longer than 2048 bytes, or an id is longer than 512
- the fact is already in the context
- the context already has 8 memories (`MAX_MEMORY_CONTEXT_ITEMS`)
- the serialized context would exceed 16 KiB (`MAX_MEMORY_CONTEXT_BYTES`)

A failing item is popped whole. Quotes are never truncated into a different
sentence. Background items share that 16 KiB cap and have their own cap of 8
items and 4 KiB. At most 4 raw source snippets are attached
(`push_source`). A missing source is omitted and the notice says some saved
details were left out because their sources are unavailable.

`evidence::encode` serializes the context for the model and, if the JSON
still exceeds 16 KiB, drops whole background items, then source snippets,
then memories, and adds a notice that only a bounded selection fits. A
multi-search lookup is capped at 4 searches (`MAX_MEMORY_SEARCHES`).

### Inject

The bounded JSON is inserted as a system content block on that model call
only. It is labeled untrusted data, not instructions. The guidance tells the
model to use a statement only when its quote supports the answer, to cite
`memory:fact:…` in square brackets, to keep helper wording distinct from
user statements, and not to claim that a save has finished just because the
user asked. The block is not written into provider session history. The next
turn recalls again. Date-specific lookup, when tools are on, is a separate
bounded read with the same evidence cap.

If policy or storage revision changed between the start and the end of the
read, the model receives an empty context instead of a stale one.

## How a citation opens its source

The model is told to cite the identifier on the injected item. A fact token
is `[memory:` plus the stored fact id plus `]`. Stored fact ids already start
with `fact:`, so the token reads `[memory:fact:…]`. A source token is
`[source:` plus the stored episode id plus `]`, and episode ids start with
`episode:`, so it reads `[source:episode:…]`. `memory_citation_refs` keeps at
most 32 complete tokens in first-appearance order and ignores a repeated id.
An unfinished token, a token whose inner id does not start with `fact:` or
`episode:`, or a token past the cap is not a row. Tokens are navigation
hints. They are not proof, and a made-up id does not grant access.

In the chat transcript, `hide_memory_citation_tokens` removes the raw token
from the visible sentence. The citation card ("Saved details") lists each
recognized reference. Choosing a row builds `MemoryAction::Inspect` for a
fact or `MemoryAction::InspectSource` for a source. `Shell::open_memory_source`
selects the Knowledge tab and `MemoryView::open_source` loads that record
from storage. The store must still resolve the id under the current
permission and withdrawal rules. Knowledge can also open the same source
from the memory list without a citation.

## How withdrawal blocks recall

None of these actions delete the row. They set `stopped_at` (and the
validity end dates) on the facts they cover.

- **Stop using one fact** (`MemoryAction::StopUsing`, `stop_memory`) stops
  that fact only. If a memory job lists the fact, that job's episode is
  marked raw-withdrawn, which also keeps that raw text out of later recall,
  and pending or in-progress jobs on a withdrawn episode are cancelled.
  A `supersedes` neighbor and a `repeats` neighbor stay recallable until
  they are stopped themselves. Other facts from the same source stay.
- **Remove a fact** (`MemoryAction::Remove`, `remove_memory`) walks
  `supersedes` in both directions from that fact, with no depth cutoff, and
  stops every fact in that correction family. It marks the related source
  versions raw-withdrawn and cancels their jobs. The `repeats` edge is not
  part of this walk.
- **Stop a source** (`stop_memory_source`) sets `revoked_at` on that episode
  and its versions, then stops every fact derived from those versions or
  listed on their jobs, and cancels those jobs.

A helper episode is eligible for cross-chat recall only when its dependency
record is complete and every listed source is still private and neither
revoked nor raw-withdrawn, every listed fact still has no `stopped_at`, and
no fact derived from a listed source has been stopped. Stopping one of those
records therefore blocks the helper replies that depend on it. Unknown
manifests never become complete.

After `stopped_at` is set:

- `MemorySearch::accepts` rejects the fact at every `valid_at` and `known_at`,
  including a question about an earlier time
- `MemoryContext::push` and `push_background` reject it
- store recall filters require `stopped_at IS NONE` and the current
  invalid/expired window
- a recall that already started is dropped when the recall revision or
  remembering generation changes before the provider request is sent
- a helper whose dependency closure includes the withdrawn source or fact
  fails that closure check and is not eligible for cross-chat recall

Knowledge can still list a stopped fact or a raw-withdrawn episode when the
reader asks to see stopped records. Stopping the source is the stronger
gate: `revoked_at` covers every version, and every fact taken from those
versions is stopped. Raw text whose episode is revoked or raw-withdrawn is
not recalled.

## Checklist against SovraSRX personal memory

SovraSRX capture (§3), provenance (§4), and recall (§5) are the question
list. This table is the answer for this app.

| SovraSRX question | What this app does |
| --- | --- |
| When does capture fire? | Every private user message on an ordinary chat turn and on a team run, as soon as the command is accepted. Team **replies** are saved as assistant episodes at each visible message end. Not three learning passes, and not a pass over tool evidence. |
| Must the reply wait? | No. The user text is queued. Extraction waits until the interactive turn releases the model slot. |
| What if the process stops mid-turn? | Accepted episodes stay queued and the worker finishes the storage write on shutdown. A model failure leaves the job retryable (3 attempts). A gate outage does not invent an empty fact list and mark the turn done. |
| What is raw evidence? | The episode: user text, or a visible helper reply, with chat, time, author, and provenance. |
| What is a distilled fact? | A model candidate that quotes the user episode exactly, committed by `complete_memory`. |
| What is discarded? | The extractor returns no memories for chatter, questions, secrets, tool output, and the other refusals in the instruction. No fact row is written. The raw user episode is still stored. |
| Source-quote guard? | Yes. Evidence must be an exact substring of the user episode. Invented quotes are dropped before write. |
| How are claims updated? | `replaces` supersedes (old fact kept, validity ended). `same_as` records a repeat and leaves the old fact current. Reconciliation may not change the quoted claim. |
| How does a later question search? | Lexical match on the current question's content words, optional semantic match on facts (cosine floor 0.6), and source-text search. Fused by reciprocal rank (`RRF_K` 60), cut at 24, then bounded as above. |
| What is always in the prompt? | Nothing from memory is pinned on every turn. Background facts are a small, capped set of recent current facts, and only when the question is not historical. |
| How does the user open a receipt? | A `memory:fact:` or source citation, or Knowledge, loads the stored id. The id is a lookup key, not a proof. |
| What does withdrawal do? | Stopping one fact hides that fact. Removing a fact also stops its `supersedes` chain. Stopping a source stops facts taken from it. Helper replies that depend on a stopped record are blocked. `repeats` neighbors are not stopped automatically. Rows remain. |

### Not shipped in this app

These are SovraSRX personal-memory behaviors. They are **not** behavior of
this desktop app, and this document does not describe them as if they were:

- Wiki pages, page slugs, and `[[slug]]` citation chips
- An affect log (valence, label, intensity) captured on each turn
- Captured to-dos or action items with due phrases
- A nightly conversation miner, a retro-miner, or a nightly self-heal pass
- Postgres full-text search, tsvector ranking, or trigram title similarity
- An organisation lens, a persona lens, or a shared "brain of brains"
- A core-identity block compiled into every prompt
- An answer-from-memory gate that skips web search
- Standing-directive pages with an importance floor
- Confidence, importance, volatility, and category scores on each claim
- Connector-read capture (documents, mail, drives) as a separate ingest
- A remember tool that writes a wiki page. A person can still save a note
  from Knowledge; that note is a user-authored episode, not a page
- Early and late learning passes on the assistant's finished answer
- Episode embeddings and freshness-decay ranking of episodes

This app keeps episodes, bi-temporal facts, SurrealDB, and quote-grounded
extraction. It does not replace that model with the SovraSRX wiki.

## Limits in force

- A helper reply is recalled across chats only when its dependency record is
  complete. Unknown manifests stay unknown. Cross-chat recall of ordinary
  captured helper replies is not enabled.
- Personal facts come from user messages and notes. Helper text is evidence
  of what a helper said, not a user preference.
- Whether chatter becomes a fact is decided by the extraction instruction
  plus the quote guard. A candidate the model returns is stored when the
  quote is really in the user text, even if a person would have called the
  sentence unimportant. The instruction tells the model to return no
  memories when nothing qualifies.
- Recall uses the current question's own words for lexical search. It does
  not rewrite the query from earlier turns.
- Injected memory is at most 8 facts and 16 KiB of JSON, with a smaller
  background cap. Further matches stay in storage and can be opened in
  Knowledge.
- Semantic recall is optional and uses a 0.6 cosine floor when it runs.
  Word match still runs. The meaning leg is empty, with no notice, when
  the index is off, the embedding dimension is unset, semantic search is
  paused, the semantic service is revoked or reconfigured during the
  search, or no semantic service is attached. The model is told that
  search by meaning was unavailable
  only when that search returns an error, hits the 900 ms cap, or misses
  the recall read deadline, and only when no other recall notice is
  already set.
- At most 32 citation tokens are recognized in one reply. The store still
  has to resolve each id.
- Quality of what the model chooses to save is not guaranteed by the pipeline
  tests. Measured gaps are recorded in [agent-memory.md](agent-memory.md).

## Where the code lives

| Step | Entry |
| --- | --- |
| Queue a user message without waiting | `RememberingRuntime::record_user` in `crates/sovra-runtime/src/remembering.rs` |
| Hold the reply's model slot | `model_priority::interactive` / `background_ready` in `crates/sovra-runtime/src/model_priority.rs`, taken from the command loop and `spawn_prompt` in `crates/sovra-runtime/src/lib.rs` |
| Extract and commit | `process_chunk` in `remembering.rs`; `extract_memories_in_context` in `crates/sovra-adk/src/memory.rs` |
| Admit or refuse a candidate | `MemoryInput`, `MemoryCandidate`, `parse_memory_candidates` in `crates/sovra-core/src/memory/ingestion.rs` |
| Supersede or repeat | `complete_memory` path in `crates/sovra-store-surreal/src/facts.rs` (`supersedes`, `repeats`) |
| Save a team reply | `TeamMemoryCapture` in `crates/sovra-core/src/memory/team_capture.rs`, persisted from `crates/sovra-runtime/src/memory_capture.rs` |
| Helper dependency manifest | `crates/sovra-core/src/memory/dependencies.rs` |
| Select, rank, bound | `MemorySearch` and `MemoryContext` in `crates/sovra-core/src/memory/recall.rs`; `fuse_rrf` in `crates/sovra-core/src/memory.rs`; `Reader::retrieve` in `crates/sovra-runtime/src/recalling.rs` |
| Inject into the model call | `AdkHarness::with_conversation_memory` in `crates/sovra-adk/src/recall.rs` |
| Open a citation | `Shell::open_memory_source` in `crates/sovra-ui/src/shell.rs`; citation rows in `crates/sovra-ui/src/agui_widgets.rs` |
| Withdraw | `stop_memory` / `stop_memory_source` on `MemoryStore`; closure walk in `crates/sovra-store-surreal` removal |
