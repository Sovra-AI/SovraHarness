# Agent memory: research and implementation contract

Dated audit log and measured limits for memory. The current capture and recall
contract is [memory capture and recall](memory-capture-and-recall.md). This file
is the history of investigations, not the day-to-day guide.

## Runtime input tracking — 2026-11-03

New private ordinary and team replies record their actual visible conversation
inputs and every returned memory lookup, including automatic background facts.
The provider adapter observes requests and streamed output; external tools, media,
opaque server history/cache and native server continuations invalidate complete
tracking. A model cannot certify access by writing a citation or a permission claim.
Older history without durable source links also stays unknown. File replies are
saved for review but await file-grant lineage before recall eligibility.

User admission returns a receipt only after storage. The receipt links the exact
visible user message; helper receipts match exact wording, message identity where
available, and the originating user turn. These links persist even if the user
has switched chats. A receipt cannot overwrite a link or update a chat activity
date. Context truncation preserves source references and explicit omission
notices; the current raw user text is kept separately, including team commands.
Schema 11 adds optional chat source/time fields without rewriting old chat JSON.
Schema 12 separates raw wording withdrawn with a detail from whole-source
revocation, preserving independently grounded sibling facts. Older revocations
remain in force; the migration does not infer a narrower historical permission.
No legacy authorship is inferred and historical backfill remains opt-in/unimplemented.

Native verification found an inconsistency that the first clean-database fixtures
missed: fact provenance loading still returned legacy sources with unknown grants,
so a new reply's complete runtime manifest failed storage certification. A failing
real-engine regression now covers that path. Batched direct/background/related
fact evidence uses the same explicit private-user scope, source version, pause
and source-withdrawal checks. Unsupported adapters fail closed. Unknown records
remain reviewable; omitted supporting records produce a bounded model notice.
Fact withdrawal still preserves separately grounded sibling facts from the same
source; raw full-source recall retains its stronger whole-record withdrawal gate.
Diagnostics record only manifest counts and eligibility booleans, never content.

Capture joins the user-admission receipt under a two-second bound at completed
output; missing receipts leave helper dependencies unknown. It never waits for
fact extraction. Team captures accumulate saved preceding replies, and capture
failures invalidate complete tracking for later replies. The shared bounded
manifest may conservatively include inputs read by another member during that
turn. Recall continues to recheck the immutable closure in storage.

`cargo xtask ci` passes **991 tests, 10 skipped**, including real runtime/disk
ordinary and team recall after restart, transitive withdrawal, automatic-fact
dependencies, unknown-history exclusion, late receipt persistence, additive
schema-10 migration, provider input/output tracking and minimum/wide/200% layout
checks. The new receipt-link regression failed before implementation. Native MCP
and release verification for this integration are still in progress. Earlier
benchmark results below describe their named storage revisions; no new runtime
performance or general answer-quality result is claimed.

## Helper dependency storage — 2026-11-03

Helper recall requires a complete record of the inputs exposed to the model,
not merely citations chosen by its answer. The source admission contract now
accepts a runtime-certified dependency manifest. `None` means unknown;
`Some(empty)` explicitly certifies no saved inputs. User statements cannot supply
helper manifests. Sources and fact IDs share a 128-item bound and are canonicalized
for idempotent admission. A source version's manifest is immutable.

Inside the admission transaction, the adapter expands existing immutable helper
closures and fact provenance into a flattened closure. Missing inputs, unknown
permissions, untracked parents, self-reference or closure overflow prevent recall
eligibility. Original wording is still saved for review; dependencies are never
silently truncated to grant access. Retrieval checks every retained ancestor and
fact before limiting results. Withdrawing an original record, a helper record or
a supporting fact therefore blocks transitive paraphrases without hiding unrelated
replies. Corrected support stays available as explicitly labeled historical
helper wording; the flag respects requested validity and knowledge dates.
Model guidance distinguishes helper output from user statements and commitments.

Schema 10 additively marks missing dependency metadata unknown. Existing source
and fact IDs, citations and provenance stay unchanged. Runtime output capture
still supplies unknown manifests: collecting all recalled evidence, visible
history and tool/file inputs is the next integration step. This storage change
does not yet enable cross-chat recall of ordinary captured helper replies. The
existing README limitation remains accurate, and no automatic backfill is added.

Verification: the initial recall regression failed before implementation. Final
`cargo xtask ci` passes **980 tests, 10 skipped**, including dependency withdrawal,
correction dates, missing/unknown inputs, manifest immutability, the 128-item
closure boundary, additive migration and real-disk reopening. The pinned-engine
source-query plan regression continues to use the full-text index. The release
build passes in 3m 19s and produces one 221,478,552-byte ELF executable, retaining
the same X11/keyboard and C-runtime shared-library requirements. Installer packaging
and a clean-machine launch were not repeated.

The isolated synthetic runtime rerun is recorded in
[`runtime-dependency-storage.json`](evaluations/memory-recall-2026-11-03/runtime-dependency-storage.json).
It retrieved all **576/576** expected evidence items and left no pending or active
jobs. At 100k sources, p95 dispatch-to-provider was **43.66 / 52.96 / 122.24 ms**
at **1 / 4 / 16** concurrent requests, versus **42.17 / 56.09 / 123 ms** in the
earlier dispatch report. These single-host synthetic measurements do not establish
a speedup. The suite uses an unoptimized build, deterministic local HTTP responses,
no semantic service and zero provider tokens/cost. It measures the existing user-
evidence workload with the additional source-query gate, not a helper-heavy corpus.
The run preceded the final historical-label predicate fix; that predicate is not
exercised by this current-only user corpus. A first attempt was deliberately stopped
before completing any stage because it overlapped compilation; it is not included.
Query counts, transaction conflicts and oldest queue age remain uninstrumented.

After a manual development launch of the final temporal fix, Development MCP
submitted the tagged "Copper birch recall check" question about the user's saved
monthly income. The configured Claude Haiku 5.5 model through OpenRouter returned
the saved amount, quoted two original statements and exposed source links. Opening
a cited detail showed its original wording and review controls. The two test
records were withdrawn through confirmations, their filtered list became empty,
the test chat was deleted and the earlier income conversation restored. The native
renderer still cannot produce MCP screenshots; the answer was checked with a
local OS capture outside the repository. This verifies continued user-memory
recall and source navigation after migration, not completed runtime tracking or
cross-chat recall of helper output.

## Replies in background conversations — 2026-11-03

A regression reproduced lost streamed text and queued follow-ups when switching
chats. The workspace now parks each conversation's live messages, generation
state, queue, team attribution and pending model-setup request. Session-scoped
events fold into their existing, non-deleted conversation. Live buffers are
swapped rather than cloned on each chunk. A queued follow-up becomes a typed
command with its originating session and bounded context before returning to the
visible pane; a background completion cannot redirect it to the selected chat.

The shell saves the session named by each completed event, including generated
titles and team events. Navigation first synchronizes the departing transcript,
then saves it; Stop also saves its partial snapshot. Returning to a conversation
restores its live controls and can resume its original model-setup request.
Deleting a conversation drops its parked queue and late events do not recreate it.
This does not add cancellation of the runtime turn when deleting a background chat.

Pure and headless regressions cover concurrent ordinary/team streams, interleaved
helper attribution, queued command context, partial saves, setup and deletion.
A real runtime with deterministic streaming HTTP and embedded disk storage verifies
both ordinary and team replies completed elsewhere survive database reopening.
Unknown sessions are ignored; this does not import connected-channel conversations.
Helper cross-chat recall, connected-source provenance and optional backfill remain
separate unfinished work.

`cargo xtask ci` passes **971 tests, 10 skipped**, including minimum/wide/200%
memory UI coverage. Through Development MCP, a fictional "Blue orchid background
check" was sent with the configured Claude Haiku 5.5 model through OpenRouter.
Generation was observed before switching to a clean new chat. Returning after
completion showed all 45 requested sentences and the exact ending "BLUE ORCHID
COMPLETE". The transcript and ending survived a manual application restart;
original-source review also contained both the prompt and helper reply. Both
records were withdrawn through the real confirmations, the filtered list became
empty, the test chat was deleted and the income conversation was restored.
The renderer still cannot capture MCP screenshots; locally inspected OS captures
remain outside the repository. This is a conversation-persistence check, not an
answer-quality or cross-chat helper-recall benchmark.

The release build passes in 3m 52s and produces one 221,445,664-byte native ELF
executable. Its direct shared-library requirements remain X11/keyboard and the
normal C runtime; this increment adds no service or language runtime. Installer
packaging and clean-machine launch were not repeated.

## Team reply capture — 2026-11-03

A real-runtime/disk-store regression reproduced a visible team answer disappearing
from original-source review after reopening. Team AG-UI output now uses the same
owned capture task and admission port as ordinary assistant replies. A pure,
bounded reducer assembles exact visible text by message ID, retaining the run ID,
member ID, helper name and message-start time. Distinct turns have distinct run IDs
even within one clock tick. Source review displays the recorded helper's name;
the Skills entry explains that these records are available. Anonymous visible
AG-UI summaries remain anonymous assistant output. Activity cards, hidden reasoning
and moderation controls are not relabeled as source text.

The event channel holds at most 64 events; each source buffer is capped at the
existing 128 KiB admission limit, with at most eight active team messages. Oversize
replies report a save failure instead of silently saving a truncated source. Empty
or interrupted unfinished messages do not produce complete-source receipts.
Capture checks the in-process pause epoch and supplies its original persisted
policy generation to atomic admission, so pause/resume cannot revive an older
capture even if its database operation starts after resumption. This adds no stored
field or schema migration. Admission still uses the shared policy transaction fence.

Regression coverage includes interleaved Unicode replies, duplicate end events,
missing attribution, anonymous summaries, oversize/empty/interrupted buffers,
two distinct turns, disk reopen, withdrawal, and pause/resume while normal and team
responses are held mid-stream. The fake HTTP model makes no live inference calls.
Helper output remains excluded from personal extraction and cross-chat recall
until dependency-aware withdrawal is complete. Connected-message provenance,
file/artifact source references, backfill and the broader evaluation remain open.

Native restart verification found a second gap: the source survived but the chat
transcript lost the team reply. The shell saved ordinary completion events only.
A regression first failed on the missing save command; AG-UI message end, run end
and run error now request a conversation snapshot after the reducer folds them.
This preserves completed members and the final activity card without requiring
navigation to another chat. The subsequent background-conversation audit and
verification are recorded above.

Final `cargo xtask ci` passes **965 tests, 10 skipped**, including named-source
review in both themes at minimum/wide/200% sizes. The native app was driven through
Development MCP with fictional single-helper exchanges. "Silver fern team check"
verified attributed source persistence and exposed the transcript failure; after
the history fix, "Ivory moss history check" retained Priya's reply and the completed
team card across a manual restart without navigating away before shutdown. Both
sources from each exchange were withdrawn through the real confirmation controls,
the filtered lists became empty, both test chats were deleted, and the earlier
conversation was restored. Pending and failed-check counts were zero. MCP screenshot
capture remains unsupported by this renderer; OS captures were inspected locally
and kept outside the repository. These checks do not establish cross-chat recall
of helper output, which remains intentionally gated as described above.

The final release build passes in 3m 29s. The output is one 221,415,008-byte native
ELF executable with the same normal X11/keyboard/C-runtime shared libraries; no
new dependency, service or language runtime was introduced. Packaging and a
clean-machine launch were not repeated for this increment.

## Background dispatch — measured increment, 2026-11-03

A readiness regression with paused Tokio time reproduced a 250 ms wait before
starting the second independent extraction despite free slots. The worker now
re-enters its biased select loop after each successful claim, so queued capture,
configuration changes and completed jobs are handled before another claim. New
evidence, a completed job and the foreground activity watch all wake dispatch.
An empty claim or unavailable shared permit clears immediate dispatch; the 250 ms
recovery poll still covers retry dates, expired leases and permits held by another
worker. No slot limit, lease, consolidation order or interactive priority is removed.
Virtual-clock coverage includes filling four slots, holding the fifth until a slot
is released, foreground completion, new evidence and avoiding empty-queue spin.
Tokio's test clock is enabled only through the runtime's development dependency.

The first faster-dispatch benchmark failed during the 100k stage while persisting
an extraction result. Its partial report is retained as `runtime-dispatch-contention.json`;
the original backend error was not logged, so shared-fence contention is an inference,
not a confirmed diagnosis of that individual failure. Runtime logging now preserves
the backend cause. Independent claim/stage/finish/fail operations unnecessarily
wrote the global policy record; a new real-engine regression caught that shared
write. A second test opens a native transaction, reads a processing job, commits a
pause or source withdrawal through the real port, and proves the stale job write
cannot commit. Existing jobs therefore use their own cancellation row as the fence.
Admission and explicit retry keep their policy fences; fact consolidation still
checks and writes the fact revision. This does not yet remove every shared admission
write required by the broader plan.

Full CI after these changes passes **954 tests, 10 skipped**, including the real
embedded-store four-lease overlap test and minimum/wide/200% memory UI checks.
The final isolated runtime benchmark passes **576/576 expected evidence reads**;
every stage ends with zero pending and active jobs. Full reports are
[`runtime-before-dispatch.json`](evaluations/memory-recall-2026-11-03/runtime-before-dispatch.json),
[`runtime-dispatch-contention.json`](evaluations/memory-recall-2026-11-03/runtime-dispatch-contention.json)
and [`runtime-after-dispatch.json`](evaluations/memory-recall-2026-11-03/runtime-after-dispatch.json).

Each cell below is seconds to dispatch 64 requests and drain their background
checks, at concurrency **1 / 4 / 16**. This includes a zero-cost deterministic
HTTP fixture, not real model inference. Database setup/reopening is outside these
stage durations. Same machine, unoptimized test build, four extraction slots,
semantic search off, native app kept open; no build or CI was run alongside either
completed measurement. This is one before/after experiment, not a statistical
estimate of production speedup.

| Sources | Before (seconds) | After (seconds) | After retrieval p95 (ms) |
| --- | --- | --- | --- |
| 1,000 | 16.00 / 16.01 / 16.01 | 1.68 / 0.78 / 0.42 | 17.1 / 30.7 / 40.0 |
| 10,000 | 16.01 / 16.02 / 16.02 | 2.48 / 1.19 / 0.68 | 21.0 / 32.3 / 51.4 |
| 100,000 | 16.35 / 16.12 / 16.13 | 10.27 / 4.28 / 3.04 | 42.2 / 56.1 / 123.0 |

Retrieval latency did not improve uniformly; for example, 100k/four-request p95
was 52.0 ms before and 56.1 ms after. The measured improvement is queue-drain time.
The fixture still does not instrument SQL query counts, all transaction conflicts,
oldest queue age or real provider tokens/cost. Actual model quality and complete
cross-source memory coverage are not established by these measurements.

The optimized release build passes (3m40s), producing one 221,356,080-byte native
ELF executable with the Linux system-library dependencies listed below. No new
runtime service or language is introduced; installer/clean-machine validation was
not repeated. After a manual `cargo xtask dev` reload, stdio MCP verified a fictional
Amber Kettle exchange: source wording and helper attribution were correct, background
checking finished, personal-detail count stayed four, and waiting/failure counts were
zero. Both source records were withdrawn through confirmation controls; the search
became empty, the temporary chat was deleted and the earlier earnings conversation
restored. MCP screenshot capture returned **Screenshot capture is unavailable on this
platform renderer**. An OS window capture was inspected instead and kept outside
the repository; minimum/wide/200% checks passed headlessly.

The native capture also exposed a separate chronology defect: a newly created
chat appeared under Older with a 2965-week label. The reducer converted message
sequence numbers into Unix timestamps, the runtime guessed replacement dates,
and history grouping compared dates against the newest saved conversation.
The UI now supplies wall time to the pure reducer before user input and runtime
events; changed messages advance activity time without replacing creation time.
Navigation does not touch dates, rollback cannot move a thread's date backwards,
and runtime storage preserves caller dates exactly. History grouping uses the
current clock. Original evidence continues to use its independent runtime source
timestamp. Regression coverage includes immediate headless display, older history,
reply completion, navigation, clock rollback and continued chats after disk reopen.
This increment passes `cargo xtask ci`: **957 tests, 10 skipped**, including the
minimum/wide/200% UI checks. The rebuilt native app was manually restarted twice
and driven through the stdio Development MCP. A fictional "Cobalt lantern date
check" conversation displayed **Today / now** immediately and after reopening;
its exact user/helper source records remained available with distinct authorship,
with zero pending checks and no new personal detail. Both test sources were
withdrawn through the confirmation controls, the filtered active-source list was
empty, and the temporary chat was deleted. The earlier conversation was restored.
MCP screenshot capture still reports unsupported renderer; local OS screenshots
were visually inspected and kept outside the repository. Broader provenance,
backfill and evaluation work remains open.

The timestamp increment also builds in release mode (3m 51s on this machine).
`target/release/sovra` is one 221,362,464-byte native ELF executable, with normal
X11/keyboard/C-runtime shared libraries and no new service or language runtime.
Installer packaging and a clean-machine launch were not repeated for this increment.


## Attributed conversation admission — verified increment, 2026-11-03

`MemorySourceAdmission` now carries producer identity, immutable version, explicit
user/helper authorship, provenance, private permission and the original conversation
and source timestamp. Storage assigns admission time and atomically creates user
extraction work. New assistant replies use this port, emit `SourceSaved` only after
persistence and surface storage failures without failing the reply. They create no
personal-fact extraction job. Saved-source review labels helper replies explicitly.
Source-version withdrawal cancels work across the whole identity family and blocks
new versions from reviving withdrawn evidence. Schema 9 preserves existing IDs and
provenance; legacy family identity is the existing record ID, while missing author,
permission and admission time stay unknown.

Assistant cross-chat recall remains disabled: dependency-aware withdrawal is still
needed so a helper's repetition cannot resurrect a removed personal detail. The
current `ConversationContext` preserves roles, text and speaker but does not carry
immutable source references; recall is injected at the provider boundary, and tool
results have separate readers. A safe assistant-recall implementation must track all
of those dependencies through capture, including working context after reopening,
rather than merely scan the reply for citation strings. This
inline admission contract does not yet unify document/artifact readers or every
team/connected-message producer. `cargo xtask ci` passes: **949 tests passed, 10 skipped**. This includes
real-engine idempotence/version/withdrawal tests, schema 8→9 migration, actual
`EXPLAIN FULL` checks for indexed family lookups, disk-reopen chat attribution and
an explicit runtime storage-failure test. The first migration regression failed
because the newly defined composite index did not select missing legacy family
fields; an unconditional, coalescing migration preserves existing values and visits
legacy rows. A failing runtime regression first showed `LegacyUnknown` for a real
streamed reply; it now shows `AssistantOutput` after reopening.

After a manual development reload, connected stdio MCP created a fictional Juniper
Comet reply. Original-record review showed its exact wording and **A reply from a
helper**. The personal-detail count stayed at four with no new extracted detail.
Both synthetic source records were withdrawn through the confirmation controls,
and the filtered browser then showed no records. The temporary chat was deleted
and the prior earnings conversation restored. Minimum/wide/200% source-browser
coverage passed headlessly; this increment's native run used the existing window
and text size. Earlier release and corpus measurements apply to the prior checkpoint,
not this new admission code. No new performance claim is made.

## Previous checkpoint — 2026-11-03

The earnings recall failure is fixed and verified through the connected Development
MCP. Edit/removal, older details, multiple concurrent lookups, failure isolation,
and source-only recall have regressions and live observations below. Full CI passes
**940 tests, 10 skipped**. The final optimized release build passes (3m51s), producing
one 221,307,928-byte native executable with the same Linux system-library dependencies
listed below. No additional application/database runtime is required.

The final synthetic runtime fixture retrieves expected original evidence for
**576/576 requests** at 1k/10k/100k sources and 1/4/16 concurrency. At 100k, measured
p95 dispatch-to-provider latency is 44.5 / 59.7 / 138.4 ms. This excludes real model
inference and does not establish general answer accuracy or a speedup. Failed
baseline and intermediate reports are preserved alongside the passing result.

**The full accepted all-source/backfill plan remains unfinished.** Common versioned
admission for files/artifacts/assistant and connected-channel evidence, complete
permission revocation across those readers, typed assertion/plan/commitment
classification, optional backfill UI/progress, policy-fence separation, missing
performance counters and LongMemEval evaluation still require work. The seven-case
oracle subset is pinned but has not been evaluated. This checkpoint must not be
reported as completion of that broader plan.

## Reliable everyday recall — active investigation, 2026-11-03

The current objective includes dependable automatic recall across interactions,
editable/removable knowledge, continuing background learning, bounded parallel
work, measured latency, and deployment in the existing native binary. The earlier
browsing completion audit below does not establish those outcomes.

### Observed failure and first regression

Development MCP and a native window capture located the conversation asking
“how much do I earn?” The reply incorrectly asserted that income was not saved.
The saved detail uses “monthly income”; meaning-based retrieval was off. Inspection
shows that private chat, team and attachment turns already wrap the model with
automatic recall. The failure is not simply an absent memory-tool invocation:
word matching misses the paraphrase, while the lookup instructions previously
emphasized historical queries. A new disk-reopen/runtime regression reproduces
the mismatch with synthetic income, tools disabled and no embedding service.
Before the fix the actual provider request contained the question itself as a
source, but omitted the saved detail. This also shows why “nonempty sources” is
not a sufficient test of successful recall.

The first change supplies up to eight recent current details as a separately
labeled `background` working set. It shares the 16 KiB evidence cap and has its
own 4 KiB cap; direct matches get space first. A bounded storage read overlaps
keyword, optional semantic and original-source retrieval. It adds no model call
or external service. Evidence still requires exact source grounding, current
validity and final pause/revision checks. Historical lookups omit this current
working set. This is not a complete profile or a solution for every old memory
in a large library. Lookup guidance now requires reformulating missing personal
context rather than asserting that an empty search proves nothing was saved.

### First verification checkpoint

`cargo xtask ci` passes: **919 tests passed, 9 skipped**. The real runtime/disk
regression now verifies the differently worded income question, a subsequent
correction and removal from current background context. Core tests cover source
grounding, deduplication, byte/item caps, entity scope, and exclusion from historical
lookups. Related-query and semantic-consent tests now distinguish targeted results
from independently supplied background; unauthorized, withdrawn, future and
unsupported evidence remain excluded, and embeddings still require consent.

Through Development MCP, a synthetic note saying a package should go to Reykjavik
was recalled for “What is the parcel’s destination?” A native correction changed
it to Tallinn, and a fresh chat used Tallinn. After stopping that version, a
different question retrieved the older original note. This is a confirmed gap:
version-specific **Stop using** is not deletion of the whole logical memory.
Removal across correction history and source-only retrieval remains required.
The new regression's current-background check does not cover that stronger case.

The user's exact earnings question was also repeated in a fresh live chat. The
configured response model (`anthropic/claude-haiku-5.5`, OpenRouter) answered from
the saved income and expenses and displayed three navigable source controls.
The answer remains in the native chat for review. This is one observed successful
answer, not a benchmark or a latency result. The initial observation loop timed
out because navigation moved away from Chat; reinspection found the completed
conversation. No request was resent. Native OS window captures were inspected;
MCP screenshot capture remains unsupported by this renderer.
The three synthetic chats were removed through their native controls. Both test
note sources and the six generated question/reply records were withdrawn through
source review (retained as stopped history), and the successful earnings chat was
left open. Existing personal facts were not edited or removed.

Concurrent workspace changes replaced the watched runner with a manual one-shot
`cargo xtask dev`. Verification used that rebuilt binary and did not reinstate
Watchexec. The app remains running with Development mode visible.

### Whole-history removal — implementation checkpoint

The confirmed native failure above now has a distinct `MemoryAction::Remove`
and backend-neutral `MemoryStore::remove_memory` operation. **Remove from memory**
is available on current and historical non-stopped details. Its confirmation
explains that all corrected versions and supporting source records stop appearing
in recall, while stopped review history, unrelated details and original chats remain.
The existing per-version withdrawal operation remains available to internal callers.

The adapter follows both directions of immutable correction edges in batches of
64, with cycle deduplication and no silent depth cutoff. A short transaction checks
the observed fact revision, withdraws the entire collected family, revokes original
and repeated source records, and cancels remaining work for those records. A changed
revision causes bounded fresh traversal/reconciliation; exhaustion reports a retryable
error. No model call occurs in the transaction. Source records shared with other
details are suppressed as whole-record search results, while unrelated exact fact
evidence remains available. This is recall withdrawal, not physical erasure of chats
or database history, and does not identify arbitrary unlinked future paraphrases.

The regression first failed with the unsupported port, then passed against the
embedded engine. Coverage includes removal from each of three versions, idempotence,
current/historical source search, preserving another detail in the same original
record, repeated sources, remaining leased chunks, known aliases, and an intervening
manual correction. The concurrency regression also exposed cancelled sibling query
errors masking the original transaction cause; error selection now preserves that
cause deterministically. Runtime/provider verification removes an edited synthetic
income detail, reopens disk storage, and checks both a paraphrase and an exact-word
question against the actual model request.

Full CI passed this checkpoint: **922 passed, 9 skipped**, including controls at
640×420, 2560×1440, and 200% text size. After the manual development reload,
Development MCP saved a synthetic amberglass parcel note, edited Reykjavik to
Tallinn, opened the explicit removal confirmation, and removed it. Current saved
details returned from five to the original four. A fresh chat asked for the removed
destination with exact source words; the configured live model used neither city
and said it could not establish the answer. It also said additional searches had
unavailable results, so this establishes the deletion check, not general lookup
reliability. The synthetic chat was deleted and its question/reply source records
withdrawn through native controls. Searching original records for the synthetic
marker then returned no matches. Original personal details were preserved and the
successful earnings conversation was restored. The renderer still cannot produce
MCP screenshots; an OS capture of only the Sovra window was inspected instead.

### Independent retrieval failures — implementation checkpoint

A separate runtime regression reproduced a source-search failure discarding
successful fact retrieval. Each of the four retrieval legs now owns its timeout
under a shared deadline; an error or timeout retains successful siblings. The
two-second total budget reserves 400 ms for provenance and 100 ms for mandatory
final checks. Provenance failure drops ungrounded facts while keeping verified
original-source results. An explicit notice describes incomplete coverage. Policy,
revision and pause cancellation still apply to the whole assembled context.

The failing-provider and permanently-held-source regressions now pass through
`Client` and inspect the actual provider request. The first full CI checkpoint for
this change passed **923 tests, 9 skipped**; a further regression exercises removal
while an independent source read is pending. These are functional deadline tests,
not corpus-scale latency or throughput measurements. The revised UI also clears a
stale “saved note” message after editing/removal and reports the action performed.
Final CI with the withdrawal regression passes **924 tests, 9 skipped**
(`/tmp/sovra-memory-current-ci.log`). Release verification is recorded in the integrated checkpoint below.

Follow-up inspection of the live model's “additional searches” statement found
that the lookup receipt selector uses only the latest receipt in a request. Multiple
lookups emitted together therefore do not currently attach separately labeled
results for every requested query. The regression below reproduces and fixes this
without retaining evidence in tool responses.

### Multiple lookups per tool round — implementation checkpoint

The real ADK tool-loop regression first failed because the first of two queries
was absent from the next provider request. The selector now collects all receipts
from the latest matching tool round, including receipts in separate content records.
It keeps their original order, validates every query, and ignores previous-turn
receipts. The per-turn four-call budget is unchanged.

`MemoryReader::search_batch` adds a backend-neutral bounded batch contract. The
runtime overlaps up to four queries under one two-second deadline, reserves time
for a final batch-wide policy/revision check, and cancels all siblings on pause or
withdrawal. Readers without that guarantee decline multi-query requests rather
than performing unsafe sequential fallback. Successful independent query results
retain their own availability notices. Evidence stays ephemeral at the provider
boundary; ordinary tool receipts contain no recalled facts.

Multiple results are presented as separate query/evidence pairs so current and
historical filters cannot silently merge. They share a 16 KiB serialized payload
budget, including query labels. Oversized results drop whole records and disclose
that the selection is incomplete; source quotes are never cut. A single query
retains the existing top-level evidence arrays and adds its filters to that same
bounded payload.

Ten focused tests passed, including the real tool loop, payload size and intact
quotes, historical single-query compatibility, and HTTP/runtime overlap. The held
runtime test proves two independent queries start before either is released. It
then completes one and removes the saved detail while the other is held; neither
result reaches the provider. This is functional concurrency evidence, not a claim
about corpus-scale speed. The disk-reopen/older-memory test also passes; final full CI verification follows.

### Old details and focused evidence — verification checkpoint

A real embedded-database/runtime regression saves income and payday, adds 32
unrelated newer details, reopens storage, and asks a paraphrase with no useful
keyword overlap. The initial request lacks both old details; two model-requested
lookups recover both with citations. Removal plus a second reopen suppresses the
income while retaining payday. The HTTP model is scripted, so this verifies
retrieval and isolation, not natural-language answer quality.

An isolated configured-model evaluation (`synthetic-memory-old-recall-v1`,
`anthropic/claude-haiku-5.5` through OpenRouter, semantic search off) exercised
old income/payday, corrected income, and removal with synthetic data only. All
three answers recovered or withheld the expected amounts. The corrected answer
initially attached an unrelated inventory citation to payday, then explicitly
corrected itself. This is a citation defect, despite the correct answer content.
Observed response times were 5,476 / 5,258 / 6,157 ms under concurrent development
work; three observations are not latency percentiles or a benchmark. Tokens and
model cost were not instrumented in this run.

That observation motivated a failing regression: explicit focused searches were
still carrying unrelated recent background records. Initial automatic context now
uses `MemoryReader::search`, while tool requests use the bounded focused batch
contract, including a one-query batch. Focused results omit unrelated background;
automatic turn context retains the small working set. This reduces distractors but
does not by itself establish correct citations from every model response.

The post-change repeat recovered the old income/payday, used the corrected income,
and withheld both removed amounts while preserving payday. Manual review found
valid citations in all three answers. Its response times were 4,608 / 4,658 /
4,671 ms. These small stochastic runs do not establish a speedup or a general
citation-accuracy rate. [Before](evaluations/memory-recall-2026-11-03/before-focused.json)
and [after](evaluations/memory-recall-2026-11-03/after-focused.json) reports contain
only synthetic data, exact answers, expected results, settings and review limits.
Fourteen focused regressions passed, including older-detail recovery after restart,
withdrawal during overlapping queries, intact bounded quotes, and the shared
research-board/card integration test.

### Integrated native checkpoint — 2026-11-03

`cargo xtask ci` passed all gates: **937 tests passed, 9 skipped**. This includes
minimum 640×420, wide 2560×1440 and 200% text-size headless memory controls,
together with the shared research-board/card integration correction. The app was
manually rebuilt and relaunched through `cargo xtask dev`; Development mode
started on and its explicit indicator remained visible.

The connected stdio Development MCP submitted the user's earnings question in a
fresh native chat. The configured model answered from saved details and displayed
three source controls. Opening the first control showed the actual supporting
record. Personal values and the private OS capture are excluded from committed
reports. MCP screenshot capture is still unavailable in this renderer; the native
window was captured and inspected through the OS fallback.

A synthetic violetglass parcel note was saved, edited with retained history, and
removed through the explicit confirmation control. Inspection confirmed the new
correction/removal notices and a return from five to the original four saved
details. An initial test input appended to the prefilled edit box; a subsequent
edit selected/replaced its contents and confirmed the intended value. Whole-history
removal included this intermediate test version. Personal facts were untouched,
and the successful earnings conversation was left open. This latest native pass
covers source navigation and controls; the isolated model report and disk/runtime
regressions establish the old-detail, correction and post-removal recall checks.

`cargo build --release -p sovra-app` completed successfully in 7m18s. `file` and
`readelf -d` identify one 221,280,816-byte x86-64 Linux PIE executable with direct
links to libxcb, libxkbcommon, libxkbcommon-x11, libgcc_s, libm, libc and the system
loader. The embedded database is part of the executable; no Python/Node or external
database-server dependency is introduced. This is a release compilation and ELF
dependency check, not an installer/distribution or clean-machine smoke test. The
release binary was not launched alongside the existing development GUI.

### Runtime performance fixture

The opt-in `synthetic_runtime_retrieval_measurements` integration test exercises
`Client`, real embedded disk storage, automatic source retrieval, the ADK provider
boundary, durable capture receipts, and ordinary background extraction. It uses
1,000 / 10,000 / 100,000 synthetic admitted sources, 64 requests per size at each
of 1 / 4 / 16 concurrent requests, and a deterministic zero-cost loopback provider.
The source-only fixtures intentionally contain no extracted facts. Historical jobs
are cancelled before measuring; each batch's new extraction jobs finish before the
next batch begins. A copied completed `source-storage-v1` fixture may supply the
100k seed, avoiding a second long seed run while preserving its original database.

```sh
SOVRA_RUNTIME_BENCH_OUTPUT=/tmp/fresh-runtime-report \
SOVRA_RUNTIME_BENCH_SEED=/tmp/completed-source-storage-fixture \
cargo nextest run -p sovra-app --run-ignored only synthetic_runtime_retrieval_measurements
```

Omit `SOVRA_RUNTIME_BENCH_SEED` to create all synthetic databases from scratch.
Never supply production data. Reports separate dispatch-to-provider latency from
client-observed capture/completion upper bounds; the former includes runtime
scheduling and loopback HTTP overhead, and excludes live model inference. The
fixture reports grounded provider requests, queue counts and extraction calls.
SQL query counts, conflicts, oldest queue age and actual-provider token/cost
measurements remain unavailable rather than being reported as zero. This fixture
does not evaluate semantic retrieval or generated-answer correctness.

The first runtime measurement exposed a source-ranking defect before it could be
used as a successful-performance result. At both 1k and 10k, only the first three
of 64 sequential requests received the expected original source, and subsequent
4/16-concurrency runs received none. `Recall fixturecodeN` also matches the word
“Recall” in earlier questions. Original-source search ordered by recency before its
12-row limit, allowing those questions to displace the old answer; the provider
context's smaller source budget made the problem visible even earlier.

A real-engine regression reproduced this with one old answer and forty newer
questions. Search now sums BM25 term scores, then breaks ties by time and stable
ID, preserving scope, temporal, withdrawal and result bounds. The pinned 3.3.4
planner resolves `search::score` references only from direct bind parameters or
literals, not `$terms[index]`; source terms therefore use individually bound
parameters. Pinned-engine regressions cover relevance/withdrawal and `EXPLAIN FULL`
index use for one and two terms. The revised runtime measurement follows below.

The intermediate summed-BM25 repeat recovered **531/576** expected sources but
failed its all-evidence assertion. It still missed early sequential queries where
short questions received stronger length normalization than the longer original
answer. That is retained in `runtime-bm25-intermediate.json`, not counted as a
successful benchmark. A second real-engine regression reproduces the miss with
1,000 originals and five short recent questions.

The final implementation ranks each term separately with BM25 and batches the
bounded term queries into one adapter request. Core combines them using the same
pure reciprocal-rank fusion implementation as fact retrieval. Smaller result legs
win score ties; source versions stay distinct and duplicate hits do not receive
extra votes. At most eight terms produce twelve rows each, fused to twelve before
the existing provider byte/item budget. The second regression now preserves the
expected original inside the actual provider source budget. Seven focused tests
pass, covering both ranking failures, deterministic fusion, limits, versions,
withdrawal and the real full-text query plans.

The upstream LongMemEval oracle dataset and official evaluator have also been
pinned in [a preparation manifest](evaluations/memory-recall-2026-11-03/longmemeval-oracle-manifest.json).
It selects seven question IDs deterministically across categories, including
abstention. **No LongMemEval evaluation has run yet.** Oracle evidence sessions
exclude the full distracting history, so any later result must be labeled as an
oracle subset and cannot be presented as a LongMemEval-S retrieval score.
Dataset and protocol: [official repository](https://github.com/xiaowu0162/LongMemEval)
and [cleaned release](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned).

### Runtime measurement result — 2026-11-03

The final opt-in run passed: **576/576** provider requests contained their expected
original source, across 64 requests for every corpus/concurrency pair. All 576 new
inputs received durable capture receipts and completed extraction; every measured
stage ended with zero waiting/working jobs. This is exact-source retrieval with a
local scripted provider, not a generated-answer quality score. Full CI with the
fusion changes passes **940 tests, 10 skipped**.

Dispatch-to-provider **p50 / p95 milliseconds** (64 samples per cell):

| Initial sources | 1 concurrent | 4 concurrent | 16 concurrent |
| --- | --- | --- | --- |
| 1,000 | 14.4 / 18.5 | 29.2 / 36.2 | 45.5 / 54.8 |
| 10,000 | 17.1 / 20.2 | 27.7 / 34.7 | 46.3 / 54.3 |
| 100,000 | 39.5 / 44.5 | 40.0 / 59.7 | 71.7 / 138.4 |

This includes runtime scheduling, retrieval and loopback HTTP delivery. It excludes
live-model inference. The machine was an AMD Ryzen 7 3700X (8 cores/16 threads),
about 62,195 MiB RAM, using the unoptimized test profile and the existing native
app in the background. This agent launched no build/CI during the final measured
stages. Earlier runs overlapped development work, so their timings are not a
controlled speedup comparison. Client-observed capture-ack p95 at 100k was 1.7 / 5.9 /
72.7 ms; these are upper bounds because queued events can wait behind another
request while the fixture observes them.

[Failed recency baseline](evaluations/memory-recall-2026-11-03/runtime-before-ranking.json),
[failed summed-BM25 intermediate](evaluations/memory-recall-2026-11-03/runtime-bm25-intermediate.json),
and [final per-term fusion report](evaluations/memory-recall-2026-11-03/runtime-after-fusion.json)
retain individual latency samples, queue counts, scope, environment and missing
metrics. Corpus contents are synthetic; the 100k fixture was copied from the
previous isolated storage run, whose original database was preserved.

With instantaneous fixture extraction, each 64-input phase still takes roughly
16 seconds to drain: the worker currently claims one job per 250 ms tick. Four
longer model calls can overlap, but this scheduling interval limits empty-work
throughput. SQL query counts, transaction-conflict counts, oldest queue age and
live-provider token/cost instrumentation remain outstanding. No “blazing fast”
claim is inferred from these bounded source-only measurements.

### Final source-only native check

After rebuilding the final per-term fusion version through `cargo xtask dev`,
Development MCP submitted a fictional copperlark capsule labeled “marigold
waypoint.” Knowledge showed the original four personal details, zero pending or
failed jobs, and “No new detail was saved.” In a **fresh chat**, the configured
model recovered the label, explicitly kept its fictional attribution, and supplied
an original-source citation rather than a fact citation. Opening the citation
showed the exact source wording and completed processing state. This confirms a
live case where evidence omitted by extraction remains available independently.

All four synthetic question/reply/source records were withdrawn with their native
confirmation controls, both synthetic chats were deleted, and source search for
the marker returned no records. The earnings conversation was restored. Personal
facts were not changed. The OS capture remains private and is not included in the
repository; MCP renderer screenshot support remains unavailable.

### Research informing the remaining work

Primary sources read on 2026-11-03; dates below identify the research versions,
not an assertion that vendor scores transfer to Sovra:

- [LongMemEval, ICLR 2025, revision 2](https://arxiv.org/abs/2410.10813v2)
  separates indexing, retrieval and reading, and evaluates updates, temporal
  reasoning, multiple sessions and abstention. Evaluate those stages separately.
- [Anthropic Contextual Retrieval, September 2024](https://www.anthropic.com/research/contextual-retrieval)
  combines contextualized chunks with lexical and semantic retrieval. Preserve
  original passages and their context; do not rely only on extracted facts.
- [Anthropic context engineering, September 2025](https://www.anthropic.com/research/effective-context-engineering-for-ai-agents)
  describes combining supplied context with just-in-time discovery. A compact
  working set complements targeted search; unlimited prompt stuffing is not the goal.
- [Letta memory blocks](https://docs.letta.com/v1-sdk/memory/memory-blocks)
  documents always-visible bounded context alongside archival retrieval. Sovra's
  initial recent working set is simpler than curated persistent memory blocks.
- [Hindsight, December 2025](https://arxiv.org/html/2512.12818v1)
  combines parallel lexical, semantic, graph and temporal retrieval, fusion,
  reranking and a context budget. Its world/experience/opinion distinctions argue
  for preserving attribution rather than turning assistant output into user facts.
- [SimpleMem, January 2026](https://arxiv.org/html/2601.02553v1)
  describes context-independent memory units and adaptive query-aware retrieval.
  Query reformulation and sufficiency checks need tests beyond exact-word matching.
- [Storage Is Not Memory, May 2026 technical report](https://arxiv.org/abs/2605.04897)
  emphasizes verbatim events and retrieval-time selection. Preserve evidence
  omitted by extraction; evaluate end-to-end answers as well as retrieval hits.
- [DolphinBench, September 2026 revision 2](https://arxiv.org/abs/2609.24971v2)
  evaluates implicit memory-dependent tasks and reports cost and latency alongside
  correctness. Tests must include requests that never explicitly say “remember.”
- [When Retrieval Is Not Enough, October 2026](https://arxiv.org/abs/2610.36308)
  finds stale competing statements harmful even when the newest statement is
  retrieved. Current-answer tests must inspect obsolete evidence and generation,
  not merely assert that the newest fact occurs somewhere in the prompt.
- [Tokio cancellation guidance](https://tokio.rs/tokio/tutorial/select)
  documents cancellation through dropping futures. Runtime work must remain
  bounded and owned; independent reads should overlap without detached tasks.

### Acceptance still to establish

| Requirement | Current evidence / remaining check |
| --- | --- |
| Automatic recall in every permitted private turn | Wiring exists for chat, team and attachments; prove paraphrases, implicit follow-ups and old details beyond the working-set limit. Respect remembering-off and connected-sender scope. |
| Edit/remove individual knowledge | Existing correction and stop-using transactions/UI exist. Audit discoverability, correction ancestry, original-source retrieval and re-learning after removal; clarify retained history versus permanent erasure. |
| Continuous learning across interactions | Durable user-message capture/chunking exists. Verify failures/retries, corrections, source-only evidence, relevant assistant output with separate attribution, and restart behavior. |
| Fast and parallel | Four extraction slots and overlapping retrieval already exist. Measure the revised runtime path at 1k/10k/100k sources and 1/4/16 requests, retain fast successful legs when another fails, avoid repeated model work, and inspect real query plans. No speedup claimed yet. |
| Answer correctness | Synthetic HTTP regressions inspect provider inputs; run live-model positive/negative/update/delete checks and fixed versioned fixtures. A green transport test is not an answer-quality score. |
| Single binary | Final optimized release build and ELF dependency inspection pass: one native executable with embedded storage and Linux system libraries. Installer/clean-machine verification was not run. |
| Native UI and integration | Run full CI and Development MCP, including editing/removal and responsive controls; retain explicit live-verification limitations. |

## Memory browsing experience — implementation and local verification, 2026-11-02

The active design objective is an understandable, powerful workspace for reading
stored knowledge, checking its evidence and exploring its connections. The first
implementation adds readable dates, subject filters, explicit connection reasons,
and a return trail to the live saved-memory browser. Connections come from real
stored subject, source and correction links; no model invents edges for display.
Each neighborhood is bounded to 24 items, labels incomplete coverage, and lets
people open adjacent versions or filter by an exact subject. Related current
details exclude stopped versions; explicit correction links retain review history.
The overview counts stored subject links across all current facts, independently
of the visible page, and shows the first 12 by count then name. It labels the
selection when more exist and excludes the generic `user` label. Selecting a
subject applies an exact filter before pagination. Recent activity opens the
retained original message, including checks that found no new detail. Long
sources show a Unicode-safe 600-character excerpt with an explicit full-text
control; the stored evidence is unchanged.

The design uses these researched principles:

- [NN/G: recognition rather than recall](https://www.nngroup.com/articles/recognition-and-recall/): expose recognizable subject labels and readable statements so people need not remember search syntax or internal IDs.
- [NN/G: progressive disclosure](https://www.nngroup.com/articles/progressive-disclosure/): start with readable cards, then reveal evidence, connections and correction controls in the detail reader.
- [Li et al., Knowledge Graphs in Practice, version 4](https://arxiv.org/abs/2304.01311v4): the study identifies challenges with generic node-link diagrams for consumers and discusses knowledge cards, temporal views and discovery. Sovra keeps labeled, navigable connection cards; a shared subject is explicitly distinguished from an asserted relationship. The memories page also draws a bounded picture of those same stored links, with the words still readable beside it. A generic "you" or "user" label alone is not a link. When the bounded picture leaves links out, the screen says more exist. Browser graph engines were not embedded.

**Check for repeats** on that page compares loaded wording on the device. It does not call a model or the network, and it does not delete or edit a record. Wording that normalizes to the same statement keeps both records and stores the older one as an earlier version. Close wording is listed in everyday words and is not joined until the person confirms. That confirmation stores the same non-deleting link, so both details stay on the page after a refresh and the close pair is not listed as waiting again. A genuinely different detail is not listed. The picture names a conversation by the title stored for that chat. Two conversations with the same title stay two nodes. A note typed on the memory screen is not drawn as a chat.

These are design decisions, not usability-study results for Sovra. The browsing
implementation and local verification audit are complete for the knowledge
currently retained by the app; coverage and live-renderer limits are recorded
below. This does not complete the separate all-source capture/backfill plan.
The recent activity list remains a
bounded receipt view; the original-record library provides complete paged access
to the source episodes currently retained by memory.

### Supporting records for repeated memories — 2026-11-02

**Browse supporting records** on a saved-memory detail scopes the original-record
library to that exact fact. Search, paging, source inspection and the stopped
record filter preserve that scope. The selected statement stays visible. Returning
requests a fresh detail from storage and restores its exploration trail, including
the route back to a conversation when opened from a reply citation.

Provenance has two existing forms: `derived_from` records the original source,
while durable processing jobs retain fact IDs reused for later statements. The
browser reads both forms without adding edges, changing fact IDs, or inventing
new authorship. Per-record linked-detail counts deduplicate their union, and the
reverse source-to-details query uses both. This fixes repeated records previously
appearing to have no linked details. Stopped evidence remains explicitly opt-in
for local owner review; model recall policy is unchanged.

Verification: `cargo xtask ci` passed every gate: **916 tests passed, 9 skipped**.
The real embedded-store regression first reproduced the unrelated-record leak,
then verified exact fact filtering, equal-time paging, repeated-source counts,
reverse links, withdrawal and retained history. A runtime test reopens the disk
database before reading the original and repeated supporting statements from
different conversations. Headless native controls preserve the search scope,
return trail and original chat-citation route in both themes at 640×420,
2560×1440 and 200% text size.

Development MCP on the existing watcher verified opening supporting records,
reading original text, returning to the detail, and opening the source's two
linked details. Native OS captures of the supporting library and overview were
inspected. The live selected detail had one supporting record; multiple repeated
records are covered by the real-store/runtime tests, not claimed as a live-data
check. The current-only, unfiltered overview was restored without changing stored
knowledge. The Development mode indicator and watcher remain active. MCP screenshot
capture is still unsupported by this renderer; minimum/wide/200% coverage is
headless, not live pixel inspection at each size.

### Browsing completion audit

The implemented paths cover reading current details and retained history; literal
search and a complete paged subject directory; labeled source, subject and
correction connections; full original records even with no extracted detail;
all supporting records retained for repeated statements; reviewed file
relationships and their saved source passages; and return navigation. Existing
add/correct/stop controls, confirmation and draft preservation, capture status,
retry, consent and folder revocation retain regression coverage in the full gate.
Empty, loading, failure and partial-coverage states remain explicit. Skills and
README describe the entry points in ordinary language.

This is an implementation and local verification result, not evidence from a
beginner usability study or a scored model-quality evaluation. The live file
workspace had no reviewed relationships; populated browsing and revocation were
verified through real embedded storage, runtime events and headless native
controls. Connection neighborhoods remain bounded selections, while subject and
source libraries page through their retained records. There is no claim that
the app has already captured every available document, artifact or assistant
statement; the separate plan's remaining work is listed below.

### Browsing reviewed file relationships — 2026-11-02

**Knowledge → Files → Browse saved relationships** exposes the already retained
file-knowledge graph through native controls, without invoking a model. Readable
statements lead to connected subjects, recording dates, any established world
validity, and all relationship/endpoint source passages. Following a subject uses
its exact stored ID. Inference, approval, source freshness and truth remain
distinct; explicit filters include outdated source versions or replaced claims.
Unknown world dates and origins remain unknown.

The browser sends typed `BrowseKnowledge` and `KnowledgeEvidence` commands through
the runtime's existing bounded reader tasks. Correlated results preserve errors
and reject old replies. Removing folder access or changing inclusion options
invalidates both the page and any pending evidence read. Evidence retrieval uses
the file adapter's existing saved-version and permanent grant-revocation checks;
no source files or graph records are changed by browsing.

One-hop graph queries now return an exclusive recording-time/ID cursor. It also
advances past a bounded batch of filtered records, so older matching evidence is
not stranded behind the traversal inspection limit. Multi-hop queries retain
their bounded traversal behavior and reject pagination cursors. A bounded return
trail restores earlier query views; it does not promise a snapshot across edits.
Empty results link to the relevant Skills card and its reviewable example. This
navigation does not send a message or run a model, and an unfinished custom-skill
edit remains open.

Verification: `cargo xtask ci` passed with 914 tests passed and 9 skipped. Real
file-store regressions cover equal-timestamp paging, insertion between pages,
and continuation after 1,000 temporally excluded relationships. The real runtime
reopens the disk store, returns the original cited text, and rejects that text
after folder removal. Reducer tests reject stale pages, mismatched evidence,
closed readers and replies arriving after invalidation. Native tests exercise
search, paging, subject navigation, retry, replaced/outdated labels, the return
trail and Skills navigation in both themes at minimum/wide/200% layouts. The
existing Add folders control remains visible on entry at 200% text.

Development MCP verified the entry point, Enter-to-search, empty results,
outdated-source selection, and the exact helper card on the existing watcher.
An OS capture of the native browser was inspected. The live workspace had no
saved file relationships, so populated relationship/evidence navigation is
verified by the real-store/runtime and headless tests, not claimed as a live
populated-data check. MCP screenshot capture remains unsupported on this renderer.
No user knowledge, files or model settings were changed by these live checks.

The complete **Browse all subjects** directory now pages alphabetically through
current saved labels, with literal case-insensitive search and per-subject counts.
It uses an exclusive name cursor rather than offsets; inserting an earlier name
does not duplicate later pages. Stopped and superseded facts do not contribute.
No historical processing or model calls are needed to browse. The generic `user`
label is omitted; untagged details remain available in the main saved list.
The core port, review commands/results, runtime and embedded adapter own the data
path; UI input only selects a query. Errors stay distinct from empty results.
Explicit navigation resets the page's scroll position after its accepted response;
background refresh and stale responses do not move the reader.

### Original-record library — 2026-11-02

**Browse original records** pages by descending source time and ID and searches
literal original wording before limiting results. Each card contains a bounded
600-character Unicode excerpt. Opening a record loads its full original text and
current provenance, processing and withdrawal metadata. It can open all linked
saved details, including retained history, without relying on successful model
extraction. Processing states are loaded once for each page as a bounded ID batch;
`EXPLAIN FULL` on pinned SurrealDB 3.3.4 verifies `memory_job_episode` for that read.
This is an additive index, reapplied with the existing schema contract.

This is explicit local review of retained records, separate from model recall.
Paused memory and stopped records can still be inspected by their owner; recall
continues to enforce its own permission/policy checks. The default list excludes
stopped sources. A confirmed withdrawal returns to the current source query;
**Include stopped records** makes the retained record reviewable with its stopped
label and disabled repeat-withdrawal action. It never grants recall access.

Unknown legacy origin and sharing permissions remain explicitly unknown. New
explicit user notes and corrections record their user origin and private scope
inside their existing atomic write. No model calls, historical extraction, or
file/artifact ingestion is added by this browser.

### Original-record verification — 2026-11-02

`cargo xtask ci` passed: 907 tests passed and 9 skipped. Embedded-store tests
cover literal search beyond the displayed excerpt, same-time pagination,
withdrawal and pause, explicit unknown legacy metadata, source-to-detail links,
and the job lookup query plan. The runtime regression reopens disk storage and
reviews original evidence with zero extracted details. Native headless tests
cover both themes at 640×420, 2560×1440 and 200% text, including search, paging,
linked history, withdrawal confirmation and cancellation, and return navigation.

The source reader omits the overview header, combines related metadata, and
wraps its actions. Citation tests now reveal controls inside the actual scroll
pane, avoiding clicks obscured by the fixed header. The full gate also exposed a
file-index test that confused elapsed time below one second with equal timestamp
seconds; its fixture now fixes the saved observation timestamp explicitly and
checks same-size edits with a preserved modification timestamp.

Development MCP on the existing watcher verified original-wording search,
reading evidence with no linked details, following a record to its saved details,
and returning to the overview. A native OS window capture of the compact reader
was inspected, including explicit unknown metadata for a retained helper reply.
The Development mode indicator remained visible. No stored records were changed.
The MCP screenshot operation remains unavailable on this renderer; minimum,
wide and 200% coverage is headless rather than live pixel verification at each
size. The watcher remains running.

### Subject directory verification — 2026-11-02

`cargo xtask ci` passed with 900 tests passed and 9 skipped after the directory
and scroll-preservation changes.

The real embedded-engine regression covers filtered alphabetical paging, insertion
before a cursor, Unicode labels, empty search results, and exclusion of stopped or
superseded details. Runtime events return the same directory after closing and
reopening the disk-backed database. Native headless tests cover both themes at
640×420, 2560×1440, and 640×420 with 200% text: long names, paging, Enter-to-search,
selection, empty/error states, retry and return navigation. A background-save
regression verifies that the scrolled reader keeps its position after refresh.

Development MCP on the existing watcher verified the directory, an Enter-submitted
no-match search, clearing back to all subjects, selecting a subject and opening its
filtered saved details. The unfiltered overview was restored without changing
stored knowledge. An OS capture of the native directory was inspected; the
previously recorded MCP screenshot-renderer limitation still applies.

### Verified browsing checkpoint — 2026-11-02

`cargo xtask ci` passed after the reading typography and action-layout changes:
897 passed, 9 skipped. Coverage includes the real embedded subject counts,
source/correction/subject links, runtime review events, and native headless
navigation at 640×420, 2560×1440 and 200% text size in both themes. Long sources,
long subject names, exact filters, empty results, return trails, and a source
whose extraction found no new details are exercised through real controls.

Development MCP on the existing watcher confirmed subject filtering, opening a
linked detail, retracing to the previous detail, reading an original message
from an empty extraction result, and returning to saved knowledge. These live
checks were read-only and left the unfiltered overview restored. Native window
captures through the OS were inspected for the overview and detail reader.
`sovra_screenshot` still returns “Screenshot capture is unavailable on this
platform renderer”; minimum/wide/200% coverage is headless, not a claim of live
pixel inspection at all sizes. The watcher remains running.

## Source-backed parallel capture — implementation in progress (2026-11-02)

Private user messages admitted through desktop chat, team prompts and the existing
private connected-messaging route now retain searchable original evidence separately
from extracted facts. Admission commits the episode and pending job atomically.
The 256-job admission ceiling is removed; the 64-input runtime buffer uses
backpressure and emits a saved receipt only after the storage write succeeds.
Disk/write failures remain explicit failures, never successful capture receipts.

Schema 8 preserves existing episode/fact IDs and correction provenance. Existing
jobs gain a cursor and conversation identity. Existing episodes gain
`legacy_unknown` origin and `unknown` permission metadata: migration does not
invent consent or authorship. Newly admitted user evidence is explicitly private,
version 1. The original-evidence reader filters scope, origin, time, remembering
policy and withdrawal before returning bounded, exact UTF-8 spans. Source citations
open the original message independently of fact extraction. They are historical
statements, not automatically current truth; answer guidance preserves that distinction.

Extraction uses process-wide permits (four by default; startup configuration
`SOVRA_MEMORY_EXTRACTION_SLOTS`, clamped to 1–16). Independent source chunks overlap;
only the earliest unfinished source in a conversation is claimable. Chunks are
contiguous and at most 2048 bytes, prefer sentence boundaries, and retain their
byte offset. Each committed chunk advances the cursor transactionally; failure
retries that chunk. A model call returns at most 32 grounded candidates per chunk.
This removes the previous eight-candidate message limit, but is not a guarantee
that a model finds every fact in an arbitrarily dense chunk. Original evidence
remains searchable regardless of extraction coverage.

Extraction receives at most three bounded preceding-context items and no live fact
IDs. A process-wide consolidation lock serializes fresh fact lookup and reconciliation.
Reconciliation may change only candidate references, not the extracted claims or
quotes. A separate factual revision participates in the commit transaction and
changes on manual edits, withdrawal and fact writes. Stale reconciliation retries
three times, then returns to persisted retry/failure handling. No model call runs
inside a database transaction. Both lexical and optional semantic retrieval start
together under the same deadline; primary provenance is loaded in a batch.

Grounded candidates and their extractor are persisted against the current lease
before consolidation. Reclaiming an interrupted chunk reuses those candidates;
advancing the cursor clears the staging record. Active chat, file and team turns
defer new background model calls. Existing provider calls are not preempted and
keep their timeout. Preceding context uses admitted, unrevoked sources only.
Original-source review has a confirmed **Stop using this message** action that
retains review history while revoking recall and cancelling pending processing.
The real-engine source-search `EXPLAIN FULL` regression verifies use of the
`episode_text` full-text index on pinned SurrealDB 3.3.4.

The existing `meta:remembering.revision` write fence remains deliberately in place.
Inspection of pinned SurrealKV shows that ordinary reads do not participate in
commit-time conflict validation (`get_for_update` reads do). Removing that fence
without an equivalent policy lock could let admission race a pause. Separating
queue transactions from this fence is still pending verified engine support.

Outstanding parts of the accepted plan: a common admission envelope for file and
artifact versions and assistant/team output; permission revocation across those
readers; typed assertion/plan/commitment distinctions; optional
reviewable historical backfill; source-edit UI; queue query-plan measurements;
full runtime/model benchmarks and versioned LongMemEval comparisons. These changes
must not be treated as completion of that larger plan. No production data is used
for benchmarks, and no historical backfill starts automatically.

### Storage measurements and verification — 2026-11-02

The isolated `source_pipeline_measurements` fixture completed 100,000 synthetic
sources on embedded SurrealDB 3.3.4 / SurrealKV. Raw results are in
[`storage.json`](evaluations/memory-sources-2026-11-02/storage.json). It used the
working tree's schema-7 source admission/search implementation, before schema 8
added extraction staging; no model, embedding service or evaluator was called
(zero model tokens/cost). Linux, Ryzen 7 3700X, 16 logical CPUs, repository debug
test profile with optimized dependencies; the watcher and builds shared the host.
These are observations from one run, not an isolated hardware comparison.

| Stored sources | Newly captured | Capture seconds | New sources/second | Retrieval p50/p95 ms, 1 request | 4 concurrent | 16 concurrent |
| --- | --- | --- | --- | --- | --- | --- |
| 1,000 | 1,000 | 1.264 | 791 | 1.64 / 1.73 | 3.09 / 3.92 | 9.77 / 12.18 |
| 10,000 | 9,000 | 31.477 | 286 | 4.78 / 40.05 | 13.43 / 14.35 | 55.05 / 58.08 |
| 100,000 | 90,000 | 2,358.348 | 38 | 35.54 / 38.81 | 136.76 / 393.12 | 493.62 / 567.20 |

Capture was sequential. Each retrieval concurrency used four batches (4/16/64
requests), with one source query per request. All exact-code lookups found the
omitted evidence, despite zero extracted facts. The fact-only score of zero is a
mechanical empty-fact fixture, not a baseline model-quality evaluation. There were
no capture/query errors. Queue age, conflict counts, concurrent capture throughput,
token usage under real extraction, answer correctness, abstention and LongMemEval
remain unmeasured. Capture slows substantially with growth; no speedup is claimed.

The final full CI run passed 874 tests, with nine opt-in tests skipped, including
minimum/wide/200% source navigation and withdrawal controls in both themes.
Real-runtime/disk regressions cover source-only recall after reopen, four active
leases plus a waiting fifth, responsive Ping, staged extraction recovery, manual
correction races, pause, and withdrawal. A live-check regression also
covers withdrawing failed sources without reviving them through Retry.

Through Development MCP on the existing watcher, the visible Development mode
indicator was confirmed, a synthetic Cedar-code message was saved, a separate
conversation produced original-source citation controls, and opening the first
control showed the exact earlier message after a watcher restart. Confirmed
**Stop using this message** returned its completion notice. The selected response
model reported `anthropic/claude-haiku-5.5` on OpenRouter. This is a navigation and
storage check, not a scored answer-quality result. `sovra_screenshot` returned
**Screenshot capture is unavailable on this platform renderer.** Pixel-level live
verification is therefore unavailable; the headless layout coverage passed.
Both synthetic sources were withdrawn, the two test conversations were removed
through their native controls, and the UI was returned to a new chat. The original
two saved details and three attention items remained unchanged. The watcher and
Development mode remain running.


Latest context audit checked 2026-11-01; earlier research checkpoints are dated below.
The seven implementation and local verification steps
below are complete in the working tree, including the file index, file knowledge,
skills and team integrations based on `79d86b4`. This is not a published or signed
release. The final audit below supersedes earlier checkpoint failures; measured
quality and portability limitations remain explicit.

## Helper context and concurrency audit (2026-11-01)

Delegated specialists previously received saved-memory lookup but no indexed-file
readers; team runs did not attach the file index. Both also missed the user's
standing instructions. They now receive the same granted read handles as chat,
including file search/catalogue, current and historical passages, reviewed file
relationships and saved-memory lookup. Current attachment readers are available
to delegates. Read handles retain their scope and revocation checks; proposal,
UI and unrelated connected-service tools are not inherited by this policy.
File and explicit lookup tools require tools to be enabled and supported by the
selected model. Automatic saved-memory retrieval still follows the remembering
setting independently of file indexing.

The repository uses **ADK-Rust 2.3.0**, a separate Rust implementation. Google's
ADK guidance informs the design; Python or Go examples are not assumed to be Rust
APIs. This audit checked the installed `adk-agent`, `adk-tool`, `adk-core` and
`adk-runner` 2.3.0 sources, particularly `LlmAgentBuilder`, `AgentTool`,
`ToolConcurrencyConfig` and `RunnerBuilder`.

Primary guidance checked on the web:

- [Google ADK context](https://adk.dev/context/): use scoped context and load larger
  content through references on demand. Sovra supplies granted readers rather
  than embedding the whole Knowledge library in every prompt.
- [Google ADK parallel workflows](https://adk.dev/agents/workflow-agents/parallel-agents/):
  concurrent branches must be independent, and data sharing must be explicit.
  Sovra isolates delegate sessions and returns their results to the parent.
- [ADK-Rust memory in agents](https://adk-rust.com/en/docs/memory/tools-and-agents):
  automatic retrieval and deliberate lookup serve complementary purposes.
  Sovra implements these through its existing core memory port and model/tool
  adapters so evidence validation, temporal queries, pause and review rules stay
  consistent with Knowledge. It does not add a second transcript memory store.
- [Tokio cancellation](https://tokio.rs/tokio/tutorial/select#cancellation):
  dropping owned futures cancels their work; separately spawned tasks need their
  own lifecycle. Delegation futures belong to the run and cancellation tests hold
  both active and queued work to verify that neither continues after its owner.

Context boundaries checked:

| Context | Contract and verification |
| --- | --- |
| Standing instructions | Preserved separately from a selected skill and supplied through ADK's global instruction to every specialist/team member; checked in fake-model and actual HTTP requests. |
| Current request | Parent explicitly delegates task details and attachment IDs. Helpers do not inherit sibling work or the parent's live tool-call history. |
| Earlier chat | The authoritative visible snapshot is bounded, preserves user/assistant roles and appears once. Fresh sessions exclude old tool calls, reasoning, widgets and provider continuation IDs. Delegation tests verify the same role-labelled history at the child boundary. |
| Selected procedure | Applies to the parent for this turn; it is not promoted into standing instructions. A regression asserts the procedure marker stays out of child requests. |
| Saved memory | Fresh cited evidence is retrieved at the provider boundary, with a shared bounded lookup allowance. Runtime tests cover historical queries, unrelated/future/expired evidence, pause and revocation while reads are pending. |
| Files and relationships | Search then read an exact source reference. The real-index/HTTP test verifies both delegation and team replies; removing the root prevents subsequent source text from reaching the provider. Historical and graph tools retain their existing validation rules. |
| Attachments | The current turn's inventory is authoritative. Delegates read through its tool; unknown and previous-turn IDs fail, and raw media is not automatically copied into the child session. |
| Request budget | Final admission runs after context assembly and counts instructions, history, evidence, tool schemas/results and media reservations. Optional history/evidence yields first; oversized mandatory input fails before provider IO. Estimates are conservative, not provider-specific tokenization. |
| Team conversation | Bounded transcript plus the explicit current request, followed by ordered member replies and the Conductor. Team work remains sequential because later replies use earlier ones. |

Independent delegation batches now queue above **four active calls** through
`RunConfig.tool_concurrency`. This is a per-invocation limit, not a process-wide
provider quota; separate conversations can each have active work. Ordinary tool
calls on the parent retain ADK's sequential ordering. A helper enables automatic
parallel tools only when its entire tool set is read-only and concurrency-safe;
otherwise it preserves call order. The pinned automatic dispatcher moves safe
reads ahead of ordered calls in a mixed batch. A regression reproduced that
reordering and verifies that the guard prevents it. Document reading can also
use bounded batches inside a helper; four delegates is not a global model-request
cap. Independent keyword memory reads separately use
four owned futures per lookup and preserve query order for deterministic fusion.

Regression coverage includes:

- `sovra-adk`: helper/team source access; standing instructions and selected-skill
  scope; current attachment inventory; six delegates with four held active and
  two queued; cancellation of both groups; mixed-tool ordering; history restoration
  and budget tests.
- `sovra-app::memory_lifecycle`: a real temporary folder and local file index,
  runtime `Client`, actual ADK execution and an HTTP model fixture that performs
  search/read/delegation. It asserts source-bearing chat/team text and revocation.
  This verifies wiring and protocol behavior, not a live model's answer quality.
- `sovra-runtime::models`: memory retrieval across chats, teams and attachments;
  historical lookup; model/context changes; pause/revocation and concurrent reads.

Verification on the shared macOS checkout: `NEXTEST_TEST_THREADS=4 cargo xtask ci`
passed every gate, with **744 tests passed and seven opt-in tests skipped**.
The complete CI log from that checkout (not retained in `docs/evaluations/`) recorded the result,
including the concurrent document-tool changes. Four test workers avoided the
startup timeout seen when multiple full CI suites competed for resources; test
deadlines and assertions were unchanged. This supersedes the earlier shared-checkout
build failure recorded below.

The desktop sends an explicit history snapshot. The existing inbound WhatsApp
path uses its ADK session history instead; it is not claimed to have desktop
snapshot restoration. Removing a grant prevents future reads; it cannot retract
content already delivered to a provider or erase a visible earlier reply.

## Capture feedback and concurrent reads (2026-10-31)

The current work preserves accepted memory inputs on normal shutdown: closing the
owned input channel lets queued writes finish in order before the worker exits.
Unfinished extraction is cancelled and recorded as interrupted, subject to the
existing retry budget. Explicitly pausing remembering still cancels pending work.
Sudden process termination or a failed disk write is not a successful save.

Independent keyword reads now overlap, at most four per lookup, and return in
query order for deterministic fusion. Extraction checks recent context alongside
keyword candidates. Existing deadlines and pause cancellation remain in effect.
Model extractions remain ordered because a later correction needs the saved fact
IDs produced by earlier messages; overlapping writes alone would not prevent stale
model context. This is bounded concurrent IO, not a claim of a universal speedup.

A live synthetic smoke check using the currently selected OpenRouter model,
`google/gemini-4-argon`, captured a jasmine-tea preference and a Porto residence
with its explicit 2024-01-01 start date. A separate conversation recalled jasmine
tea and cited the actual saved fact; the question added no fact. Both extraction
jobs completed on attempt one. The first reply and extraction completed in 6.465 s;
the recall reply completed in 4.420 s. This small observation is not a latency or
accuracy benchmark. The [retained report](evaluations/knowledge-2026-10-31/smoke.json)
contains only synthetic messages and was generated from an isolated copy of the
knowledge changes while a separate session edited MCP support in the shared
checkout. No synthetic memories were added to the user's store.

Memories shows the most recent completed capture receipt, including empty results
and failures, above actual saved details and their sources. Optional meaning-search
setup follows the results. Files puts reading status before search, reports empty
searches, and accepts Enter. The teaching example is explicitly labelled; it does
not masquerade as captured information.

Primary guidance checked online for this change:

- [W3C status messages](https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html):
  report waiting, success and failure without taking focus; native status roles and
  plain text apply that principle here, without claiming web conformance.
- [W3C older users](https://www.w3.org/WAI/older-users/): readable controls, text
  scaling and clear feedback inform the minimum/wide/200% checks in both themes.
- [Tokio channels](https://tokio.rs/tokio/tutorial/channels): owned message-passing
  workers and bounded work keep background activity separate from interaction.
- [Async-channel close](https://docs.rs/async-channel/latest/async_channel/struct.Sender.html#method.close)
  retains queued messages for receivers;
  [Futures buffered](https://docs.rs/futures/latest/futures/stream/trait.StreamExt.html#method.buffered)
  limits in-flight futures while retaining output order. These are the shutdown
  and lookup mechanisms used here.

Verification entry points:

```sh
cargo xtask ci
# macOS, native offscreen Metal rendering of clearly labelled synthetic UI data:
SOVRA_KNOWLEDGE_VISUAL_OUTPUT=/tmp/sovra-knowledge-visual-new \
  cargo run -p sovra-ui --example knowledge_visual
```

The optional visual proof writes Files, Memories, capture-receipt and source
screenshots at minimum/standard/wide/200% sizes in both themes. It uses scripted protocol
events and fictional details; it does not prove a model extracted those details.
The example runs on the process main thread for AppKit and uses a hidden native
renderer; it does not open another app window or replace the development watcher.
The retained native previews show [Memories](evaluations/knowledge-2026-10-31/memories.png),
[Files](evaluations/knowledge-2026-10-31/files.png),
[the original message](evaluations/knowledge-2026-10-31/source.png),
[the minimum window](evaluations/knowledge-2026-10-31/minimum.png), and
[200% text](evaluations/knowledge-2026-10-31/large-text.png). All use fictional data.
The real-disk lifecycle test separately checks saved evidence in provider requests
after a runtime restart. The opt-in model smoke check below evaluates actual model
capture and recall in an isolated store, without adding test memories to your data.

`cargo xtask ci` passed every gate in the verification checkout based on `6e3185b`
with these knowledge and build-runner changes, excluding the concurrent MCP work:
**677 tests passed, six opt-in tests skipped**. The complete CI log from that checkout (not retained in `docs/evaluations/`)
records formatting, Clippy, layering, styling, dependency policy, unused dependency
checks and tests. The opt-in live smoke check above was run separately. All 100 UI
tests also passed together with fail-fast disabled; the native renderer produced
32 screenshots across four window/text configurations and both themes.

At that 2026-10-31 checkpoint, the shared checkout's `cargo xtask ci` reached Clippy but could not compile the
separate MCP work: `Event::Mcp` is unhandled in `sovra-runtime/src/connections.rs`
and `sovra-runtime/src/lib.rs`, and `Command::Mcp` is unhandled in the runtime
command loop (failed shared CI log from that checkout, not retained in `docs/evaluations/`).
This also prevented the development watcher from launching that
shared app. The existing `xtask dev` / Watchexec supervisor remained running; it was
not replaced with a verification binary. At that checkpoint the combined checkout
was not green; the 2026-11-01 result above records the subsequent successful check.

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
