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
