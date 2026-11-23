# Performance

How to measure Sovra's speed, find what is slow, and keep it fast. Measure before
and after every change; an optimisation without a number is a guess.

## Benchmarks

Benchmarks use [Divan](https://docs.rs/divan): attribute-based, quick to compile, and
it scales each sample until timer noise is negligible. They live in each crate's
`benches/` and build with the optimised `bench` profile (it inherits `release`).

```sh
cargo bench -p sovra-core --bench search          # all search benchmarks
cargo bench -p sovra-core --bench search -- models # only names containing "models"
```

`crates/sovra-core/benches/search.rs` times the searches that run on every
keystroke or redraw, at realistic sizes: 2,000 catalog models, 120 chats of 40
messages, 300 artifacts of about 4 KB. Compare medians from runs on the same
machine with nothing else busy. Benchmarks do not run in CI: shared runners are
too noisy to gate on, but `cargo xtask ci` still compiles and lints them.

When adding a benchmark:

- Build inputs outside the timed closure, and pass them through `divan::black_box`
  so the optimiser cannot skip the work.
- Use realistic data sizes and text (long descriptions, whole conversations), not
  tiny fixtures; the costs that matter grow with the data.
- Keep the pure logic in `sovra-core` so it can be benchmarked without a window or
  a runtime.

## Connected knowledge

On one Linux development machine, the existing synthetic folder benchmark produced
the following timings. The growth stage adds 9,000 short UTF-8 files to an index
of 1,000. The unchanged scan visits all 10,000. Identifier lookup used twenty
queries against the complete index. These are local observations subject to
filesystem cache and other work on the machine.

| Operation | Before | After tuning | Final repeat |
| --- | ---: | ---: | ---: |
| Grow the catalogue to 10,000 files | 5.615 s | 4.634 s | 5.208 s |
| Rescan all 10,000 unchanged files | 3.004 s | 1.845 s | 2.639 s |
| Identifier search, median | 0.955 ms | 0.976 ms | 1.005 ms |
| Identifier search, 95th percentile | 1.005 ms | 1.036 ms | 1.134 ms |

The first implementation took 6.325 s to grow the catalogue. Reusing prepared
statements in the indexing transaction brought this down to 4.634 s. English
word-form search shares stored source text with the literal-token index. An
additional language builds its postings on first use. Artifact search uses a
persistent BM25 index over current wording. [SQLite prepared statements](https://www.sqlite.org/c3ref/stmt.html) · [FTS5 storage and tokenizers](https://www.sqlite.org/fts5.html) · [SurrealDB full-text indexes](https://surrealdb.com/docs/learn/data-models/full-text-search/search-indexes)

A separate development-build test searched 100 local files, 100 personal notes
and 100 saved tables. Each query checked that all three sources returned the same
normalized measurement. After one warm-up, 20 different identifier queries took **2.25 ms
median and 2.77 ms at the 95th percentile**, from the client command to the
analyzed result event. On the same corpus, a common word-form query matching
all three collections took **32.9 ms median and 39.8 ms at the 95th percentile**
over 20 repetitions. Rendering, model inference and first-use language index
creation are outside that interval.

In the optimized core benchmark, a 64-point ring took 12.19 ms median after
incremental triangle enumeration, down from 30.47 ms for the first implementation.
A repeat during workspace verification measured 15.93 ms. The 16-point ring
measured 34.83–45.22 µs across those runs and the 64-row sensor table measured
154–183 µs. Larger clouds show the exact point window used. [GUDHI’s Rips filtration](https://gudhi.inria.fr/python/latest/rips_complex_user.html)

```sh
SOVRA_FILE_BENCH_OUTPUT=/tmp/sovra-file-run.json cargo test -p sovra-files --lib file_capture_and_search_measurements -- --ignored
cargo test -p sovra-app --test memory_lifecycle connected_source_query_latency -- --ignored --nocapture
cargo bench -p sovra-core --bench search -- knowledge
```

## Folder indexing

The on-disk benchmark calls the same folder scan as the app. It times discovery,
file reads, text extraction, SQLite and full-text indexing, history, and completion.
It checks source citations, replacement, renaming, deletion and search after reopening.
File creation and verification are outside the timer. To select a disk, set
`TMPDIR` to an existing directory on that filesystem.

```sh
SOVRA_SCAN_BENCH_FILES=10000 cargo test -p sovra-files --lib \
  --config 'profile.test.package.sovra-files.opt-level=3' \
  --config 'profile.test.package.sovra-core.opt-level=3' \
  end_to_end_folder_indexing -- --ignored --nocapture
```

Set the count to 200000 for the larger run. For native file-change notifications,
run the same command with `native_watch_registration_measurements` as the test
name. Each invocation measures five registrations on 221 directories with
20,000 files.

Five alternating runs per build on a Ryzen 7 3700X with an ext4 NVMe disk gave
the following median wall times. The baseline was `f02d3a8`. Both builds used
the same Rust profiles and generated fresh synthetic folders. The OS could cache
their newly written files, and a separate development app was indexing in the
background.

| Workload | Before | After |
| --- | ---: | ---: |
| First scan of 10,000 short text files | 4.616 s | 3.195 s |
| All 10,000 files unchanged | 1.240 s | 0.434 s |
| 1% edited, one rename, one deletion and one addition | 1.270 s | 0.466 s |
| Eight 20,000-line documents | 1.616 s | 0.583 s |
| Two 95,000-line streamed documents | 0.755 s | 0.644 s |
| Recheck those two streamed documents | 26.2 ms | 6.4 ms |
| 40 three-page PDFs, 40 presentations and 40 CSVs | 44.8 ms | 35.0 ms |
| Register native watches across 20,000 files (15 registrations per build) | 58.3 ms | 24.9 ms |

The writer now confirms unchanged versions against the live row, retains the
original change history and uses fewer statements for each new file. Both normal
and streamed passage extraction find headings in source order. The scan writer
can use up to 32 MiB of SQLite page cache on demand and groups small records into
larger transactions. FTS5 builds index segments per transaction, so batching small
writes reduces index merging work. Files whose modification second overlaps
their last observation still get a content check. [SQLite FTS5](https://www.sqlite.org/fts5.html#the_automerge_configuration_option) · [SQLite page cache](https://www.sqlite.org/pragma.html#pragma_cache_size) · [Git timestamp checks](https://git-scm.com/docs/racy-git)

## Profiling

`cargo build --profile profiling` builds a release binary with debug symbols and no
LTO, for perf, [samply](https://github.com/mstange/samply) or Instruments. On Linux,
perf needs `kernel.perf_event_paranoid` at 2 or lower for an ordinary user; that is
a machine-wide setting, so change it only on your own machine. Add
`RUSTFLAGS="-C force-frame-pointers=yes"` for cheaper, more reliable call stacks.

## Rules that keep it fast

These come from measured fixes (see the changelog entry "Searching is faster"):

- **Never copy text to compare it.** `text.to_lowercase().contains(query)` in a
  filter copies every chat, model description and document on every keystroke.
  Use `sovra_core::text_search::Needle` (the query lowercased once, texts read in
  place) and `cmp_ignore_case` for sorting.
- **Do not look up what you already have.** Filtering then calling a linear
  `model(&key)` for each result is quadratic; use `ModelsState::filtered_models`,
  which returns each match with its details.
- **Do not sort what a set will order.** Facets go into `BTreeSet`s, so they use
  the unsorted matches.
- **Render reads, it does not copy.** Views redraw on every event batch. Borrow
  shell state, or copy only what is drawn (`ArtifactsView::list_data`), never a
  whole library, contact book or catalog.
- **Return borrowed text when nothing changes.** `hide_memory_citation_tokens`
  returns a `Cow` and copies only replies that hold citation tokens.

| Benchmark (median, one laptop) | Before | After |
| --- | --- | --- |
| `models_facets` | 5.69 ms | 0.25 ms |
| `models_search` | 3.03 ms | 0.29 ms |
| `models_sort_by_name` | 2.96 ms | 0.12 ms |
| `history_search` | 3.60 ms | 0.28 ms |
| `artifacts_search` | 2.17 ms | 0.10 ms |

A frame at 60 Hz has 16.7 ms for everything, so each of these used to take a fifth
to a third of one on its own.

## Sources

- [The Rust Performance Book](https://nnethercote.github.io/perf-book/): benchmarking,
  profiling, heap allocations and build configuration.
- [RustTraining: benchmarking](https://microsoft.github.io/RustTraining/engineering-book/ch03-benchmarking-measuring-what-matters.html):
  why statistical harnesses beat `Instant::now()`, and CI noise.
- [Divan](https://nikolaivazquez.com/blog/divan/): the harness and its sample scaling.
