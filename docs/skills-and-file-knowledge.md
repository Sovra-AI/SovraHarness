# Skills and local file knowledge

Implementation record for the 2026-10-25 request. This is an active implementation,
not a claim that the items below have shipped. The reference supplied with the request
is `pasted-text-1.txt` (48,618 bytes), read in full before design work.

## Automatic skill application (2026-11-03)

Explicit `/skill` selection forces a procedure. Without it, the orchestrator gets a
bounded standing list (`SkillCatalog::auto_apply_guide`, at most 6 KB, descriptions cut
to 280 characters) of procedures it may load through `load_skill`. Progressive
disclosure: the list holds only names and descriptions, and bodies load when the
model picks one. Design choices:

- **Only skills that add guidance are listed.** `Skill::auto_apply` defaults on for
  user skills, the file procedures, and the terminal procedure (so file changes go
  through the approval card rather than a chat question). It is off for specialists,
  which routing already consults, for web search, which is routed, for save/presentation
  tools, whose tool descriptions suffice, and for workspace actions. This keeps the
  model from loading skills that change nothing.
- **Precision over recall.** The guide says to load a skill only when the person
  wants the outcome that skill produces, not when a word merely overlaps; to load
  the single most specific match, or two for a request with two parts; and never
  for chatter.
- **Installed skills come first**, because they encode the person's own routines.
  When the budget runs out, the guide says how many skills were left out and that
  `load_skill` search covers them.
- **Model-chosen loads are not user selections.** `Skill::applied_block` omits the
  `Active user-selected skill.` marker, so a model's choice cannot trigger the
  forced delegation a `/` command does. Descriptions are stripped of that marker
  before they are listed.
- **Capability-scoped.** A skill appears only when every tool it names is present in
  the turn. Untrusted message senders get neither `load_skill` nor the list.
- **Opt-out:** the skill editor's **Use it on its own when a request fits** checkbox,
  `sovra-auto-apply: false`, or the Agent Skills `disable-model-invocation: true`.

## Scope and acceptance

- [ ] One validated, extensible skill/capability catalogue drives slash discovery,
  help, skill details, model discovery, and execution; every available tool is reachable.
- [ ] Native composer supports `/`, ranked search, aliases, keyboard selection,
  cancellation, usage hints, and accessible rendering at minimum/wide/200% sizes.
- [ ] Explicit skill activation resolves before model execution. User text remains
  separate from instructions, stored conversation text, titles, and recall queries.
- [ ] Built-in and user-authored skills support discovery, activation, enable/disable,
  persistence, validation, and bounded on-demand resources. Skills cannot grant tools
  or filesystem access absent from the runtime's capabilities.
- [ ] `/files` provides scoped local file access; `/index-folder` opens the native
  directory chooser. Picker cancellation preserves the draft. Index progress, errors,
  cancellation, refresh, removal and recovery are visible.
- [ ] Persistent local indexing is incremental, bounded, cancellable, and excludes
  generated/noise inputs by policy. Changes, removals, renames and missed watcher
  events reconcile correctly. User-selected roots bound access, including symlinks.
- [ ] Stable file/chunk identity, content-defined chunk reuse, format-aware extraction,
  citations, metadata/lexical filtering, semantic ranking, and graph traversal.
- [ ] Part/whole hierarchy, logical entities separate from byte representations,
  provenance/epistemic tiers, transaction versus valid time, task/project context,
  and salience are explicit data rather than undocumented prompt conventions.
- [ ] Background summary hierarchy and compiled knowledge maps retain source lineage,
  invalidate stale descendants, and expose source/summary distinctions on retrieval.
- [ ] Structured file operations and compound retrieval/group/join workflows run under
  one supervisor; mutations show a concrete preview and require user confirmation,
  detect concurrent edits, preserve versions, and support rollback.
- [ ] ADK retains orchestration across multi-step work; bounded tool retries and
  cancellation do not abandon the requested synthesis after a specialist lookup.
- [ ] Platform ingestion has honest capability/fallback reporting. Privileged/raw-volume
  ingestion must be isolated from the normal desktop process and cannot silently
  elevate. Cross-platform and history guarantees require platform-specific evidence.
- [ ] Functional/security/retrieval tests, realistic latency/resource measurements,
  documentation, changelog and `cargo xtask ci` pass before completion.

## Existing SovraSRX behavior

The older `SovraSRX/agents` tree has no skill references. The current shipped system
is in `SovraSRXProduction`:

- `backend/src/app/services/skills/builtin/registry.py` validates unique names and
  aliases, required instructions, categories, and action handlers.
- `frontend/src/lib/chat/slash-command-rank.ts` ranks exact name, name prefix,
  aliases, label, then description; stable order breaks ties.
- `docs/features/slash-commands.md` distinguishes local UI actions from model skills.
  Server-side expansion separates a short request from the full procedure so recall
  and titles do not contain skill boilerplate. Missing arguments return usage guidance.
- `backend/src/app/skills/loader.py` discovers SKILL.md directories; per-user skills
  use a separate persisted catalogue and runtime `list_user_skills`/`load_user_skill`.

The native implementation preserves these useful contracts through core domain types,
runtime commands/events and native controls. It must not advertise the web product's
unimplemented integrations as working native tools.

## Research and design constraints

### Indexing UX work (2026-10-28)

The current request is to make indexing intuitive and powerful, prioritizing file
limits and skipped content. This goal remains active. The first change removes
fixed collection/scan limits and adds complete large UTF-8 indexing plus expandable
scan notices. It does not complete the broader browser/setup experience or remove
rich-document decoder limits.

Verification of the initial limits/scan-notice checkpoint: `cargo xtask ci` passed:
**578 tests passed, 5 opt-in tests skipped**. New regression tests first reproduced
the large UTF-8, seventeenth-folder, deep-folder and database-page restrictions.
Coverage now includes a source beyond 20 MiB, a match at its end, refresh/reopen,
cancellation inside the replacement transaction, split Unicode and long lines,
invalid/truncated input, crossing the former scan counters, and 70 directory levels.
The scan-notice controls are tested at 640×420, 2560×1440 and 200% interface size
in both themes. Counter-boundary tests establish removed cutoffs, not a performance
benchmark over 100,000 real files or a gigabyte database. The existing dev watcher
remains running; no alternate application process was started manually.

Folder details now offer persisted hidden-file and generated-folder inclusion
settings. Changes refresh the folder; newly excluded files leave current search
immediately while saved history remains available. Credential folders and Sovra's
own index storage stay excluded.

The inventory is now exposed directly in Knowledge: browse all folders or one
folder, filter filename fragments and extraction status, follow cursor pages,
inspect extraction caveats, and read a literal saved-text excerpt. Metadata-only
and failed files remain visible without matching a content keyword. Listing and
preview requests run independently of scans, carry correlated success/error
responses, and discard stale results after filter changes or scope revocation.
The preview is explicitly an excerpt of saved text, not a whole-file or live view.
This browser checkpoint passes `cargo xtask ci`: 586 tests passed, 5 opt-in tests
skipped. Tests cover stale responses, scope removal/reconfiguration, inventory
and preview errors during a held scan, native paging/filter/preview actions, and
minimum/wide/200% layouts in both themes. Native GPU pixel fidelity has not yet
been checked for the new browser.

Large-text follow-up: UTF-16/32 now use the bounded streaming path too, with
strict decoding and explicit encoding notes. The regression first reproduced
metadata-only treatment above 20 MiB, then verified both byte orders, BOM-less
UTF-16 inference, end-of-file retrieval, reopen/reuse, one-byte reads, split
surrogate pairs, malformed sequences and cancellation during replacement.
Decoding expansion is covered with a source below 4 MiB whose decoded text exceeds
4 MiB. All 82 file-adapter tests and `cargo xtask ci` pass at this checkpoint:
590 tests passed, 5 opt-in tests skipped. Legacy encodings and rich-document decoder
budgets remain open work.

Full scan reporting now persists every recorded skip, metadata-only extraction
and read failure, replacing the previous latest-report rows on a new scan.
Knowledge exposes path/reason filters, category filters and paging beyond the
50-example summary. Cancelled/interrupted/failed scans remain explicitly incomplete;
pre-upgrade folders are asked to refresh before a complete report is available.
Pages capture a scan and an upper sequence bound, reject new-scan/filter mismatch,
and flag newly recorded entries when navigating an older snapshot. Excluded-only
walks now publish progress and observe cancellation between batches too.
This checkpoint passes `cargo xtask ci`: 599 tests passed, 5 opt-in tests skipped.
Coverage includes 94 notices across pages/reopen, filtering and stale-page rejection,
partial and failed scans, live-scan snapshots with newer-entry flags, schema 8/7
migration without losing citations, distinct escaped non-Unicode filenames,
Client responsiveness, native report controls and both themes at minimum, wide
and 200% layouts. The native light-theme report was also inspected in the running
development app: paging/filter controls, live-scan labels and actual failure
messages are readable at the desktop window size. Native dark/minimum/200% GPU
checks remain unverified; those layouts are covered by headless tests.

The native report exposed an ordinary PowerPoint package being misidentified as a
generic ZIP above 100 internal parts. A synthetic 120-slide regression reproduced
the rejection before the fix. Container detection now reads package part names
without that cutoff, and Office/OpenDocument/EPUB extraction bounds the part count
by the already-bounded input size instead of 100. Expansion bytes, compression
ratios, text, page and nesting budgets remain enforced. Decoder revision `extract-v3`
causes older snapshots to be re-extracted on refresh while preserving historical
citations. Regression coverage includes Word/Excel packages over 100 parts, the
last presentation slide through index/reopen/reuse, and oversized declared expansion.
All 90 file-adapter tests pass; `cargo xtask ci` passes all gates with 602 tests
passed and 5 opt-in tests skipped. The native development watcher remains running.

Legacy text now uses the same atomic streaming path, with `stream-text-v3` snapshots.
A bounded full-source detection pass follows a failed Unicode attempt, handling
long ASCII headers and ISO-2022-JP escapes across read boundaries. Strict decoding
never substitutes characters for malformed sequences in the selected encoding.
Guesses remain explicitly labelled; encoding inference is not certainty.
Regression coverage includes Windows-1252 beyond 20 MiB, Japanese/Chinese text,
threefold Thai expansion below 2 MiB of source, one-byte reads, incomplete final
characters, and cancellation during inference, validation and transaction writes.
`cargo xtask ci` passes every gate: 608 tests passed, 5 opt-in tests skipped.
The development app remains supervised by the existing xtask/Watchexec session.

Traversal recovery now continues through accessible sibling folders after metadata,
directory-open or directory-iteration errors. Each failed location receives a
persisted report entry and increments the failure count once. Partial walks finish
as Failed/incomplete, preserve the last completed time and skip missing-file
retirement; only a later complete scan can confirm removals. Knowledge explains
the possibility of older results and offers Retry scan. A Unix permission regression
checks two inaccessible subfolders, 24 updated siblings, preserved unvisited/deleted
snapshots, reopen and recovery; headless controls and both-theme minimum/wide/200%
layouts exercise the incomplete-state guidance.
`cargo xtask ci` passes every gate: 609 tests passed, 5 opt-in tests skipped.

Document preparation now distinguishes folder indexing from chat attachments.
The index's text-only mode removes the 500-page/slide cutoff, preserves short PDF
text layers, and records no-text PDF metadata with OCR guidance instead of the
12-picture attachment error. It avoids unused page rasterization and media payload
allocation. `extract-v4` refreshes prior representations without relabelling history.
Regressions cover first/last text in 601-page PDFs and 601-slide presentations,
reopen/reuse, a 20-page no-text PDF, short text and actual scanned-PDF media handling.
Attachment limits and document byte/expansion budgets remain enforced. The no-tokio
Xberg build does not enforce its configured timeout; a hard parser deadline remains
open work.
All 99 file-adapter tests pass. `cargo xtask ci` passes every gate: 611 tests passed,
5 opt-in tests skipped. The existing development watcher remains running.

Document reads now register a cooperative cancellation signal with the runtime.
Cancel, Pause and shutdown reach a busy decoder; reconfiguration uses the same
interruption path. A registration belongs to one decode and is released afterward.
Documents inside archives receive the same signal. Cancelled extraction keeps the
previous searchable version, without a failed-file report or partial replacement.
Checks inside individual decoder formats determine how quickly they stop; this is
not a forced termination guarantee. Regressions cover PDF replacement/retry,
archived PDFs, dropped registrations, and real Client Cancel/Pause/shutdown handling.
`cargo xtask ci` passes every gate: 614 tests passed, 5 opt-in tests skipped.

Reader warnings now survive the complete extraction/index/UI path. A synthetic
presentation with a missing second slide reproduced the old false assurance;
readable first/last slides now retain a **Partial text** status and a note naming
the missing slide. The same works inside an archive. Knowledge offers matching
file/report filters, while preview text stays separate from diagnostic messages.
Archive media/inspection-only entries and BZip2's first-stream restriction also
mark partial coverage. This reflects known reader limitations, not a guarantee
that a reader detects every omission. PDF image/OCR limitations remain explicit.
Schema 10 retains old report records and adds per-representation partial status;
`extract-v5` regenerates earlier representations without changing saved history.
Regressions cover filters, text counts, citations, restart/reuse, repaired sources,
version history, migrations from schemas 7/8/9, and UI controls/layouts in both themes
at minimum/wide windows and 200% interface size.
`cargo xtask ci` passes every gate: 616 tests passed, 5 opt-in tests skipped.
The running native light-theme app also shows the new filter, partial rows and
reader explanations against its real catalogue. Native dark/minimum/200% rendering
remains covered by headless checks rather than a fresh GPU inspection.

Document reading budgets now belong to folder options, independent of chat's
20 MiB source and 4 MiB text ceilings. The UI exposes three memory choices and
explains source, text and unpacked-byte limits; existing folder JSON defaults to
Standard. `extract-v6` and budget-aware version identities prevent cross-budget
reuse. Changing only a budget avoids the unrelated inclusion-policy inventory
walk. A valid 65 MiB PDF is searchable; Lower memory keeps metadata with a clear
limit explanation, and restoring Standard reuses the complete saved version.
A DOCX with over 16 MiB of text retains its last passage under Standard, while the
lower budget discloses the reader's limit instead of presenting a complete read.
The library can wrap its own resource errors in a generic parsing error, so those
reader details are preserved too. Indexed archives accept more than 100 members
within their total-byte budgets. Large source and archive reads check cancellation
between blocks; source reads publish progress. Tests cover source/text budgets,
reopen, restrictive transitions, recovery, archive end markers and cancellation
preserving prior text, alongside real controls and responsive layouts.
The 200% layout check caught an overflowing budget label; shortening it kept the
full limits available in the explanatory text. `cargo xtask ci` passes every gate:
621 tests passed, 5 opt-in tests skipped. The running native light-theme app
also shows the three choices, selected Standard budget, and full per-document
limits in Folder settings.

Knowledge now opens directly to Files, with separate Memories and How it works
sections. Navigation stays above the scrolling content. Browsing saved files and
reading a scan report each open a focused view with Back to folders; switching
sections retains browser selections. Folder settings show three saved notice
examples and link to the full report. Add a folder and local search precede folder
details. Regression checks cover section switching, preserved previews, closing
reports with late responses, and navigation at minimum/wide windows and 200% text
in both themes. The minimum-window test caught a clipped Add a folder action at
200%; reducing the content's top spacing fixed it. `cargo xtask ci` passes every
gate: 623 tests passed, 5 opt-in tests skipped. This navigation change has headless
layout coverage; a fresh native visual check of these sections remains outstanding.

Folder setup now accepts multiple folders in the native picker. One selection
travels as one queued command, so it is not split across the 16-operation mutation
queue. Registration completes for the selection before automatic scans start;
individual failures retain their paths/reasons and do not discard successes.
Duplicate paths/scopes are counted once. Existing scopes preserve their settings,
including paused updates. The Files page reports the result and offers Retry
failed folders; the draft remains intact when the picker is cancelled. Due scans
now run immediately between queued commands rather than waiting a second per
folder. Pause and Remove are still handled before the next automatic scan.

The picker regression reproduced the former single-selection restriction.
The real Client regression adds 24 folders plus an unavailable path and duplicate,
holds the first scan, checks Ping responsiveness, cancels/removes waiting folders,
then verifies every remaining folder scans and re-selection preserves pause.
Reducer coverage keeps addition failures separate from unrelated search messages;
headless checks exercise retry and layout at minimum/wide windows and 200% text
in both themes. `cargo xtask ci` passes every gate: 633 tests passed, 6 opt-in tests
skipped, including the concurrently integrated workspace changes. Native picker
selection is verified through GPUI's simulator and the pinned Linux portal code;
a fresh visual inspection of the resulting Files page remains outstanding.

Saved-text inspection no longer stops at the first passage. Catalogue rows report
the number of readable parts in their current version. The reader provides First,
Previous, Next and Last part controls, a saved-part position and line range, and
Retry this part after a failed read. Each request remains bounded to one stored
part. Source/partial-read caveats remain visible, text is literal, and parts are
explicitly distinguished from original document pages. Navigation preserves the
selected root/file/version reference; replaced or revoked versions fail instead
of mixing text. Late replies and replies for a different part are rejected.

The first regression reproduced the missing navigation. Storage checks rebuild a
multi-part Unicode document exactly through its catalogue references, reach its
last marker, reopen it, and reject old references after replacement/removal.
Core checks cover bounds, stale/mismatched results and retry targets. UI checks
exercise Next/Last/Previous/First, busy-reader retry, boundary/loading disabled
states, and control bounds at minimum/wide windows and 200% text in both themes.
`cargo xtask ci` passes every gate: 636 tests passed, 6 opt-in tests skipped.
The native app remains supervised by the existing watcher; fresh GPU inspection
of the saved-text reader is still outstanding.

Filename and scan-report filters now ignore capitalization by default, using
canonical Unicode normalization and case folding while preserving original paths.
Match case restores exact matching. Literal punctuation stays literal, and changing
the matching mode resets pagination. The model-facing catalogue schema exposes
the same choice and the existing Partial text state. The implementation uses the
pinned caseless 0.2.2, unicode-normalization and rusqlite 0.39 scalar-function APIs.

Text, catalogue, reviewed-knowledge and summary queries no longer reject more
than 16 explicitly selected folder IDs. Folder grants and ID validation still
apply, as do independent page, traversal and model-request budgets. Regression
tests first reproduced the Unicode filtering failures and the 16-folder rejection;
real storage tests now search 24 selected folders while excluding a 25th. Tests
also cover canonical-equivalent accents, case folding, exact matching, literal
punctuation, pagination and mismatched cursors. UI checks exercise both matching
controls at minimum/wide windows and 200% text in both themes. `cargo xtask ci`
passes every gate: 639 tests passed, 6 opt-in tests skipped.

A further regression reproduced binary bytes named `notes.txt` being classified
as Partial text. Xberg 1.3.10's plain-text extractor accepts bytes rejected by our
strict decoder, replacing invalid UTF-8. Its typed Text format result now requires
strict source decoding too. Unsupported binary files retain metadata with explicit
inspection/conversion guidance; index preparation avoids building a discarded byte
inspection payload. Chat attachments retain labelled inspection and original bytes.
The `extract-v7` representation revision corrects cached results on refresh while
preserving history. Tests cover text-like, database-style and unknown filenames,
absence of binary fragments from search, report notices, reopening, reuse and
conversion to readable Unicode. All 111 file-adapter tests pass. Full
`cargo xtask ci` passes every gate: 640 tests passed, 6 opt-in tests skipped.

SQLite and binary property-list indexing now use folder document budgets rather
than attachment limits. SQLite indexing removes the 100-table, 10,000-row and
256-column overrides and its fixed 4 MiB output cap; native SQLite column/SQL
limits and the five-second progress deadline remain. Table enumeration and rows
are consumed incrementally. Reading-budget or query failures retain decoded rows
with Partial text status and recovery notes, separately from source text. Binary
cells and malformed UTF-8 are also disclosed. Property lists remove the indexing
event-count cap and use the chosen text budget while keeping 64-level nesting.
Both adapters observe cancellation during decoding; SQLite registers the runtime's
cancellation signal with its progress handler. No source database is modified.

Three regressions first reproduced the old cutoffs and truncated databases being
labelled Text. Checks now reach late rows/tables and a 300th column, read beyond
4 MiB and 100,000 property-list values, reopen partial results, increase the budget
to reach omitted rows, and preserve prior versions on cancellation. Attachment
row limits remain separately labelled; unread cells produce explicit warnings.
Pinned rusqlite 0.39 limits and progress-handler APIs were verified in local source.
The `extract-v8` revision refreshes older capped representations. Full
`cargo xtask ci` passes every gate: 645 tests passed, 6 opt-in tests skipped.

The folder overview now surfaces notices before Folder settings is opened. A
completed scan with notices says to review them and explains the Scan report
action. The prompt disappears when a later scan clears the notices; capped saved
examples are not presented as a total issue count. A regression reproduced the
missing prompt, then checked completed scans with unread notices, skips without
examples, clean scans and the report action through Client. Minimum/wide/200%
checks cover the callout in both themes. Full `cargo xtask ci` passes every gate:
646 tests passed, 6 opt-in tests skipped.

Current goal audit against the indexing UX request and the stated priority of
file limits and skipped content:

| Requirement | Current evidence |
| --- | --- |
| Discover folders and add several without queue-size surprises | Files-first Knowledge navigation, native multi-folder picker, failed-path retry, and the 24-folder runtime regression in `sovra-runtime/tests/file_index/batch.rs`. |
| Remove arbitrary collection and attachment limits from indexing | Storage/scan regressions in `sovra-files/src/index/tests.rs`, late-content tests in `documents.rs`, `legacy_text.rs` and `structured.rs`, plus selected-scope tests in `catalogue.rs`. Per-folder document budgets remain explicit. |
| Explain skipped, unread and partial content and how to recover | Complete report pagination/reopen/filter tests in `index/tests/report.rs`, partial and repaired-source tests in `documents.rs`/`structured.rs`, and the visible folder notice/report action in `file_index_view.rs`. |
| Find files and inspect all saved text | Literal Unicode filename/report filters, paging, extraction filters and version-bound text navigation; storage tests plus real control tests in `file_index_view/browser.rs` and `report.rs`. |
| Stay responsive and preserve work | Client tests keep Ping/search/pause/remove responsive during scans; cancellation and inaccessible-subfolder tests retain previous or unvisited versions. |
| Work across window sizes and themes | Headless controls/layout checks cover 640×420, 2560×1440 and 200% interface size in both themes. |
| Verify native pixels and navigation | Pending: the supervised native app is running, but XInput events go to the `i3lock` window rather than Sovra. The desktop owner was asked to unlock it; the lock was left intact. |

The live visual check remains outstanding. Native notification resource budgets,
codec expansion/nesting protections and documented parser limitations are distinct
from the arbitrary collection cutoffs removed here; this work does not promise
unlimited decoding or support for every file format.

### Implementation checkpoint: local folder index

`/index-folder` (aliases `/index-file-system` and `/index-files`) opens a native,
single-folder chooser. Cancelling preserves the draft. Knowledge displays each
scope's actual scan counters, errors, cancel/refresh controls, removal confirmation,
and local text search. Pause/Resume updates persists independently of manual
Refresh. Cancelling a scan also pauses automatic updates. Removing a scope deletes its file index and revokes
retrieval; it does not alter source files. Reviewed knowledge history retains its local
snapshots but permanently loses access links, including across a later re-grant.

The `sovra-files` adapter now uses SQLite FTS5 for lexical retrieval and immutable
source versions, with SHA-256 content IDs and FastCDC text chunks. It reuses existing
versions after hashing unchanged files and preserves unambiguous Unix file identity
across renames. Every complete refresh reconciles missing paths. Cancellation,
failed traversal retain unvisited records; interrupted scans can be
refreshed after reopening. Indexed text is a snapshot, not a claim of live freshness.
Search and citation reads exclude superseded versions and revoked roots.

`/file-catalog` (aliases `/query-files` and `/list-files`) lists granted scopes and
observed file metadata. It accepts literal filename/path filters, exact parent
directories, MIME prefixes, byte ranges, inclusive filesystem modification dates,
extraction status and text keywords. Filters combine before grouping by scope,
directory, extension or MIME type. Queries beyond 32 keywords are rejected rather
than silently dropping terms. Listings return at most 40 files; cursors bind to
the same filters and catalogue revision, and reject changes between pages. Grouped
results report truncation and require narrower filters to inspect further groups.
Counts describe observed entries and include scan coverage and unknown sizes;
filesystem timestamps do not establish dates of events mentioned in a document.
Files beyond the extraction limit and failed extractions retain metadata with an
explicit status and no readable passage. A transactional schema migration preserves
existing citations and leaves legacy byte sizes unknown until the next refresh.
The extraction revision is part of each source-version key. Refreshing upgrades
older extraction results; blank files and binary inspection descriptions never
become searchable source passages in the current revision.

`/file-history` (aliases `/file-provenance` and `/file-versions`) adds observed
lineage through `inspect_file_history` and `read_file_history`. Each scan is an
activity with start/completion times, status and extraction revision. File
observations append immutable path, physical identity, size, filesystem dates,
presence and extraction state, with a predecessor link. Unchanged scans reuse the
last observation. Extracted representations retain their own content digest,
generation activity, first-indexed time and chunk membership. These IDs distinguish
a tracked file, a physical filesystem object and a particular text representation;
they do not themselves identify one logical document across different formats. The
reviewed knowledge layer below can explicitly ground multiple representations in a
logical document, with evidence and uncertainty.

Observation insertion and current file updates share a transaction. Only a completed
eligible walk records missing paths; a cancellation during reconciliation rolls
that absence transaction back. Missing means **not observed in the scope**, which
can reflect a policy exclusion or replacement as well as deletion. Interrupted
activities remain labelled. History pages contain at most 20 observations and use
monotonic sequence cursors, so newer observations do not shift older pages.
Historical text reads use exact observation references, report the observed path,
and provide a next-passage reference. Current reads still reject superseded sources.
Both require the original folder grant to remain present.
Current scan coverage accompanies history, so retained observations do not imply
that the most recent scan completed successfully.

The v4 migration records only the legacy state it actually has. Imported observations
have an explicit legacy tier, unknown observation time/confidence and no invented scan activity;
earlier text versions without observations are counted as a provenance gap. Fresh
filesystem observations have a separate evidence tier and confidence in the
observation itself. Recorded/observed timestamps remain distinct from optional
real-world validity, which is unknown for filesystem observations. No event date
inside a document is inferred from file modification time. This is an observation
history and extracted-text store, not complete historical reconstruction or a
backup of original bytes.

Foreign-key lookup indexes cover observation predecessors, scan ownership and
representation generation. Revoking a large scope can use these indexes while
removing its derived lineage; it need not repeatedly scan every child table for
each parent row. The follow-up v5 migration adds these indexes to existing histories.

`/search-files` activates the same tool-backed procedure used by model discovery.
`list_indexed_folders` discovers scope IDs, and `query_indexed_files` narrows source
selection before `search_indexed_files` and `read_indexed_file` retrieve evidence.
The runtime supplies an async reader bridge, keeping database IO off its executor.
Search returns at most eight passages per tool call; source reads require the opaque
version reference returned by search. The agent loop is tested with invalid queries,
stale references, source reads and final cited synthesis. Tool-disabled models cannot
acquire these tools through skill activation. The folder chooser explains that chat
retrieval sends requested file metadata and matching passages to the user's selected AI service.

Source access uses capability directory handles, no-follow opens at each traversed
component, and bounded reads. Defaults exclude hidden files, generated folders,
credential folders, symlinks and non-regular files. As of 2026-10-28, folder
registration and scans no longer impose the former 16-root, 100,000-entry, 1 GB
read/storage or 64-level traversal ceilings. An iterative depth-first walk holds
only the current branch's directory capabilities and checks cancellation between
entries. OS descriptor/path limits, SQLite's limits and available disk still apply;
this is not an arbitrary-scale performance guarantee. Native notification discovery
retains its own resource budget and falls back to periodic scanning.

Large text files use two streaming passes and immutable source chunks. Input
buffers are 64 KiB plus a three-byte carry; decoded output is bounded independently
of source size. UTF-8, BOM-marked UTF-16/32 in either byte order, and conservatively
inferred UTF-16 are supported, along with legacy encodings detected by chardetng.
Inferred encodings are labelled. Malformed sequences in the selected encoding
are rejected, never replaced silently; surrogate pairs and stateful encoding
escapes can cross input boundaries.
The first pass validates the entire text and hashes its original bytes; a new version's
second pass writes chunks in a transaction and verifies the same hash and file
metadata before publishing. A known Unicode version needs only the validation/hash
pass. Legacy text first makes a Unicode attempt and a bounded full-source inference
pass; inferred input is then validated/hashed and written through the same path.
Cancellation rolls back partial chunks and keeps the prior snapshot. Source text,
including markup, stays literal and is neither rendered nor executed. These files
are no longer subject to the 4 MiB attachment-text or 20 MiB attachment-file limits.
Rich-document codecs now use per-folder budgets: Standard defaults to 256 MiB
source and unpacked content / 64 MiB text, with Lower memory (64/16 MiB) and Large
(1 GiB/256 MiB) alternatives. Codec-specific protections remain. Oversized
non-streamable files keep metadata and an explicit explanation with a Folder
settings action. Chat attachment limits are separate and unchanged.

Scans run on an owned blocking worker; cancellation bypasses its queue and search
runs independently. The settings summary retains up to 50 notice examples; the
full Scan report persists and pages through all recorded notices, including
hidden/generated/credential exclusions, symbolic links, metadata-only files and
read failures. Reports use bounded write batches and display pages without a
total-entry cutoff. Migration to schema 9 preserves source citations and old
summary examples; complete reports start with the next scan. Selecting a directory does not count all children of
an excluded subtree as visited. Folder details expose inclusion options. The file
browser lists observed files, including partial-text, metadata-only and failed rows;
excluded subtrees and symlinks remain scan notices. Filename filters currently
match literal case-sensitive substrings. Pagination keeps one bounded page in
the UI; if the catalogue changes between pages, the user reloads the list.

Automatic monitoring uses notify's native backend while the app is open. It
registers non-recursive watches on at most 2,048 eligible directories per scope,
following each folder's hidden/generated inclusion choices and skipping symlinks.
Notification callbacks only set a
bounded dirty flag. Runtime coalesces bursts (1.5-second quiet period, 10-second
maximum delay), limits repeated scans to one per 30 seconds, and reconciles every
15 minutes even when no events arrive. Reopening/resuming reconciles immediately.
Watch failures or resource limits visibly fall back to periodic scans. A completed
scan refreshes registrations to cover new directories; notifications during a scan
remain pending. Source reads still pass through capability-based traversal.
Closing the app marks ongoing work interrupted and preserves the update preference.
If saving Pause fails, monitoring still pauses for that session and the UI reports
that the setting could not be saved for reopening.

Verification checkpoint: `NEXTEST_TEST_THREADS=2 cargo xtask ci` passed all gates
and 349 tests on macOS. Summary coverage includes source/ancestor invalidation,
permanent grant revocation, durable request budgets, pause/resume races, exact cache
reuse, immutable history, scope boundaries, hierarchy limits, Unicode literal search
and schema-7 migration. Reviewed knowledge coverage includes pending/accepted/rejected
state, duplicate submissions, current citation checks, grant revocation across
re-grants, cross-root identity constraints, historical query filters, supersession,
atomic cycle rejection and order-independent hierarchy replacement. Native review
requires a click and remains reachable in both themes at minimum/wide/200% sizes.
Semantic coverage includes both HTTP protocols, explicit
folder consent, changed destinations, cancellation during a response, saved Ollama
capability discovery, vector validation/dimension consistency, transaction rollback,
current-version citation filtering, and compact/wide/200% native controls in both
themes. History coverage includes unchanged-scan reuse, renames,
content transitions, absence after a complete scan, cancellation, interrupted
activity recovery, pagination during new observations, scoped historical reads,
grant revocation and legacy uncertainty. The fifteen-step ADK loop discovers retrieval and mapping
skills, reads current and historical sources, rejects injected approval arguments,
creates a pending proposal, verifies that it is absent from saved knowledge and
returns a cited comparison.
Catalogue coverage includes literal filters and exact
directories, grouped counts, extraction states across refreshes, revision-bound
pagination, concurrent legacy migration with preserved citations, bounded folder
discovery, rejected malformed tool arguments and the full ADK discovery/query/
search/read/synthesis loop. Other coverage includes native nested-folder notifications,
fallback/pause/removal through the runtime Client, interrupted-scan recovery,
failed preference writes, late results after scope removal, and native controls
at minimum/wide/200% sizes. This validates the checkpoint, not the remaining
acceptance items or performance claims.

### Semantic retrieval implementation

Knowledge offers an explicit embedding-model choice per folder. Its approval record
binds the model key, provider protocol, normalized server URL, organization/project
and available catalogue modification revision. Models and credentials are reloaded
before each batch/query; an edited destination or revision fails closed until the
user chooses again. Neither the chat model nor a newly added folder implicitly
enables bulk embedding requests. Only Ollama `/api/embed` and OpenAI-compatible
`/embeddings` are supported here. OpenRouter and Anthropic are not advertised as
embedding transports. Compatible servers with unknown capabilities may be chosen
explicitly; known chat-only models are excluded. Models requiring document/query
prefix templates are not yet supported. No downloads are initiated.

SQLite schema 6 stores consent generations and per-generation/chunk float32 vectors.
Each background pass sends at most eight complete extracted passages, with a 2 MB
response cap, 30-second runtime deadline and no automatic retry loop after failure.
Ollama receives `truncate:false`. Responses must contain exactly one finite,
nonzero, bounded-dimension vector per input; compatible indices restore input order
and reject duplicates/gaps. A generation locks its dimensionality on the first
batch. Storage checks both consent and current chunk text before committing, rejects
late/revoked work, and prunes vectors for superseded chunks. The next app launch
resumes incomplete indexing; failures can also be retried by rebuilding the folder's
embedding settings. Rebuilding intentionally discards that generation's vectors.

Search always runs the keyword leg and adds exact cosine-ranked passages from
approved folders, then fuses ranks with RRF (k=60). Semantic requests have a shared
10-second deadline; SQL/ranking work has five-second bounds. Final citations are
resolved again against current versions and grants after network work. UI and ADK
responses carry incomplete-coverage/failure notices; semantic similarity is retrieval
evidence, not factual confidence. Disabling a folder cancels its active request and
deletes its derived vectors; text already delivered to a server cannot be recalled.

Current limits: 4,096 dimensions, 20,000 unique embedded passages per folder, the
available local disk, and exact linear ranking rather than an
ANN index. Server-side replacement of a model under an unchanged name/revision
cannot be detected reliably; use immutable model identifiers and rebuild after such
a change. Tests use deterministic synthetic vectors and local HTTP servers. They
verify protocol, ranking, citations, cancellation and consent, not real-model recall,
embedding quality or production latency. Real-corpus benchmarks remain pending.

This semantic-retrieval checkpoint does **not** implement salience, summary generation,
or structured file mutations/rollback. Those remain in the acceptance
ledger above. Filesystem identity is read from open handles through cap-fs-ext; Windows/ReFS and Linux
verification is still required. The upstream 64-bit Windows identity API does not
establish complete ReFS identity guarantees. A full scan hashes bytes to detect same-size changes;
it is incremental extraction, not a journal-based scan.

### Summary storage foundation

Schema 8 stores immutable derived summaries with exact direct inputs and a
deduplicated closure of raw source passages. Claims cite direct input indices;
records retain model destination, prompt/storage revision, generation activity,
extraction caveats and unknown factual confidence. Nodes can have multiple parents
without counting a source twice. A parent may refer only to already stored, current
nodes in its own approved configuration. Inputs are limited to 16 and 64 KiB,
output to 6,000 bytes, and hierarchy depth to 64.

Source version, path and availability changes invalidate every affected ancestor
in the same index transaction. Explicit derived corrections retire their ancestors
without erasing history. Ordinary reads exclude stale results; historical reads
must request them. Grant revocation disables the entire approved configuration,
including when the removed scope had not contributed a passage, and nulls normalized
links permanently. Re-granting a folder cannot restore those old records.

Summary requests reserve durable budget before a model call. Failed/interrupted
attempts consume budget, exact cache hits do not, and pausing cancels pending attempt
tokens so a later resume cannot accept their responses. Commit revalidates input
text, references, configuration and freshness within its transaction. Queries use
bounded pages, literal Unicode-lowercased text, explicit scope coverage, and a
five-second SQL budget. The existing database size ceiling still applies.

Background model selection now shares catalogue-first destination resolution with
embedding requests. Incomplete manual metadata cannot override a catalogue's known
capability restrictions. Credentials remain resolved separately at execution time.

This is persistence infrastructure, not a working background generator. No new
summary action or automatic model request is enabled. Recursive generation/re-embedding,
collapsed-tree retrieval, compiled Markdown maps, scheduler and native consent UI remain open.
The stored algorithm revision identifies this DAG format and does not claim RAPTOR.

The separate clustering engine now performs global UMAP with a square-root-sized
neighborhood and local UMAP with ten neighbors (both adjusted for small batches).
Exact cosine neighbors include each point itself first. Reduced vectors feed a
full-covariance Gaussian mixture: EM uses Cholesky solves, stable log-sum-exp,
diagonal regularization, deterministic farthest-point initialization and bounded
iterations. It compares admissible component counts with full-covariance BIC.
Posterior thresholds retain overlapping groups; the strongest component retains
an input when none exceeds the threshold. These weights are not factual confidence.

Oversized groups recursively repeat global/local clustering. Non-shrinking groups
and the recursion limit use a reported, balanced embedding-distance bisection.
Coincident embeddings do not produce invented statistical clusters. Every source
must remain covered; each final group must satisfy both input-count and byte limits.
The byte budget includes source labels/extraction notes and is not a model-token
estimate. Model requests additionally pass the context admission described below.

The adapter owns a single Rayon worker and checks cancellation between nearest-neighbor
rows, UMAP epoch batches and EM fitting steps. Current ceilings are 512 inputs,
16 reduced dimensions, 32 candidate components, 256 reductions, 2,048 output groups
and a 20-second computation budget. Defaults are smaller where appropriate. These
are explicit build-stage bounds, not demonstrated whole-corpus scalability. The
runtime scheduler and larger-corpus staging still need integration. UMAP's internal
negative sampling remains stochastic, so repeated builds are not promised to be
bitwise reproducible; exact persisted summary cache keys remain separate.

UMAP 0.4.5 is pinned. The available Linfa 0.8.1 package requires `sprs =0.11.2`,
which conflicts with UMAP's `sprs ^0.11.3`; the core therefore implements the bounded
full-covariance EM step directly without adding a numerical dependency to the pure
domain. Tests compare correlated Gaussian density and fitted likelihood against
analytic values, verify mixture selection on synthetic populations, and exercise
the actual UMAP/global/local pipeline, cancellation, soft overlap and budget fallbacks.
This does not establish real-corpus retrieval quality or production throughput.

The summary HTTP adapter now supports Ollama chat, Anthropic Messages, OpenRouter
and compatible chat completions. Runtime must construct a fresh adapter on its
owned blocking task immediately before each reserved request, which revalidates
the approved destination and reads credentials. Each instance allows one HTTP
attempt, including failed or cancelled attempts; it cannot silently retry against
an old destination. Prepared payload validation binds configuration, scope and
ordered references and counts labels and extraction notes toward the 64 KiB limit.
The adapter does not itself establish storage freshness or reserve budget: those
remain required scheduler operations before any production use.

Requests carry source text and extraction notes as data, provide no tools, and
ask for claims with zero-based input citations. Response parsing requires a natural
completion, rejects tool calls/refusals/truncation, and strictly validates JSON
fields, citation indices and the 6,000-byte text ceiling. HTTP bodies are bounded
to 128 KiB and server error bodies never enter error messages. Shared transport
also retains the embedding response ceiling of 2 MiB and disables redirects.
OpenRouter requests explicitly disable the documented web, file-parser,
response-healing and context-compression plugins, including account defaults.

Context admission requires advertised context metadata and conservatively charges
one unit per serialized source/system byte plus 1,024 units for framing and the
requested output budget. This is not exact tokenization or proof of a server's
runtime capacity. The usable context is capped at 65,536; Ollama receives that
context setting explicitly. Output requests are bounded to 4,096 tokens and any
smaller advertised maximum. Oversized groups fail before HTTP rather than losing
source text. Unknown context metadata fails closed. Provider parsing and local
HTTP tests cover the transport contract. The three core summary tests, five
provider protocol tests and four local HTTP tests passed. The latest protocol
and HTTP tests were compiled with the pinned Rust compiler and dependency
artifacts matched to Cargo's fingerprints: Cargo could no longer read a dehydrated
workspace manifest under disk pressure. The HTTP harness reused the actual shared
fixture and summary test source, omitting unrelated runtime tests.

The workspace CI attempt passed formatting, Clippy, layering, styling, dependency
policy and unused-dependency checks before test linking failed with
`ld: write() failed, errno=28 (No space left on device)` in the store integration
test. The final OpenRouter plugin guard and fixture correction passed focused
validation afterward. These changes are not included in the 349-test full-CI
checkpoint above; another complete run is still required.

Protocol sources: [Ollama chat](https://docs.ollama.com/api/chat),
[Anthropic Messages](https://platform.claude.com/docs/en/api/messages/create), and
[OpenRouter API reference](https://openrouter.ai/docs/api/reference/overview) and
[context compression](https://openrouter.ai/docs/guides/features/message-transforms).

Design sources: [RAPTOR](https://arxiv.org/html/2401.18059v1) describes the actual
recursive clustering algorithm; [GraphRAG's default dataflow](https://microsoft.github.io/graphrag/index/default_dataflow/)
distinguishes source text units, extracted entities and derived community reports.
Neither establishes quality or performance guarantees for this implementation.
Additional implementation references are [UMAP's pinned API](https://docs.rs/umap-rs/0.4.5/umap_rs/struct.Umap.html)
and [Gaussian mixture estimation](https://scikit-learn.org/stable/modules/mixture.html),
which describes full covariance, EM fitting and model selection.

### Reviewed logical knowledge

Schema 7 adds an append-only knowledge layer. `/map-files` proposes source-grounded
entities and typed relationships; `/file-knowledge` queries them and reads evidence.
The vocabulary distinguishes logical documents, sections, topics, people,
organizations, projects and tasks. Relationships are grounded-in, part-of, about,
for-project, depends-on, related-to and revises, with explicit domain/range rules.
Logical IDs must be reused explicitly. Similar names or byte hashes do not merge
entities automatically. Part-of edges are acyclic; a rejected batch commits nothing.

A proposal contains at most eight current extracted passages, sixteen new entities
and twenty-four relationships. It records its originating helper/session/invocation,
the exact structured changes, full evidence snapshots, uncertainty, optional world
validity, and prior records it would supersede. Knowledge renders this preview using
native controls. Only the UI's ReviewKnowledge command can approve it; the model
receives a pending response and can continue independent synthesis. There is no
approval tool or approval argument. Proposals survive reopening and duplicate
submission of the same draft/origin reuses the existing proposal.

Saving is atomic and rechecks current versions and every required grant. Approval
records permission to persist, never truth: relationships remain ModelInference
with optional uncalibrated confidence. Recorded time and real-world validity are
separate; unknown dates do not match a dated query unless explicitly requested.
Supersession closes the earlier transaction-time interval without deleting its
content. Review state and time remain with each proposal; relationship results carry
their creator, proposal ID and original evidence IDs.

Queries traverse one to three hops and return at most forty relationships, with a
thousand-candidate scan cap, five-second work budget and explicit truncation. They
check both relationship citations and endpoint identity grants. Entity labels retain
their creation evidence; each relationship's freshness follows its own citations.
Changed or unavailable source versions require include_stale, superseded claims
require historical recorded time or include_superseded, and evidence reads expose
current=false. Part-of acyclicity includes stale but still-granted logical edges:
source freshness alone does not remove a logical parent. Revoked edges are excluded.

Removing a folder nulls evidence foreign keys permanently and rejects dependent
pending proposals. Re-adding the same path cannot restore those old links. Local
review snapshots remain retained but inaccessible through the UI/tools. There are
at most eight pending proposals and ten thousand retained proposals, within the
existing database budget. This phase does not implement bulk graph visualization,
automatic entity resolution, salience, summary hierarchies, file mutation or rollback.

The vocabulary and provenance model are application adaptations of
[W3C PROV-O](https://www.w3.org/TR/prov-o/) and
[PIMO's logical entities and grounding occurrences](https://www.semanticdesktop.org/ontologies/2007/11/01/pimo/).
They do not claim full RDF/OWL conformance. The 349-test CI checkpoint above covers
this phase. The tests establish protocol, storage and headless UI behavior; realistic
corpus quality, cross-platform evidence and the remaining acceptance items are still
open.


Additional semantic retrieval references:

- [Ollama embedding API](https://docs.ollama.com/api/embed): batched text inputs,
  vector arrays and explicit rejection of over-context input with `truncate:false`.
- [OpenAI-compatible embedding wire format](https://developers.openai.com/api/reference/resources/embeddings/methods/create):
  float output, input limits and indexed result entries. Endpoint compatibility is
  checked with local protocol fixtures; it does not certify every third-party server.

- [SQLite foreign-key indexing](https://www.sqlite.org/foreignkeys.html#fk_indexes):
  child-key indexes avoid repeated full child-table scans when parent rows are removed.
- [W3C PROV-O](https://www.w3.org/TR/prov-o/): entities, generation activities and
  derivation motivate explicit representation/activity/predecessor links. These
  are typed native records; this checkpoint does not claim full RDF/OWL conformance.
- [NEPOMUK File Ontology](https://www.semanticdesktop.org/ontologies/2007/03/22/nfo/):
  physical file data and interpreted information are distinct. A file identity or
  byte digest alone cannot establish that two formats express the same logical document.
- [SQLite snapshot isolation](https://www.sqlite.org/isolation.html) and
  [row-value pagination](https://www.sqlite.org/rowvalue.html): each catalogue
  response reads counts, coverage and entries in one transaction. Following a
  cursor uses the last ordered key and additionally checks Sovra's revision to
  detect changes between separate requests.
- [notify 8.2.0](https://docs.rs/notify/8.2.0/notify/): native backend limitations,
  dropped/coalesced events, network filesystems and watch resource ceilings. Native
  event ingestion is exercised on macOS; other platform backends remain unverified.
- [SQLite FTS5](https://www.sqlite.org/fts5.html): transactional external-content
  indexing and BM25 ranking. Owned triggers keep chunk content and FTS consistent.
- [FastCDC v2020](https://docs.rs/fastcdc/5.0.0/fastcdc/v2020/index.html): content-defined
  chunk boundaries; UTF-8 boundaries are adjusted before storing text passages.
- [cap-std directory capabilities](https://docs.rs/cap-std/4.0.3/cap_std/fs/struct.Dir.html)
  and [no-follow extensions](https://docs.rs/cap-fs-ext/4.0.3/cap_fs_ext/trait.DirExt.html):
  scope-relative filesystem operations without unsafe application code.

### Earlier research

Primary sources checked 2026-10-25:

- [Agent Skills specification](https://agentskills.io/specification) and
  [client integration](https://agentskills.io/client-implementation/adding-skills-support):
  metadata discovery, explicit/model activation, progressive disclosure and bounded
  resources. Installed instructions do not expand runtime permissions.
- [Apple FSEvents guide](https://developer.apple.com/library/archive/documentation/Darwin/Conceptual/FSEvents_ProgGuide/UsingtheFSEventsFramework/UsingtheFSEventsFramework.html):
  event streams require reconciliation; dropped/coalesced events cannot establish an
  infallible journal of all file contents.
- [RAPTOR paper](https://arxiv.org/abs/2401.18059): recursive summaries are derived
  knowledge. Its benchmark improvements are not guarantees for this application's data.
- [Kuzu repository](https://github.com/kuzudb/kuzu/): archived October 2025. The
  supplied reference's library/lifetime examples are design suggestions, not code to
  copy. Existing embedded storage and supported adapters must be evaluated first.

The attachment's absolute performance/accuracy claims (millions of files in seconds,
100% historical reconstruction, sub-millisecond arbitrary queries) remain hypotheses
until reproduced. NTFS, ReFS, APFS and Linux filesystem journals are different APIs;
an implementation must not claim one raw-table parser covers them all. Existing safe
Rust, dependency, local-data and styling rules remain binding.
