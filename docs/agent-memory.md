# Agent memory: research and implementation contract

Research checked 2026-10-27. The seven implementation and local verification steps
below are complete in the working tree, including the file index, file knowledge,
skills and team integrations based on `79d86b4`. This is not a published or signed
release. The final audit below supersedes earlier checkpoint failures; measured
quality and portability limitations remain explicit.

## Capture and recall check (2026-10-29)

A live synthetic check with the selected OpenRouter model,
`anthropic/claude-haiku-5.5`, saved a breakfast preference and a dated residence
from one conversation. A separate conversation recalled jasmine tea and cited the
actual saved fact. Both extraction jobs completed on their first attempt; the
question produced no new facts. The check used a separate local store with 100
synthetic distractor facts and semantic search disabled. The
[retained report](evaluations/memory-2026-10-29/smoke.json) contains synthetic
messages only. This two-turn check establishes that the configured capture and
recall path works for this example, not a general accuracy score.

The UI regression exposed a separate issue: opening Memories before extraction
completed left its list empty until manual refresh. Completed captures now refresh
an idle visible list, or refresh when Memories is opened. A capture during a list
request schedules a follow-up refresh. Editing, source review, unsent search text
and older pages are preserved, with a new-details notice. Headless tests reproduce
the original failure and cover the fix, loading races and preservation of an edit
at minimum/wide window sizes, 200% interface size and both themes. The real-disk
memory lifecycle test also passed, including recall after runtime restart.
The restart check closes the runtime, reopens the same disk store, and sends a
question in a new conversation with no restored context. It verifies that the
provider request contains the saved fact, original quote, source ID and citation
before proceeding to correction and Stop using checks.
Full `cargo xtask ci` passed in the isolated checkout: all gates and 647 tests
passed, with six opt-in tests skipped.

Run the small live check with an existing development connection and a new output
directory (it makes requests to the selected service):

```sh
SOVRA_MEMORY_EVAL_SUITE=smoke \
SOVRA_MEMORY_EVAL_DEV_DATA=/path/to/development/app-data \
SOVRA_MEMORY_EVAL_OUTPUT=/tmp/sovra-memory-smoke-new \
cargo nextest run --workspace --run-ignored ignored-only \
  -E 'test(synthetic_selected_model_evaluation)'
```

## Final acceptance audit (2026-10-27)

`cargo xtask ci` passes **every gate**: formatting, Clippy, layering, styling,
advisories/bans/licenses/sources, unused dependencies and **521 tests**. The two
opt-in external/growth tests remain ignored in ordinary CI; their manually run
measurements are retained below. No lint, dependency boundary, license allow-list
or recall deadline was loosened. `git diff --check` passes. The original `xtask dev`
and Watchexec remain running in the user's existing tmux pane.

| Required step | Implemented behavior and acceptance evidence |
| --- | --- |
| 1. Reliable store | Atomic facts, provenance, replacements and vector generations; real-engine rollback, temporal/entity filtering and disk reopen/migration tests in [store tests](../crates/sovra-store-surreal/tests/store.rs) and its child modules. |
| 2. Provenance and lifecycle | Durable jobs, idempotence, source-linked reuse/correction, retained withdrawal and restart recovery; [real application lifecycle](../crates/sovra-app/tests/memory_lifecycle.rs) and [consolidation tests](../crates/sovra-store-surreal/tests/store/consolidation.rs). |
| 3. Automatic remembering | Selected-model extraction, exact-quote validation, strict/schema-constrained output, bounded jobs, cancellation and visible failures; [ADK extraction tests](../crates/sovra-adk/tests/memory_output.rs), runtime tests and retained live general/temporal observations. |
| 4. Recall in real turns | Scoped keyword/optional semantic retrieval, historical search tool, related context and citations in ordinary/team provider requests; permission, pause, withdrawal, deadline and source checks in [runtime memory tests](../crates/sovra-runtime/tests/models/memory.rs) and child modules. Two live semantic ranking/latency reports document limitations rather than implying perfect relevance. |
| 5. Working context | Bounded persisted roles after restart/model changes, no tool replay, memory-off isolation and ordered shutdown persistence; [application lifecycle](../crates/sovra-app/tests/memory_lifecycle.rs) and [chat persistence tests](../crates/sovra-runtime/tests/models/memory/history.rs). Shared input admission is tested and measured for selected text/image inputs plus duration-aware recording fixtures. |
| 6. Simple controls | Live list/search/history/source/correct/stop, remembering toggle, explicit optional semantic consent and reply source navigation; headless Knowledge/Memory/Shell tests cover both themes, minimum/wide windows and 200% interface size. |
| 7. Verification and packaging | Full CI passes; live synthetic extraction, temporal, context-usage and semantic measurements plus storage growth reports retained. Current unsigned Linux package rebuilt and inspected as below; README and changelog reflect current behavior. |

Residual limitations are evidence, not silently deferred acceptance tests: model
extraction can omit facts, mis-associate dates or fail to split legacy compound
facts; unknown paraphrases may not match withdrawn aliases; related entity reads
are bounded and co-occurrence is not a proven relationship. Semantic scores do not
establish entailment, and the tested remote queries exceeded the short fallback
budget. Media estimates cover only specified formats and are not universal token
bounds. No claim of universal accuracy, state of the art, arbitrary-scale latency,
interactive release validation, signing or cross-platform portability is made.
Detailed observations, reproduction commands and historical checkpoints follow.

## Product contract

Sovra should remember useful things across conversations, explain where a memory
came from, accept corrections, and stop using a memory when asked. A person should
not need to configure databases, vector dimensions, agents, or extraction jobs.
Keep storage and scheduling in the native app binary. The user's configured AI
connection may do model work; no additional database server, Python service, or
required embedding daemon. Local storage does not mean model requests stay local.
Explain that distinction in plain language wherever remembering is enabled.

Memory must support the whole assistant, including ordinary and team conversations,
without treating generated replies, retrieved documents, or tool output as user
instructions. File knowledge stays in its existing versioned, permission-aware
subsystem. Share provider transports and evidence conventions where useful; do not
copy it into a second ungoverned memory bank.

## Research and decisions

| Primary source | Finding relevant to Sovra | Decision |
| --- | --- | --- |
| [Memory in the Age of AI Agents](https://arxiv.org/abs/2512.13564), survey, December 2025 | Separates factual, experiential, and working memory, and formation, evolution, and retrieval. | Keep source episodes, derived facts, conversation context, and explicit skills distinct. A vector index alone is not the feature. |
| [LongMemEval](https://arxiv.org/abs/2410.10813), ICLR 2025 | Tests extraction, multi-session reasoning, temporal reasoning, updates, and abstention; explores decomposed sessions and fact-augmented indexing. | Preserve evidence alongside summaries; test changed facts and absent evidence as well as successful search. |
| [Mem0](https://arxiv.org/abs/2504.19413), April 2025 | Evaluates extraction/consolidation/retrieval and a graph variant against full-history baselines. | Use bounded incremental extraction and retrieval. Vendor benchmark gains are not a guarantee for our model or workload. |
| [Zep temporal graph paper](https://arxiv.org/abs/2501.13956), January 2025, and [Graphiti documentation](https://help.getzep.com/graphiti/getting-started/welcome) | Maintains temporal knowledge and combines semantic, keyword, and graph retrieval. | Retain the embedded temporal store. Make replacement atomic and distinguish when a claim became false from when Sovra learned that. |
| [Hindsight](https://arxiv.org/html/2512.12818v1), December 2025 | Separates evidence and beliefs; uses narrative extraction, multiple retrieval paths, fusion, reranking, and token budgets. | Preserve enough context to resolve people and dates. Bound the context delivered to the reader and attach source references. Do not promote model inference to confirmed user fact. |
| [LongMemEval-V2](https://xiaowu0162.github.io/longmemeval-v2/), 2026 | Evaluates environment state, changes, workflows, gotchas, and premise awareness with accuracy and latency. | Include realistic end-to-end scenarios and retrieval latency; do not claim state of the art from a small retrieval-only test. Existing explicit skills are the procedural-memory boundary. |
| [Hindsight Memory-PRM](https://arxiv.org/abs/2608.29605), August 2026 preprint | Studies operation-level memory utility using retrieval/citation evidence and version chains. | Keep auditable provenance and revisions. Training a memory policy is a research direction, not a prerequisite or an unvalidated default for this desktop release. |
| [MemSecBench](https://arxiv.org/abs/2607.27080), July 2026 preprint, and [MemForm](https://arxiv.org/abs/2610.14431), October 2026 preprint | Examine persistent poisoning, selective repair, evidence/scope manipulation, and consolidation order. | Treat recalled text as evidence, never authority. Test unsupported writes, correction races, disabled memory, and reappearance after stopping use. Prompt filtering alone is not a security boundary. |
| [Anthropic context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents) | Discusses compact context, just-in-time retrieval, and persistent structured notes. | Keep bounded working context and retrieve relevant durable information instead of replaying every saved conversation. |

There is no experimentally established universally best memory architecture. Recent
papers use different readers, judges, datasets, budgets, and definitions of recall.
Use their mechanisms as hypotheses, measure Sovra's actual pipeline, and publish
the evaluation setup and limitations. New preprints above are evidence to consider,
not independent replication or blanket security guarantees.

## Baseline after the pull

- SurrealKV persists episodes, chat threads, facts, team configuration, and artifacts.
- Fact storage has temporal fields and graph edges; hybrid recall is implemented.
- Ordinary chat records user and completed assistant text, gated by memory settings.
- `save_fact` and `recall` still have no production conversation-memory callers.
- ADK conversational sessions are in memory; displaying saved chat is not restoring
  the model's context after a restart.
- File search already has bounded indexing, optional HTTP embeddings, citations,
  permission-aware destinations, reviewed knowledge, and derived summaries.
- The Knowledge page's conversation-memory explanation is still a fixed example.

## Implementation sequence and acceptance evidence

These are completion requirements for the active memory task. Completing the first
step alone does not satisfy it.

1. **Reliable store.** Atomic fact/source/entity/replacement writes; reject stale or
   missing replacement targets; retain temporal history; validate record types and
   vectors; bound queries. Real-engine regression tests must demonstrate rollback,
   corrections, repeat operations, and bounded retrieval.
2. **Provenance and lifecycle.** Recoverable, idempotent ingestion; explicit source
   references; duplicate suppression; scoped ownership; current/history views;
   user corrections and stopping use must survive restarts. Do not erase facts
   under the repository's history-retention rule. UI wording must describe actual
   retention rather than promise physical deletion.
3. **Automatic remembering.** Extract useful supported facts from user-authored
   conversation evidence using the selected model, with validated structured
   output, bounded input/output, owned background jobs, cancellation, retries, and
   visible failure state. Do not require a second model setup for basic memory.
   Distinguish observations from inferred suggestions. Preserve provenance when
   consolidating and do not let external content rewrite preferences or policies.
4. **Recall into real turns.** Retrieve bounded relevant evidence across sessions;
   combine keyword and semantic paths when an embedding destination is available;
   integrate existing file knowledge without bypassing its permissions. Support
   temporal and entity constraints, relevance thresholds, abstention, and explicit
   source citations. Verify the actual ADK request and ordinary/team paths.
5. **Working context.** Restore a bounded conversation context after restart and
   model changes; preserve roles and avoid replaying tool side effects. Memory-off
   disables both new remembering and long-term recall, including queued work.
6. **Simple controls.** A live "What I remember" view with understandable empty,
   loading, failed, and paused states; search; source; correct; stop using; and an
   accessible on/off control. Keep technical settings out of the normal path.
   Use the existing design system and test minimum/wide windows and 200% text.
7. **Verification and packaging.** Real-store tests including reopen/migration;
   deterministic fake-model runtime/ADK tests; privacy and adversarial scenarios;
   repeatable quality/latency evaluation with distractors and growth; single-binary
   dependency review; README/changelog updates; `cargo xtask ci` passes. Report
   actual measured limitations and never label simulated tests as live-model scores.

## Scope and scale

The shipped deployment remains one local user's app data directory, not a shared
multi-tenant network service. Internal scopes should allow safe future separation
of workspaces and sources. Remote authentication, remote multi-user authorization,
and horizontally distributed database operations are not implied by a `ws://`
variant. Design indexed, paginated reads and bounded background work so histories
can grow without loading the whole store or spawning one unowned task per record.
An embedding model/dimension change must never silently mix incompatible vectors.

## Ingestion contract

The `MemoryStore` handle fixes the namespace/database ownership boundary. Ingestion
keys combine the conversation ID with a stable message key; the model cannot choose
the ownership boundary or assign the source episode. Reusing a key with changed
content, role or source time is rejected. User messages and explicit notes may be
queued; assistant text and tool input/output may only be recorded as episodes.

Schema v3 adds a persistent job table and remembering policy. At most 256 inputs
may be pending or processing. A worker claims one input for 120 seconds, with at
most three attempts and a five-second retry delay for reported failures. A policy
revision serializes concurrent queue transitions with pause/resume. Disabling
increments the generation and cancels pending/leased work; enabling does not
revive those jobs. Expired tokens and prior generations cannot commit or fail work.

A batch contains at most eight bounded candidates. Each must quote exact persisted
user evidence; the store does not trust a modified claim snapshot. This establishes
source attribution, **not semantic truth or resistance to every poisoning attack**.
The selected-model extractor validates complete structured responses and rejects
fabricated quotes, unsupported replacements and tool calls. Extracted facts retain
the model and quote, and completed jobs
retain all candidate evidence even when they match an existing fact. The batch,
graph links, corrections and job completion share one transaction. Repeating the
same completion returns its original IDs; changing its payload is rejected.

The extractor can identify a paraphrase with `same_as`, referencing a supplied
current fact. Storage checks the target's current state, entity set and any explicit
validity date in the same transaction. Reuse preserves the original fact and source;
the new job retains its own source quote, interpretation and model. A batch cannot
both reuse and replace the same target. Explicit corrections always create a new
version, including a return to an earlier statement.

Exact matching distinguishes entity sets and explicit validity dates. Schema v6
also retains recognized wordings in an indexed alias table. Repeating one after
withdrawal resolves to the retained stopped fact without reactivating it, including
after disk reopen. These aliases support consolidation, not retrieval. Novel
paraphrases are not guaranteed to match, and model judgments can incorrectly merge
claims. Aliases and completed evidence jobs grow with retained history. Upgrading
v5 preserves expiration separately from withdrawal; absent `same_as` fields retain
the legacy serialized completion fingerprint.

Ordinary prompts, attachment prompts (only their
user-authored text) and team prompts now enter this queue. The only added direct
dependency is the workspace's existing `sha2`, for stable keys; no new service or
process is used.

## Automatic remembering contract

One runtime-owned worker uses the selected chat model and processes one persistent
claim at a time. The isolated request contains only the source message/time and at
most eight current facts. Selection puts related keyword matches before recent
facts: at most eight search terms, 24 results per term, reciprocal-rank fusion
limited to 24 related candidates, then eight recent candidates to fill gaps. The
combined storage lookup has a two-second deadline. No extra model or embedding
request is made for selection. Older facts without lexical overlap can be missed.
It has no tools, provider session history or
personal instructions. The entire input must fit 32 KiB; longer input fails visibly
rather than being silently clipped. Output is limited to 4,096 requested tokens
and 32 KiB of accepted text, with a 90-second deadline. Incomplete responses, tool
calls, fenced/malformed JSON and replacement IDs outside the supplied facts fail.
The provider adapter may buffer a nonstreaming response before application-level
validation; this is not a transport-level byte limit. The request now also obeys
shared model-specific admission described below. Long-message chunking remains unfinished.

When catalog metadata advertises `structured_outputs`, extraction requests a JSON
Schema through the pinned OpenRouter or OpenAI-compatible transport (also used for
Ollama). The schema requires all six candidate fields, rejects extra object fields,
and restricts reuse/replacement references to supplied IDs or null. Nullable references
use a separate branch: live Anthropic endpoints rejected null inside a union-type
enum. Local validation remains authoritative for source quotes, byte/count limits,
references and transactional state. Schema/configuration bytes also count toward the
32 KiB extraction input bound. Ordinary chat requests do not inherit this schema.

OpenRouter extraction sets `provider.require_parameters` so routing requires support
for the requested format, and omits temperature unless the model advertises it.
Provider enforcement still varies, as described in the
[OpenRouter structured-output documentation](https://openrouter.ai/docs/guides/features/structured-outputs).
Unknown capability metadata, the pinned native Anthropic adapter (which does not
map ADK's response schema), and legacy/custom harnesses retain prompt-based JSON
with the same strict local parser. A rejected schema request fails visibly through
the existing bounded retry policy; it does not silently weaken validation.

Extraction instructions explicitly include useful directly stated facts about named
colleagues, relationships and projects, and require the subject in the statement and
entity labels. Existing records are not rewritten to normalize old entity labels.
Independently changeable attributes should be separate candidates. For older compound
facts, a correction must preserve unchanged details explicitly reaffirmed by the new
source, without copying unsupported claims into a newly attributed record. This is
model guidance, not a deterministic semantic decomposition or migration of old facts.

The existing remembering setting starts the worker only after preferences load.
Switching off immediately stops accepting new input, aborts model work and revokes
queued/leased jobs in storage. A pause/resume sequence cannot revive old work, even
when configuration updates coalesce. Changing the selected model cancels its work
for retry; publishing unchanged model state does not cancel it. Shutdown owns and
cancels the worker and assistant-history taps. Raw assistant episodes also check
the persisted policy transactionally, so they cannot be written after a pause.
Chat history is separate and remains available when remembering is off.

Knowledge shows live loading, paused, waiting-for-model, working and failure states.
It explains that selected AI services receive messages and earlier memories, while
saved records remain local. The illustrative story is still separate from real
saved records. Saved details can now inform answers through bounded keyword recall.
Exact quote checks
establish attribution, not semantic entailment; no live-model poisoning-resistance
or quality score is claimed.

## Conversation recall contract

Ordinary, attachment and team turns bind a per-turn reader to the original user
prompt. The model adapter requests evidence immediately before each provider call,
including delegated and team-member calls. It adds that evidence only to the
outgoing request, outside ADK's persisted conversation events. Provider response
chaining is disabled for these calls so a continuation cannot silently retain
previously revoked evidence. Ordinary assistant/user history still remains; this
does not erase information already mentioned in a reply or sent to a provider.

The current lexical path searches up to eight distinct terms, removes a small
English function-word list, and uses the store's stemmed full-text index. Matching
legs are fused by RRF; rank is not confidence. There is no recent-facts fallback
for unmatched questions. At most 24 primary candidates are inspected and eight memories
included, with a shared 16 KiB serialized evidence limit and a two-second retrieval
deadline. Each accepted item contains a persisted user/note source, an exact quote,
conversation/source/fact identifiers, temporal fields and an interpretation label.
Unsupported and oversized items are rejected rather than clipped. Lookup failure
is represented as unavailable evidence, not invented personal history.

The first six supported direct matches can seed one non-recursive context lookup.
At most two distinct named entities are queried, each returning at most eight
candidates; the generic `user` label cannot seed expansion. Original date and
explicit entity restrictions remain in force. Up to two additional facts must
share a selected entity and name another entity, preferring facts from the same
source as a direct match. Co-occurrence supplies context, not proof of a relationship.
Quotes still require persisted user/note evidence. Unused item slots are filled
from remaining direct matches, within the same eight-item/16 KiB total.

These optional queries and source reads share a 200 ms allowance inside the total
two-second deadline, leaving 100 ms for the final policy check when time permits.
Optional errors/timeouts preserve direct matches. No extra model call is made.
Pause and review changes revoke the entire in-flight context. Untyped entity labels,
the eight-candidate window, and the shared byte limit can still omit useful older
relationships or admit irrelevant co-occurrences; this is not complete graph recall.

The reader checks the current setting and policy generation before and after IO.
A pause epoch permanently revokes readers created before that pause, including
after rapid resume. Current recall excludes stopped, superseded and future-dated
claims. Explicit valid-time, known-time and entity filters exist in the domain and
store. Tool-capable models now choose these filters through bounded lookups;
see the historical-query contract below. Existing permission-aware file tools remain separate from
conversation evidence. Completed ordinary and team replies now offer **Saved source**
buttons for complete `[memory:fact:…]` references, in first-appearance order with
duplicates removed and at most 32 controls per reply. References remain in the
original reply text and saved chat; streaming text is not rewritten. Each button
opens the existing explicit-review protocol for the exact fact ID, displaying its
persisted source and current/previous/stopped status. Returning preserves the chat.
Missing or invalid IDs show a lookup error rather than leaving an earlier source
visible. No model request is needed, and review remains available while remembering
is paused. Recognizing a model-authored reference does not establish that its claim
is supported, or that the reference was supplied by recall. Broader live-model
answer/citation accuracy remains unproven; retained samples do not establish it.

## Optional semantic recall

Schema v4 adds a separate conversation-embedding selection, disabled by default.
The Knowledge chooser discloses the exact destination and sends an explicit command;
folder permissions never authorize conversation work. Existing Ollama, OpenRouter
and compatible HTTP transports validate the selected connection again before each request. No new
server, download, dependency or required model setup is introduced for basic memory.
Saved details and questions leave the device only when the chosen endpoint is remote.

Each selection, including a retry, advances a generation and owns a distinct vector
table. The first valid batch pins its dimension. Old generations remain stored but
are not queried; repeated reconfiguration therefore grows disk use. This preserves
history and avoids silently mixing embeddings; compaction is not yet implemented.
Legacy fact vectors remain unchanged. Reopen reapplies the active schema and retains
selection, dimension, vectors and the remembering policy.

An owned worker visits keyset pages of at most eight current or retained facts and embeds only
missing supported user/note evidence that has not been withdrawn, using the shared
transport's text limits. This includes versions superseded before embeddings were enabled.
Pages continue every 250 ms, with a two-second delay after a complete traversal;
there is no full-store load, but an idle traversal still does work proportional to
saved history. Each batch has a 30-second deadline. A failed generation stops model
retries until explicitly selected again, avoiding repeated paid failures. Pausing,
review edits and selection changes cancel in-flight work. Each configuration request
owns a revocation token, so an earlier save cannot unblock a later queued disable.
Storage additionally fences commits on both selection and remembering generations.

Recall allows 900 ms for query embeddings within the existing two-second total.
Timeouts or provider failures preserve keyword evidence and disclose the fallback
to the reader. HNSW considers at most 96 neighbors, with temporal/entity predicates,
then admits at most 24 candidates above a provisional cosine floor of 0.6 before
RRF and evidence checks. This number is not a calibrated confidence or a guarantee
across models/languages. Deterministic orthogonal-vector tests establish filtering
and integration, not live-model answer quality. The live calibration below records
model-dependent relevance and remote latency limitations; the later growth and
temporal evaluations cover their separately described scopes.

## Live semantic calibration (2026-10-27)

The opt-in standard-library diagnostic `tests/evaluations/semantic_relevance.py`
uses a read-only development credential and an explicitly named OpenRouter embedding
model. It never reads personal chats/memory or changes app settings. Both retained
reports contain 116 synthetic facts (100 inventory distractors), 24 pre-labelled
queries, exact normalized cosine rankings, request usage and elapsed time. These are
transport/ranking measurements, not real-store HNSW, lexical fusion, extraction,
reader-answer or entailment scores. Facts use batches of eight; questions use single
requests without prefixes, matching the application's text representation.

```sh
python3 tests/evaluations/semantic_relevance.py \
  --dev-data /path/to/development/app-data \
  --output /tmp/fresh-semantic-evaluation \
  --model openai/text-embedding-3-small
```

| Measurement | [Text Embedding 3 Small](evaluations/memory-2026-10-27/semantic-small.json) | [Qwen3 Embedding 8B](evaluations/memory-2026-10-27/semantic-qwen.json) |
| --- | --- | --- |
| Vector dimension | 1,536 | 4,096 |
| Relevant labelled facts admitted at 0.6 | 8 / 20 | 17 / 20 |
| All candidates admitted at 0.6 | 13 | 28 |
| Unanswerable queries with no candidates at 0.6 | 4 / 6 | 1 / 6 |
| Relevant facts admitted at 0.3 | 20 / 20 | 20 / 20 |
| All candidates admitted at 0.3 | 46 | 515 |
| Query latency minimum / median / maximum | 915 / 1,141.5 / 2,524 ms | 1,157 / 2,217 / 11,346 ms |
| Query requests within 900 ms | 0 / 24 | 0 / 24 |
| Provider-reported total cost | 0.00003352 | 0.00002485 |

One fact can legitimately provide context without answering the question; the
precision-like counts above use strict direct-support labels. For example, the
current city cannot establish a previous city, and a tea preference cannot explain
its reason. Both models rank these related but insufficient facts highly. Lowering
the global threshold would recover paraphrases while admitting more unsupported
matches, so **0.6 remains a provisional candidate filter**, not a truth/confidence
score. No single tested cutoff separates all positives and negatives. Temporal and
source checks plus reader abstention remain necessary. The synthetic set is small,
handwritten and not a held-out benchmark; do not claim general model superiority.

All diagnostic query calls exceeded the app's 900 ms embedding allowance. This
Python diagnostic opens an HTTPS connection for each request; it does not measure
Rust scheduling, connection behavior, storage or the full turn. It nevertheless
shows that these remote routes may frequently fall back to matching words. The
bounded latency contract remains unchanged; local endpoints or different network
conditions may behave differently. No model was downloaded or selected in the app.

OpenRouter support shares the compatible embedding request/response implementation,
uses only models explicitly advertising embeddings, and keeps per-destination
consent and revalidation. The existing catalog already requests all output
modalities. References: [embedding API](https://openrouter.ai/docs/api/api-reference/embeddings/create-embeddings)
and [model catalog](https://openrouter.ai/docs/api/api-reference/models/get-models).
The new failing-first capability regression and real-HTTP test cover selection,
authentication, response reordering/normalization and changed-endpoint rejection.

## Current Linux package inspection (2026-10-27)

The final code, including related context, source navigation, temporal guidance,
recording admission and OpenRouter embeddings, was rebuilt with `cargo xtask dist`
using the isolated pinned Velopack tool already described below. Output:

- `target/dist/linux-x64-2839962/SovraHarness.AppImage`, **74,897,912 bytes**.
- AppImage SHA-256: `251206a71083b6868b77f350b7890674732c876203941b20198661220fc9180a`.
- Extracted Sovra executable SHA-256: `604f5ba7b6f607c2a0ea3b3fcc291cba06575198fba25ea33629b3c97d00f551`.

The `.nupkg` contains an identical AppImage. SquashFS extraction at offset 944,632
shows Sovra, the Velopack `UpdateNix` helper, version/launcher metadata and icons;
no database, development credential, model daemon, Python/.NET runtime or synthetic
evaluation fixture is packaged. The extracted executable's
`--veloapp-install 0.1.0` bootstrap exits successfully before application startup.
No competing GUI or watcher was launched. Dynamic dependencies remain XCB,
xkbcommon/X11, libgcc, libm, libc and the ELF loader; **GLIBC_2.43** is required.
This is a current local unsigned package, not proof of interactive release startup,
older-Linux portability, macOS/Windows behavior, signing or notarization.

## License clarification (2026-10-27)

The repeated CI failure was traced to inconsistent metadata in the published
`finl_unicode 1.5.0` archive. Its manifest declares
`(MIT OR Apache-2.0) AND Unicode-DFS-2016`, while `LICENSE-UNICODE` is the complete
**UNICODE LICENSE V3** text. That file matches the [SPDX Unicode-3.0 text](https://spdx.org/licenses/Unicode-3.0.html)
exactly after whitespace normalization. The other two license files retain the
MIT/Apache choice. The registry archive SHA-256 matches Cargo.lock:
`80bb028c8b4148c9ee0cca68fcd9add6044e81d3619f48577ddf13a263d047a2`;
all three inspected files match the corresponding archive members byte for byte.

`deny.toml` now clarifies **only this exact version** as
`(MIT OR Apache-2.0) AND Unicode-3.0`, with hashes of all three license files.
This corrects the metadata mismatch using the [cargo-deny clarification mechanism](https://embarkstudios.github.io/cargo-deny/checks/licenses/cfg.html#clarify);
it does not expand the already permitted licenses, waive the Unicode obligation,
change the confidence threshold or exclude a dependency. An upstream version or
license-text change requires renewed review. A temporary configuration with an
incorrect Unicode-file hash was also checked: it rejects the clarification and
fails the license gate, as intended. SHA-256 fingerprints of raw files:

| File | SHA-256 |
| --- | --- |
| LICENSE-MIT | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| LICENSE-APACHE | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| LICENSE-UNICODE | `361d7912957842f2ce61c774b14afba41ae91df84882982e6f7e749e01c07c91` |

## Historical query contract

A tool-capable parent, specialist or team helper receives `search_saved_details`.
It selects a focused query and optional exact entity, valid date and known date.
The adapter parses YYYY-MM-DD as midnight UTC and RFC 3339 with an explicit offset;
invalid/ambiguous strings are rejected. The runtime passes the reference time.
Instructions require a historical lookup before treating current evidence as past
state, and clarification when the question does not establish a precise date or
person. This is model-directed planning, not a deterministic natural-language
parser or a guarantee that a model chooses the correct date. Models without tool
support receive an explicit limitation instead of an advertised unusable tool.

At most four query attempts are accepted per turn, shared with delegates. A tool
response contains only a query receipt, never facts or quotes. On the next provider
request the adapter resolves the latest receipt for that invocation through the
same permission-fenced reader and injects fresh evidence outside session storage.
Receipts from earlier turns cannot select a new turn's date. Invalid or exhausted
lookups supply unavailable evidence, not a fallback to a contradictory current
claim. The existing two-second retrieval and evidence-size limits still apply to
each request. Models may compare multiple dates through successive calls; citations
and intermediate assistant text remain part of that turn's own conversation.

Schema v5 separates an explicit `stopped_at` withdrawal from temporal invalidation.
Withdrawn versions remain browsable but are excluded from lexical/vector retrieval,
embeddings and evidence admission for every date, including earlier known times.
Stop-using preserves previous validity timestamps and is idempotent. It withdraws
the selected version; other versions and the original chat remain available.
Before v5, explicit stops were indistinguishable from standalone expirations;
migration conservatively marks expired facts without a superseding successor as
withdrawn, while preserving correction history. No record is deleted. The new
read-only date parsing uses the workspace's existing `time` and `serde` dependencies
in the ADK adapter; core's dependency allowlist and the architecture remain unchanged.

## Working conversation context

The UI already hydrates saved chat threads. Every ordinary/attachment dispatch now
includes a pure projection of that visible history before the current user message:
at most 24 text messages, 24 KiB total text and 8 KiB per message. Roles and bounded
helper names are retained. Widgets, tool calls, attachment bytes and reasoning are
excluded. Oversized messages are shortened on UTF-8 boundaries with an explicit
marker; omitted older history is disclosed to the model. Construction borrows the
original text until the budget is known rather than cloning entire old messages.

The ADK adapter creates a fresh per-turn session and injects the excerpt at the
provider boundary. Live tool results within that turn still work; prior provider
history and response chaining cannot duplicate the restored excerpt or replay old
actions. Team history is bounded before dispatch and again for member prompts.
The current team question is passed separately and is not clipped by history
limits; older callers that included it as the final history line are normalized
to avoid duplication. A regression test reproduced the previous omission of the
question when an API caller supplied empty team history.
These limits apply to earlier conversation text. The assembled provider request
also passes shared admission below; the current user request is never silently
truncated. The existing chat-thread storage still retains full visible history.

Chat saves, soft deletes and reloads share one ordered runtime-owned worker. A
reload observes earlier accepted writes; a delayed older snapshot cannot overwrite
a newer one or race a subsequent deletion. Graceful runtime shutdown drains this
worker before emitting `Stopped`. The queue follows the existing Client's unbounded
acceptance policy so busy storage does not discard accepted edits or block Ping;
queue growth under sustained slow storage remains unmeasured. Storage failures are
logged as before, and forced process termination cannot promise a completed flush.
After owned workers finish and their handles drop, the executor remains alive for
up to five seconds for adapter cleanup tasks. The pinned embedded database starts
its asynchronous shutdown when its final SDK handle drops; immediately destroying
the executor cancelled that cleanup and left the directory locked. `Stopped` now
follows executor teardown. If tasks exceed the grace period, the runtime logs their
remaining count and proceeds; this is not an unlimited flush guarantee.
The native quit hook requests `Shutdown` without blocking the active UI thread.
After GPUI's event loop exits, the composition root calls `shutdown_blocking` once
and waits on a separate completion channel. That channel is signalled after
executor teardown, so the UI's event consumer cannot steal a shutdown acknowledgement.
The pinned GPUI quit-handler window is only 200 ms; storage flushing therefore does
not depend on fitting inside that window. Forced OS termination still bypasses it.

Same-chat context is independent of cross-conversation remembering. Switching
remembering off prevents long-term recall and new remembering, while an opened
chat can still refer to its own visible messages. Runtime clients that omit the
optional context keep their existing in-process session behavior; native UI turns
always provide an explicit snapshot, including an empty snapshot for a fresh chat.

## Shared input admission

Every ADK model call passes an innermost provider-boundary guard, after recalled
memory and restored conversation have been injected. Selected-model catalog limits
supply the context window and maximum output. Unknown windows use a 65,536-unit
fallback; the application caps the window at 1,048,576 units. Output defaults to
4,096 tokens, is capped by the advertised output limit and one quarter of the
window, and leaves a further 1,024 units for protocol overhead.

Text, role/framing overhead, tool definitions, live tool arguments/results and
provider configuration all count. Text uses UTF-8 bytes as conservative admission
units, not an exact tokenizer. Tool schemas are counted explicitly because ADK's
request serializer skips them. Binary attachments stay intact and use provisional
minimum reservations per item: 8,192 for images, 32,768 for audio, 65,536 for video.
Locally readable recording durations increase these to at least 64 units/second
for audio and 512 units/second for video, rounding upward. These are policy margins
above the documented static Gemini rates, not provider-specific tokenizers. Unknown
media is charged the application cap and cannot fit alongside a normal request.
These media reservations are not universal upper bounds; dimensions, duration,
provider tokenization and unreported/server limits can differ. The policy may reject
requests that a tokenizer could fit. Calibrating false rejections and missed overflow
against real model/media workloads remains required; these are not token-count claims.

Private middleware metadata identifies injected context by position, never by
matching text in the user's question. Admission removes oldest history until two
messages remain, then lowest-ranked memories until two remain, then the remaining
history and memories if necessary. Whole evidence records are removed; quotes are
never sliced into different claims. The model receives an omission notice, preserving
other retrieval notices. Saved UI history is unchanged. Mandatory instructions,
current input, selected media and live tool exchanges are not cut. If those cannot
fit, a clear error is emitted before the chat model receives the request. Memory
extraction reports this same failure; it does not secretly send an oversized input.

The guard strips its private metadata before calling a provider, preserves other
provider options and sets the output cap on the actual request. Server response
chaining is disabled and opaque cached-content requests are refused because their
hidden context cannot be measured. Full explicit request history still travels in
ordinary turns. The same guard is inherited by helpers and team members. Narrower
initial file excerpts and provider-specific token counting remain opportunities to
reduce conservative rejections; this guard does not claim to solve those yet.

## Validation log

Latest 2026-10-27 verification: **519 workspace tests passed**, with the two opt-in
evaluations skipped by default. Both evaluations were also run explicitly; live
collection completed with the quality failures below, and disk-growth assertions
passed. `cargo xtask ci` passes formatting, Clippy, layering and styling, then fails:

```text
error[rejected]: failed to satisfy license requirements
finl_unicode v1.5.0: Unicode-DFS-2016 is not explicitly allowed
  comrak v0.56.0 → xberg v1.3.10 → sovra-files
advisories ok, bans ok, licenses FAILED, sources ok
```

The pinned `xberg` dependency on `comrak` and its dependency on `finl_unicode` are
unconditional; disabling a feature does not remove this chain. License policy was
not relaxed. The [official registry metadata](https://crates.io/api/v1/crates/finl_unicode)
was checked on 2026-10-27: 1.5.0 is still the latest release and retains the rejected
license expression; 1.3.0 and 1.4.0 do too. No dependency downgrade was used to avoid
the policy. `cargo shear` and `git diff --check` pass. The existing dev watcher
rebuilt the app and remains running. Both package checkpoints recorded below
predate related-context lookup, citation navigation and recording-duration admission.
This is not a full-task or release-completion claim.

### Opt-in live-model evaluation

The ignored debug-only composition test
`live::synthetic_selected_model_evaluation` uses the selected development model
connection with synthetic data in a fresh directory. It reads that connection's
development credential through a read-only adapter, copies public connection/catalog
metadata, and starts from default preferences without the user's standing context.
It does not open the app's memory database or saved chats, modify credentials/settings,
or launch another desktop app. It seeds 100 unrelated inventory facts in the test
database, uses lexical recall with semantic search off, and now runs 15 independent
conversation turns plus a withdrawal question when a preference was extracted.
Set `SOVRA_MEMORY_EVAL_SUITE=temporal` to run the alternative eight-turn date suite
with the same isolation, selected model and distractors. It covers residence
transitions, two independent dates, an unrelated undated preference, a tentative
future move and historical answers. The default remains `general`; unknown suite
names reject before starting model work.
Historical lookup can make additional model calls; extraction can retry three times.
The run uses the configured service and can incur its normal charges.
Two consecutive terminal extraction failures stop further evaluation calls and
retain the partial report. Individual semantic misses still require manual review.

```sh
SOVRA_MEMORY_EVAL_DEV_DATA=/path/to/development/app-data \
SOVRA_MEMORY_EVAL_OUTPUT=/tmp/sovra-memory-evaluation-new \
cargo nextest run -p sovra-app --test memory_lifecycle --run-ignored only \
  -E 'test(synthetic_selected_model_evaluation)'
```

Use a fresh output directory outside the checkout. `report.json` retains each
synthetic prompt, expected behavior for manual review, full answer, elapsed answer
and extraction times, terminal extraction state, retries, interpreted candidates,
and resulting fact versions. The isolated database remains available for inspection.
A passing test means observations were collected, **not that semantic quality
passed**. Failed extraction is a reported observation, not a reason to hide later
cases. A transport/runtime failure retains the partial report. Offline CI skips
this test and never reads credentials.

2026-10-27, preliminary runs using the selected
`anthropic/claude-haiku-5.5` through OpenRouter:

- Run A collected seven cases before the original evaluation driver stopped on an
  extraction validation failure. It correctly recalled the drink, reused its
  paraphrase, linked the first residence correction, found the historical residence,
  and ignored the quoted fictional instruction. Its answers also incorrectly
  suggested that no write tool meant memory could not be updated.
- Run B allowed bounded retries and collected all eleven original cases. The drink,
  current residence, historical residence, unknown-composer abstention and final
  answer were correct on manual review. However, the first residence change did
  **not** link its predecessor, leaving the earlier residence current, and Mira's
  preference failed extraction after three attempts. The paraphrase needed two
  attempts. Correct answers therefore did not establish a correct stored history.
- Run C, after the guidance changes, collected all twelve cases. Both residence
  changes linked their predecessors with the correct explicit dates; the paraphrase
  reused the original preference. Drink/current/historical/latest answers used
  matching saved fact citations, the composer question abstained, the fictional
  instruction produced no fact, and the withdrawal question did not reveal jasmine
  tea. The historical question's extraction retried once after malformed structured
  output, then correctly returned no facts. **Mira's preference was omitted**:
  extraction completed with an empty candidate list. This remains a quality failure.
  Some acknowledgements still add unnecessary persistence disclaimers; the prompt
  change is not a guarantee of accurate self-description on every turn.

Run C's twelve answer latencies had a median of **2.983 seconds**, range
**1.731–4.694 seconds**. Answer plus completed extraction had a median of **3.315
seconds**; the historical question took 8.065 seconds including retry. These are
whole local-runtime/network/model timings on this host, not isolated retrieval
times, controlled comparative results, or a production SLA. Usage/cost was not
measured. Raw synthetic reports are retained as
[run A](evaluations/memory-2026-10-27/live-a.json),
[run B](evaluations/memory-2026-10-27/live-b.json), and
[run C](evaluations/memory-2026-10-27/live-c.json), including the failures.

The reader now describes the separate background saving process without promising
that a save completed. Extraction guidance explicitly distinguishes changed values,
same-person paraphrases, different people and questions. Validation failures retain
the violated rule without including model output or source text; a failing-then-fixed
regression checks this boundary. These changes require further evaluation rather
than erasing the earlier failures. This small, single-model synthetic sample is not
a benchmark score, a poisoning-resistance claim, relevance calibration, an embedding
evaluation, or a storage-growth/latency result.

### Structured-output and partial-update follow-up

2026-10-27: the new pinned-transport regression first failed because an advertised
schema was absent from the HTTP request. It now checks OpenRouter, compatible and
Ollama transports with and without capability metadata, empty and populated
reference sets, preservation across harness rebuilding, and ordinary chat isolation.
Existing invalid-output tests also run with schema mode enabled; provider formatting
does not bypass local validation.

- [Run D](evaluations/memory-2026-10-27/live-d.json) was interrupted after five
  consecutive extraction jobs failed. A focused synthetic request exposed HTTP 400
  for a nullable enum; separating its null branch returned HTTP 200. The
  [diagnostic result](evaluations/memory-2026-10-27/schema-diagnostic.json) omits
  credentials and provider request IDs. The evaluator now stops after two consecutive
  terminal extraction failures.
- [Run E](evaluations/memory-2026-10-27/live-e.json) completed sixteen cases, all
  extraction jobs on their first attempt. Mira's preference was retained and answered
  separately from the user's; both residence changes linked correctly. However, a
  combined Cedar fact containing monthly frequency and Friday delivery was replaced
  with only Tuesday delivery. The answer correctly abstained about frequency, exposing
  lost current knowledge rather than hiding the loss with a plausible answer.
- [Run F](evaluations/memory-2026-10-27/live-f.json), after atomic-attribute guidance,
  also completed sixteen cases without extraction retries. Cedar's monthly frequency
  and delivery day were separate facts; correction replaced Friday with Tuesday and
  reused the monthly fact with its new source evidence. The answer cited both. Mira's
  preference, the user's preference, residence history, unknown-composer abstention,
  fictional-instruction exclusion and withdrawal behaved as expected in this sample.
  Routine saving disclaimers were reduced. The withdrawal answer still speculated
  about whether the user might be Mira: bounded retrieval supplied Mira's preference
  but not the separate colleague relationship. This remains a retrieval/context
  limitation, even though the withdrawn preference was not exposed.

Run F's answer median was **3.050 seconds**, range **1.823–5.021 seconds**; median
answer-plus-extraction time was **3.510 seconds**. These are observations from the
same configured model and host, not a controlled comparison or a universal quality
score. Splitting claims, choosing correction targets and interpreting quotes still
depend on the model; earlier compound records are not automatically repaired.

### Related-context follow-up

The provider-request regression first failed because Mira's preference was supplied
without the separate colleague relationship. It now passes, alongside a real-store
composition test verifying that the relationship disappears after withdrawal.
Held-read tests check the 200 ms fallback and pause/withdrawal revocation. Invalid
primary quotes cannot trigger an entity lookup; unrelated questions do not expand.
Pure tests check entity/date scope, source preference, deduplication and limits.

[Run G](evaluations/memory-2026-10-27/live-g.json) repeated the sixteen-case live
suite after this change, with all extraction jobs completing on their first attempt.
The withdrawal answer correctly identified Mira as the user's colleague and abstained
about the user's preference, without speculating that the user might be Mira.
Separate preferences, current residence, the 2024 historical lookup, fictional-text
exclusion, unknown-composer abstention and the monthly/Tuesday project answer were
supported in this sample. Answer median was **2.592 seconds**, range **1.665–4.899
seconds**; median answer-plus-extraction time was **3.494 seconds**. This is not a
controlled timing comparison or a general accuracy score.

Run G also exposed an unresolved temporal extraction issue: the return-to-Porto
message dated the move as 2026-02-01, but its separate current-home candidate omitted
that date, so storage used the message time. Current answers were correct; validity
between the move and message dates is not established correctly for that state fact.
Routine background-saving disclaimers also recurred. Related context does not fix
model extraction errors, and no new temporal-quality guarantee is claimed.

### Temporal extraction follow-up

The extraction instructions and schema description now explicitly use the full
source to date each claim. A dated transition gives its resulting state the same
applicable date even when a separate event is also emitted; unrelated claims keep
their own dates or null. A tentative plan must not supersede a current state.
The application still treats these as model interpretations, not deterministic
natural-language date parsing, and leaves the exact-source and storage checks intact.

The focused eight-case suite ran on the same selected Claude Haiku 5.5 connection
with semantic search off and 100 synthetic distractors. Both
[temporal A (baseline)](evaluations/memory-2026-10-27/temporal-a.json) and
[temporal B (revised guidance)](evaluations/memory-2026-10-27/temporal-b.json)
completed all extraction jobs on their first attempt. Residence versions carried
2024-01-01, 2025-03-01, 2026-02-01 and 2026-04-01 as appropriate, with linked
supersessions. Newsletter delivery and Mira joining kept separate April/June dates;
monthly frequency and the unrelated tea preference had null candidate dates. The
tentative Oslo move did not become a confirmed residence or alter Lyon's date.
Historical answers returned Porto for March 2026 and Lyon for May 2026.

Both runs combined the move event and resulting residence into one fact; therefore
they did not reproduce run G's separate-state omission. This is additional coverage,
not evidence that the intermittent omission is eliminated or that the prompt change
caused a measurable accuracy gain. Both also combined Mira's colleague relationship
and joining event, leaving their independent temporal meaning unresolved. Baseline
answer median was 3.675 seconds (2.645–6.365), revised median 3.411 seconds
(2.300–6.134); combined answer/extraction medians were 5.712 and 5.397 seconds.
These single runs are not a controlled performance comparison or a benchmark score.
The revised full offline suite passed 509 tests; CI still fails only at the license
gate recorded above. No provider, dependency or persistent schema changed.

### Citation navigation validation

The headless regression first failed because cited replies had no source control.
The styling suite now restores saved ordinary and named-helper replies and clicks
through their references in both themes at 640×420,
2560×1440 and 640×420 with 200% interface size. It checks that controls remain
reachable by scrolling, a stopped record retains its original source without
offering correction/reactivation, returning preserves the reply, and a failed
subsequent lookup removes the earlier source. Pure tests cover malformed/incomplete
references, Unicode, repeated IDs and the control bound. The real-store lifecycle
test resolves an actual recall citation through the same inspection command and
compares its original episode. These verify navigation and storage identity, not
whether a model's cited sentence is semantically supported. No dependencies or
network services were added.

### Context usage calibration

On 2026-10-27, the selected `anthropic/claude-haiku-5.5` connection completed nine
direct, non-streaming OpenRouter requests using synthetic data, a 32-token output
cap, no tools/history and no memory database. OpenRouter documents that its response
usage uses the model's native tokenizer ([usage accounting](https://openrouter.ai/docs/cookbook/administration/usage-accounting)).
The [raw report](evaluations/memory-2026-10-27/context-usage.json) retains synthetic
inputs, image dimensions/hashes, counts and timings, without credentials or request
IDs. The [standard-library Python diagnostic](../tests/evaluations/context_usage.py)
reads only the active official OpenRouter development connection and credential,
requires a fresh output directory, stops on a failed request, and changes no settings.
It is development tooling, not an application or package dependency. To repeat:

```sh
python3 tests/evaluations/context_usage.py \
  --dev-data /path/to/development/app-data \
  --output /tmp/sovra-context-usage-new
```

| Synthetic input | Text bytes + current image allowances | Reported total prompt tokens |
| --- | ---: | ---: |
| Short control | 20 | 17 |
| English prose | 15,220 | 6,816 |
| Arabic/Japanese/accented text | 18,820 | 7,416 |
| Rust/JSON text | 16,020 | 8,016 |
| 200×200 PNG | 8,212 | 84 |
| 1000×1000 PNG | 8,212 | 1,316 |
| 2048×2048 PNG | 8,212 | 4,781 |
| 2048×512 PNG | 8,212 | 1,426 |
| Eight 2048×2048 PNGs | 65,556 | 38,129 |

The middle column excludes the additional configuration and framing units charged
by the real guard. An offline ADK regression reconstructs these text/media parts
and verifies that its full estimate covers the recorded native counts. The current
estimator reads image MIME type only; changing to dimension-based estimates will
require the generated PNGs as inputs too. These are one-model samples, not a proof
for every tokenizer, image mode, tool schema or server-side context. The generous
image allowance can reject requests that a precise tokenizer would accept. No
production allowances were lowered based on these samples. Anthropic's current
[vision documentation](https://platform.claude.com/docs/en/build-with-claude/vision)
also describes resolution-dependent token limits; those limits are provider-specific.

Audio/video remain unmeasured on a live accepting model. A locally generated
600-second H.264 MP4 is only **19,083 bytes**, well below the 20 MiB attachment limit.
Its [measured duration and reproduction command](evaluations/memory-2026-10-27/media-duration.json)
show that byte bounds cannot establish a duration bound. At Google's documented
static video rate of 263 tokens/second, it would require about **157,800 tokens**,
exceeding the current 65,536-per-video reservation; documented audio use is also
duration-dependent at 32 tokens/second ([token documentation](https://ai.google.dev/gemini-api/docs/tokens)).
This is a calculation from published rates, not an observed Gemini request or
rejection. The subsequent change below adds duration-aware admission for specific
containers; broader provider/media validation remains required.
All 510 offline tests pass; the unchanged license gate still prevents full CI success.

### Recording-duration admission follow-up

The regression first demonstrated that a ten-minute video with a known duration
passed the old fixed allowance for a 131,072-unit window. It now reserves 307,200
units and fails before provider IO at that window; a sufficiently large window
still admits the original bytes. Core policy uses the larger of the original floor
and the duration-based reserve, with saturating arithmetic. No short-recording
allowance was lowered. Provider processing modes can still exceed these estimates.

The file adapter reads timing fields from PCM RIFF/WAV and ordinary MP4/QuickTime
containers within the existing 20 MiB read bound. WAV checks data length, sample
rate, alignment and PCM format. Movie inspection has a shared 4,096-box bound,
supports 32/64-bit box sizes and version 0/1 timing, and takes the largest movie,
track-header or media-header duration using each applicable timescale. Fragmented
movies, invalid timing and unsupported codecs/containers return unknown. Relevant
format definitions are [Microsoft WAVEFORMATEX](https://learn.microsoft.com/en-us/windows/win32/api/mmeapi/ns-mmeapi-waveformatex)
and [Apple movie timescales](https://developer.apple.com/documentation/quicktime-file-format/movie_header_atom/time_scale).
This reads container hints, not decoded packet timelines: inconsistent or forged
metadata, unsupported edit semantics and codec validity are not fully validated.

Hints are derived again on load, so no attachment manifest migration is required.
Unknown duration adds a reader note on import or reopen without replacing other
notes. Original media bytes stay unchanged. Private ADK annotations carry hints
through request assembly, including inline tool media, and are removed immediately
before provider IO while preserving other annotations. Remote file references and
inline media without a hint retain the prior fallback. This is partial format
coverage, not a universal media-token bound or a live audio/video quality result.

Tests cover the actual 19,083-byte ten-minute movie, storage reopen, legacy unknown
timing, malformed/truncated inputs, zero scales, extended boxes, version-one
timescales, fragmented movies, complexity limits, PCM validation, arithmetic bounds,
attachment-to-request propagation, tool media, provider-call rejection and removal
of private hints. The fixture is compiled only into tests; no new dependency,
external process, server or model setup was added to the app.
The full workspace run passed 519 tests; the subsequent legacy-note adjustment
passed all nine selected duration/recording checks. CI passes formatting, Clippy,
layering and styling before the unchanged license failure above.

### Storage growth measurements

The opt-in `growth::disk_growth_measurements` test writes a fresh SurrealKV store
with 100, 1,000 and 5,000 synthetic facts. All discuss breakfast; one in ten shares
the target's tea term, so they are lexical distractors. At each size it samples
20 keyword searches, exact-entity searches and first-page reads through the public
port, checking result bounds, target inclusion and entity isolation. Facts lack
source episodes and vectors in this storage-only workload; model calls, source
lookups, end-to-end runtime deadlines and embeddings are not included.

```sh
SOVRA_MEMORY_GROWTH_OUTPUT=/tmp/sovra-memory-growth-new \
cargo nextest run -p sovra-store-surreal --test store --run-ignored only \
  -E 'test(disk_growth_measurements)'
```

2026-10-27 local run, milliseconds shown as median / nearest-rank p95:

| Stored facts | Keyword search | Keyword + exact entity | First 20 matching details |
| --- | --- | --- | --- |
| 100 | 0.461 / 0.536 | 1.039 / 1.092 | 1.527 / 1.591 |
| 1,000 | 0.894 / 1.069 | 5.752 / 9.257 | 10.827 / 12.421 |
| 5,000 | 1.497 / 1.723 | 27.057 / 29.126 | 24.418 / 25.711 |

All bounds/inclusion assertions passed. Broad entity and management reads visibly
grow with this corpus; these warm local samples do not establish constant-time
behavior or performance beyond 5,000 facts. The [raw microsecond samples](evaluations/memory-2026-10-27/growth.json)
also retain incremental write times. Normal CI skips this disk workload.

### Local Linux packaging review

2026-10-27: `cargo xtask dist` initially stopped because `vpk` was missing. Installed
the build-only .NET 8.0.427 SDK and pinned Velopack CLI 1.2.161 under
`/tmp/sovra-packaging-tools`, following the [Microsoft installation script documentation](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-install-script)
and [Velopack CLI reference](https://docs.velopack.io/reference/cli/content/vpk-linux).
No global install or new application dependency was added. This checkpoint build used:

```sh
DOTNET_ROOT=/tmp/sovra-packaging-tools/dotnet \
DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 \
SOVRA_VPK=/tmp/sovra-packaging-tools/bin/vpk cargo xtask dist
```

It produced `target/dist/linux-x64-2030180/SovraHarness.AppImage` (74,869,240 bytes),
SHA-256 `0cbaca22d9f6173de7dcf78a86f65ce6ffe491093d0f7c06b02f459cd0f0d489`.
This earlier artifact predates schema v6 and consolidation; the refreshed package
below supersedes it for the current worktree.
The extracted Sovra executable matches the fresh staging binary byte-for-byte;
the update package contains the same AppImage. The image contains Sovra,
Velopack's `UpdateNix` helper, launcher/version metadata and icons. There are no
model/database servers, .NET runtime, development credentials, source checkout or
user database in the package. The extracted executable's pinned Velopack
`--veloapp-install 0.1.0` bootstrap exits successfully before application startup.

This local executable's dynamic dependency table names XCB, xkbcommon/X11, libgcc,
libm, libc and the ELF loader. It requires **GLIBC_2.43**, so this build is not a
portability claim for older Linux distributions; runtime graphics-driver loading
also goes beyond that static table. No interactive release app was launched beside
the persistent dev watcher. Signing, other-platform installers, interactive release
startup and live-model quality remain unverified. The full CI license gate still
fails as recorded below; no release was published.

Refreshed on 2026-10-27 after schema-output and partial-update work:
`cargo xtask dist` succeeded with the same isolated tools, producing
`target/dist/linux-x64-2359371/SovraHarness.AppImage` (**74,861,048 bytes**), SHA-256
`73d3a3445c8c1b0bee91fa4e3409b402f76b6ea37ade0a47e6aaa1876f2d4ac8`.
The extracted executable matches both release and staging binaries, SHA-256
`0f4235702996fd9a298d7cdc0de4348f32bd9218741ee773f6e436c9e8b30367`, and the update
package contains the identical AppImage. Inspection found only Sovra, `UpdateNix`,
launcher/version metadata and icons; evaluation databases/reports, credentials,
model/database servers and the build-only .NET runtime are absent. The extracted
installer bootstrap exits zero before app startup. Its listed dynamic libraries
remain XCB, xkbcommon/X11, libgcc, libm, libc and the ELF loader, with a GLIBC_2.43
requirement. This is a local unsigned checkpoint, not an older-Linux portability
claim or a published release. The persistent dev watcher was not interrupted.
This package predates related-context lookup, citation navigation and
recording-duration admission documented above.

### Consolidation validation

2026-10-27: `cargo nextest run --workspace` passed **496 tests**, none skipped.
A focused core/store/ADK/runtime/app run passed 53 tests. Two regressions first
failed against the previous implementation: returning to an earlier statement
reused an obsolete version, and identical wording collapsed different entities.
They now pass, along with explicit-date separation, stale or incompatible reuse
rollback, original provenance retention, idempotent replay, v5 migration, and
recognized-alias withdrawal across a real disk reopen.

The composition test runs the real store and runtime against a deterministic HTTP
model. Ten newer distractors push the relevant fact beyond the recent-eight window;
keyword selection still supplies it for consolidation. After stopping use, that
fact is absent from the extractor context, while local alias matching suppresses
the repeated wording. These fixtures establish pipeline behavior, not live-model
semantic accuracy, poisoning resistance or a quality score.

`cargo xtask ci` passes formatting, Clippy, layering and styling, then fails on the
existing `finl_unicode 1.5.0` / `Unicode-DFS-2016` license restriction. No policy
exception was added. Live-model evaluation and the full seven-step task remain open.

### Combined store/runtime lifecycle validation

2026-10-27: `cargo nextest run --workspace` passed **487 tests**, none skipped.
The new composition-level test runs the real runtime and on-disk store against a
deterministic loopback HTTP model. It verifies extracted fact/source/model identity,
cross-chat quotes and citation IDs in the actual provider request, correction
replacement, withdrawal exclusion, and a second runtime opening the same database
with history, source notes, withdrawal and paused settings retained. Saved chat
messages still reach their own conversation with remembering off, and no new
extraction requests are sent. No private data or live-model service is used.

The restart first failed because the database directory remained locked after
`Stopped`. Inspection of the pinned SDK's session shutdown and a failing rerun
confirmed that executor destruction cancelled asynchronous adapter cleanup. The
bounded cleanup period fixes the real-disk regression. HTTP fixtures are shared
between runtime and composition tests; the only added development dependency is
the workspace's existing `serde_json`. Production dependency boundaries are unchanged.

A headless GPUI regression also first failed because quitting never sent `Shutdown`.
It now observes the command, and the disk lifecycle test uses the same blocking
completion API as the composition root. All existing minimum/wide/200% layout
checks pass; the quit wiring changes no layout or styling.

`cargo xtask ci` passes fmt, Clippy, layering and styling, then fails the existing
`finl_unicode 1.5.0` / `Unicode-DFS-2016` license gate. `cargo shear` and
`git diff --check` pass. This test establishes pipeline behavior and persistence,
not live-model answer quality, relevance calibration or crash durability.

### Ordered chat persistence validation

2026-10-27: two new runtime regression tests failed before the fix: a newer snapshot
started while an older save was held, and shutdown reported `Stopped` while accepted
writes were still blocked. They now pass through the real `Client`, including Ping
responsiveness, save/load/delete ordering, retained soft-deleted data and shutdown
draining. `cargo nextest run --workspace` passed **485 tests**, none skipped.

`cargo xtask ci` passes fmt, Clippy, layering and styling and still fails the existing
`finl_unicode 1.5.0` / `Unicode-DFS-2016` license restriction. `cargo shear` and
`git diff --check` pass. The existing dev supervisor rebuilt the application and
remains running. This establishes ordering under held IO, not crash durability,
bounded chat-queue growth or a successful complete release gate.

### Shared input admission validation

2026-10-27: `cargo nextest run --workspace` passed **483 tests**, none skipped.
Offline tests cover whole-record trimming, preserved questions and live tool
exchanges, explicit tool-schema accounting, Unicode, media reservations, private
metadata removal, provider-call rejection and visible remembering failures. A
real HTTP fixture advertises an 8,192-token window and verifies the outgoing
`max_completion_tokens` cap, omitted older context, retained evidence and rejection
of an oversized current question before a second chat request. These checks prove
the implemented estimate and transport wiring, not provider-exact token counts.

`cargo xtask ci` passes fmt, Clippy, layering and styling, then fails at the existing
`finl_unicode 1.5.0` / `Unicode-DFS-2016` license gate. `cargo shear` and
`git diff --check` pass. The existing `xtask dev`/Watchexec supervisor remains live
and has rebuilt the native app with memory configured. Calibration and the other
remaining full-task requirements below were still outstanding at that checkpoint;
the final acceptance audit records their later verification.

### Explicit review and lifecycle

The Knowledge page now uses `MemoryCommand`/`MemoryEvent::Review` for current/history
pages, literal case-insensitive search, exact source inspection, corrections and
stop-using. Pages contain at most 50 facts (20 by default), ordered by creation time
and record ID with a keyset cursor; inserts ahead of the cursor do not repeat rows.
The existing creation-time index supports the boundary; literal substring search
may still scan matching candidates and is not a constant-time search claim.

User corrections create an immutable note, a replacement fact, provenance and
supersession in one transaction. Stale targets, including stopped facts, reject
the whole edit without leaving an orphan note. Explicit edits are allowed while
automatic remembering is paused and do not call a model. Stop-using retains history;
it does not erase the original chat or remove that chat's own visible context.
The runtime serializes a bounded queue of review actions and drains accepted edits
before reporting shutdown. The pure view reducer ignores obsolete request IDs.
Committed corrections and stop-using actions revoke outstanding recall readers
before reporting success, without cancelling unrelated extraction jobs. This
cannot retract evidence in a model request already sent before the change.

### Optional semantic recall validation

2026-10-27: `cargo nextest run --workspace --no-fail-fast` passed **465 tests**, none
skipped. New real-engine checks cover consent generations, incompatible dimensions,
competing first batches, transaction rollback, temporal/entity filtering, v3 migration
and disk reopen. Real runtime HTTP fixtures verify explicit consent, evidence without
keyword overlap, changed-destination rejection, pause/disable cancellation, bounded
failure retries, keyword fallback under a stalled embedding service and the actual
model request. These fixtures do not measure live-model answer quality.

Two regression tests first failed and then passed: a queued disable could be undone
by an earlier configuration completing, and a late configuration acknowledgement
could erase newer worker progress. Consent controls and destination labels pass
headless layout checks in both themes at 640×420, 2560×1440 and 200% interface size.

`cargo xtask ci` passes fmt, Clippy, dependency layering and styling, then fails the
existing license policy check: `finl_unicode 1.5.0` requires `Unicode-DFS-2016`, reached
through `comrak 0.56.0 → xberg 1.3.10 → sovra-files`. The policy was not relaxed.
`cargo shear` and `git diff --check` pass. The existing watcher rebuilt and launched
the native app with memory configured. The full seven-step task remains in progress.

### Historical recall and withdrawal validation

2026-10-27: `cargo nextest run --workspace --no-fail-fast` passed **474 tests**, none
skipped. A real HTTP fixture drove ordinary and team calls through date/entity
lookup, then inspected the actual provider request for original evidence/citations.
Offline ADK tests cover mid-turn revocation, per-turn receipt isolation, invalid
receipts, query quotas, date offsets/fractions, and models without tools. These
are deterministic integration checks, not a measured natural-language-planning score.

Real-engine tests verify schema-v4 withdrawal migration, preservation of correction
history, idempotent withdrawal, historical lexical/vector exclusion and disk reopen.
A new backfill regression first failed because only current facts were embedded;
the worker now visits retained versions too, while withdrawn versions are excluded
from inputs and transactionally rejected on commit. Existing minimum/wide/200%
headless UI checks pass with the more precise consent and stop-using descriptions.

`cargo xtask ci` passes fmt, Clippy, layering and styling; `deny` still fails on the
unchanged `finl_unicode 1.5.0` / `Unicode-DFS-2016` policy restriction. `cargo shear`
and `git diff --check` pass. The existing watcher remains live and rebuilt the native
app with memory configured. Later entries above record context admission,
consolidation, checkpoint packaging and the limited live-model observations.

### Runs

- 2026-10-27: `cargo test -p sovra-core -p sovra-store-surreal` passed: 119 core
  tests, 8 store unit tests, and 21 store integration tests. These use deterministic
  inputs and the real embedded engine; they are not live-model quality scores.
- The store now commits facts, entities, provenance, embedding-model pinning and
  corrections together. Regression tests cover complete rollback after a missing
  source, competing corrections, stale/missing targets, record-table validation,
  repeated invalidation, distinct Unicode/entity names, vector validation and
  bounded recall/episode reads. Recall exposes source and replacement IDs; exact
  source reads preserve the recorded role and conversation.
- A real SurrealKV disk-reopen test replaces the previous misleading test that
  opened two unrelated empty in-memory stores. It verifies source episodes,
  correction links, retained invalidated facts, and embedding-model identity
  across two reopens. Engine migration tests cover adoption of one legacy model,
  rejection of mixed legacy models, incompatible index configurations, and newer
  schema versions without changing their metadata.
- Full `cargo xtask ci`: formatting, Clippy, layering and styling pass after
  fixing constant placement and diagnostic assertions. The license gate fails:
  `finl_unicode 1.5.0` requires `Unicode-DFS-2016`, which is not in the approved
  license list (`comrak → xberg → sovra-files`). That storage-foundation stage did
  not change manifests, lockfile or license policy. `cargo shear` passes.
- `cargo nextest run --workspace --no-tests pass` passes separately: **393 tests
  passed, 0 skipped**, including the disk-reopen regression and the minimum/wide/
  double-size UI checks. The full CI gate remains failed at the license check.
- Ingestion follow-up (2026-10-27): **406 workspace tests passed, 0 skipped**.
  New real-engine coverage includes immutable/idempotent input keys, source-role
  validation, whole-batch rollback, fabricated quote rejection, persisted model
  attribution, duplicate evidence retention, abandoned-lease recovery, competing
  claimers, bounded retries/backoff, queue overflow without orphan episodes, pause/
  resume generation revocation, v2 migration, and disk reopen with paused policy
  and separate database ownership. `cargo xtask ci` again passes fmt/Clippy/layering/
  styling and fails at the same unapproved `finl_unicode` license. `cargo shear`
  passes. This is storage verification; no automatic extraction, live-model quality
  or runtime cancellation claim is implied.
- The existing dev watcher remains in `1:SovraHarnessRun` (pane `%6`), supervised
  by `xtask dev` and Watchexec. Its app build and launch now succeed; runtime logs
  confirm `memory_configured=true`. Two sidebar API/field mismatches in the
  concurrent UI work were corrected without changing the watcher setup.
- Automatic-remembering follow-up (2026-10-27): **422 workspace tests passed,
  0 skipped**. Offline ADK tests verify request isolation, abstention, output bounds,
  malformed/incomplete/tool response rejection and exact-quote/replacement checks.
  Runtime tests drive the real `Client` with a deterministic HTTP model and an
  observable memory port: selected-model extraction, responsive Ping during model
  work, pause cancellation, model-change cancellation, disabled startup and visible
  unsupported-output failures. Real-engine coverage rejects late raw episode writes
  after pausing. Headless Knowledge tests cover both themes, minimum/wide windows
  and 200% interface size. These do not establish live-model extraction quality or
  a combined real-store/provider runtime result. A failing concurrent skills-library
  test exposed shrinking scroll content; preserving its natural height fixed it.
  `cargo xtask ci` passes fmt, Clippy, layering and styling, then fails at the same
  `finl_unicode` license gate above. `cargo shear` and `git diff --check` pass.
- Recall follow-up (2026-10-27): **430 workspace tests passed, 0 skipped** before
  the concurrent Connections integration changed the shared tree. New tests inspect
  actual ordinary/team HTTP requests for cited cross-conversation evidence, pause
  while retrieval is held, reject unrelated/expired/future/assistant evidence, and
  verify in ADK that injected evidence does not persist across turns or provider
  continuations. A real-store temporal regression first failed when a later
  correction rewrote earlier known-time results; it passes after the fix. These
  tests use deterministic model outputs, not measured live-model answer quality.
  A further attachment-path test has been added; its rerun currently cannot build
  because the concurrent Connections integration was missing `WorkspaceState`
  initialization and exhaustive event arms. Those core integration gaps were then
  resolved; the latest `cargo xtask ci` fails on 16 Connections-related Clippy
  diagnostics (including unchecked integer casts and documentation formatting),
  before reaching the previously reported license gate.
  Memory test assertion lints have been fixed; `cargo shear` and `git diff --check`
  pass. No current whole-worktree completion claim is made.
- Working context and review follow-up (2026-10-27): **449 workspace tests passed,
  0 skipped**. Tests cover restored text/roles after runtime restart and model
  changes with remembering off, attachment recall without learning file contents,
  bounded team history with the complete current question, atomic manual
  corrections and stale-edit rollback, keyset pages including equal timestamps,
  review while paused, shutdown draining accepted edits, and revocation of a held
  recall after stopping use. The real disk test verifies retained corrections,
  their source notes and stopped status after reopening. Headless controls run at
  640×420, 2560×1440 and 200% text size, with both themes checked. Two asynchronous
  tests now synchronize on the actual worker events instead of assuming ordering;
  page tests scroll to controls when preceding content grows.
  `cargo xtask ci` passes formatting, Clippy, layering and styling, then fails at
  the unchanged `finl_unicode 1.5.0` / `Unicode-DFS-2016` license gate.
  `cargo shear` and `git diff --check` pass. The existing dev watcher rebuilt and
  launched the app with `memory_configured=true`; it remains running. These are
  deterministic protocol/storage and headless UI checks, not live-model quality
  scores or a release-completion claim.
- The final acceptance audit above supersedes this earlier outstanding-work list.
  Calibration reports and the current package inspection have since been added,
  and the verified license-file clarification resolves the CI failure without
  expanding allowed licenses. Broader model accuracy, novel-paraphrase handling,
  media coverage and release portability remain the measured limitations described
  in that audit; the full seven-step implementation/local-verification scope is complete.
