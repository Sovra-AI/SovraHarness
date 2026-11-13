# File understanding and live folder reading

How Sovra decides **what a file is**, **what it is about**, and **what a person sees while a
folder is being read**.

Code: `crates/sovra-core/src/file_index/ontology.rs` (taxonomy),
`crates/sovra-core/src/file_index/live.rs` (wording and estimates),
`crates/sovra-files/src/index/scan*` (parallel reader), and
`crates/sovra-ui/src/file_index_view/live.rs` (panel).

## Layered model

| Layer | Question | Stored as | Standard it maps to |
|---|---|---|---|
| Format | What bytes are these? | IANA media type per saved version (`index_versions.mime`) | [IANA media types](https://www.iana.org/assignments/media-types/media-types.xhtml) |
| Kind | What sort of thing is it for a person? | `FileKind` (15 concepts) | `skos:broader` → [DCMI Type Vocabulary](https://www.dublincore.org/specifications/dublin-core/dcmi-type-vocabulary/); `skos:closeMatch` → [schema.org](https://schema.org/CreativeWork) and [NEPOMUK NFO](https://www.semanticdesktop.org/ontologies/2007/03/22/nfo/) / NMO |
| Life area | What is it about? | `LifeArea` (11 concepts) | `skos:closeMatch` → [IPTC Media Topics](https://cv.iptc.org/newscodes/mediatopic?lang=en-GB) where a genuine match exists |
| Relationships | How do files relate? | Knowledge graph (`file_index/graph.rs`) | [PROV-O](https://www.w3.org/TR/prov-o/) (`wasRevisionOf`, `wasDerivedFrom`), NIE `isPartOf` |

Each scheme is shaped like [SKOS](https://www.w3.org/TR/skos-reference/): one `prefLabel`
(the only name shown), `altLabel`s (search words), `broader`, and `closeMatch`/`exactMatch`
links to external IRIs. Kind follows Apple's Uniform Type Identifier idea of a conformance
chain: media type → kind → DCMI class, so a reasoner can walk upwards.

### Classification rules

1. The detected media type decides. `image/*` is a photo, `text/csv` a spreadsheet, and so on.
2. The extension decides only when the type is generic or missing (`application/octet-stream`),
   or when plain text is really code or data (`text/plain` + `.rs`, `.yaml`).
3. Whole words in the file name refine documents, sheets and pictures into **Bills, receipts &
   statements** or **Forms & certificates** ("Invoice 2024.pdf", "passport-renewal-form.pdf").
   Partial words never match ("billboard" is not a bill).
4. Life areas come from whole words in folder and file names ("Taxes 2024", "Holiday Spain").
   People already sort their lives this way, so it is an explainable first signal that needs
   no model. No cue means no guess.

All of this is pure, deterministic and unit-tested. Cue words cover English plus common
French, German, Spanish, Italian, Dutch and Portuguese terms, with and without accents.
Ambiguous words are left out on purpose (French *vol* is also "volume"; German *Rezept* is
both prescription and recipe), and compound words are not split ("Mietvertrag").

The taxonomy stores external identifiers and mappings in the source; it is not a
claim that external vocabularies have been copied wholesale. Review the actual
constants and dependency licenses when extending the taxonomy.

## Live progress (what a person sees)

| Rule | Why | Source |
|---|---|---|
| "Finding your files…" with an indeterminate bar while the walk counts, then "1,204 of 5,000 files read" with a determinate bar | Determinate when possible; never fake a percentage | [Apple HIG](https://developer.apple.com/design/human-interface-guidelines/progress-indicators), [NN/g](https://www.nngroup.com/articles/progress-indicators/) |
| Counts ("of 5,000 files") over bare percentages | Easier to understand | NN/g |
| Time left only after a warm-up, from a smoothed rate (EMA, α = 0.3), rounded generously ("About 4 minutes left", "Almost done") | Avoid Spotlight-style jumping estimates | [NN/g](https://www.nngroup.com/articles/progress-indicators/), [Eclectic Light on Spotlight](https://eclecticlight.co/2024/11/19/when-and-how-to-rebuild-spotlight-indexes/) |
| "What I've found so far" chips (icon + words + count), and "Your files seem to be about" | Waiting feels productive; mirrors Google/Apple Photos | [Google Photos](https://support.google.com/photos/answer/6193313), [Apple Photos](https://support.apple.com/en-gb/105081) |
| One calm line for problems plus **See why** (opens Scan report) | No flashing alerts | [Dropbox forum complaints](https://www.dropboxforum.com/discussions/101001014/is-the-time-left-to-sync-view-removed-again/791495) |
| Headline is a polite status region; the bar carries an accessible name and value | Screen readers hear phase changes, not every count | [MDN progressbar](https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Reference/Roles/progressbar_role), [MDN live regions](https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Guides/Live_regions) |
| Cancel stays available; reduced motion uses a static bar (handled by the kit's `Progress`) | WCAG 2.2.2 pause/stop | [WCAG 2.2.2](https://www.w3.org/WAI/WCAG22/Understanding/pause-stop-hide.html) |
| Updates at most about 10 per second | Smooth for eyes, cheap for the UI | [indicatif's 20 Hz cap](https://docs.rs/indicatif/latest/indicatif/struct.ProgressDrawTarget.html) as an upper reference |

## Parallel reader

```
walker (1 thread) ──bounded jobs (2×readers)──▶ readers (N threads) ──bounded outcomes──▶ writer (SQLite)
  finds + opens files                           read, hash, extract                      batched transactions,
  counts "discovered"                           skip known versions                      progress every 32 files / 100 ms
```

- **An early count.** A listing-only pass (no files opened) finds the total while reading is
  under way, so the determinate "1,204 of 5,000" bar appears almost at once instead of after
  the reading walk — which waits on the readers — reaches the end. The reading walk stays
  the authority for notices and completeness; both share one exclusion rule.
- **One walker.** Directory iteration is cheap next to extraction. Capability-relative,
  no-follow opens are kept; the bounded job channel caps open file handles.
- **N readers** = available cores − 1, between 1 and 8. Each has a read-only SQLite connection
  (WAL allows concurrent readers), so unchanged files skip extraction entirely.
- **Memory is bounded by bytes, not file count.** A reservation (the larger of source and
  extracted text) travels with each result until the writer has saved it, so readers, the
  channel and the batch share one 512 MiB budget. Output growth never waits (readers waiting on
  each other could deadlock); it may overdraw the budget briefly, which holds back new files.
- **One writer.** SQLite has one writer at a time; batching 32 results per transaction with a
  per-file `SAVEPOINT` keeps failures isolated. Notices are flushed before each progress event,
  so the Scan report always matches the counts on screen.
- **Large text files** (≥ ⅓ of the attachment limit, sniffed from their first 8 KiB) go to the
  existing streaming reader on the writer thread, which writes as it reads. Large PDFs, office
  files, media and archives stay on the parallel readers.
- **The writer is authoritative.** If a reader skipped extraction because a version looked saved
  but the writer disagrees (a rename earlier in the same batch), the writer extracts the kept
  bytes itself. A file that still fails is saved as Failed in this generation, so a completed
  walk never retires a present file. If a batch rolls back, its counters and notices do too.
- **Only a walk that reached its end** (no unreadable entries, not cancelled, not stopped early)
  may retire unseen files. Reader threads are joined explicitly; a reader crash is a normal,
  recoverable scan error.
- **Cancellation** is checked by every stage and every 64 KiB block; finished results are kept.
- **A decoder panic** on one damaged file is caught and becomes that file's "couldn't be read"
  notice instead of ending the scan.
- **Progress** is shared atomics sampled by the writer, never one event per file.

- **Unchanged files are not read again.** A file with the same path, filesystem identity,
  size and modification second as its saved, non-failed reading (same extractor revision and
  document budget) is confirmed from the index alone. As in git's "racy timestamp" rule, this
  only applies when the file was last modified strictly before the second it was last
  observed, so an edit made in the same second as a scan is always caught.
- **A writer failure stops extraction too**: extractors watch a combined stop signal, so a
  failed scan does not keep decoding files nobody will save.

The reader does not reduce scanning based on OS idle or battery state. Use Pause
when you want to stop automatic work. Cooperative cancellation is not a hard
execution deadline for every third-party decoder; see [Files](files.md).
