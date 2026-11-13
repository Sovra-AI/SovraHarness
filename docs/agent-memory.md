# Memory evaluation

Optional tests measure memory behavior outside the ordinary CI suite. For the
current implementation contract, read [Memory capture and recall](memory-capture-and-recall.md).
For file and source-retrieval diagnostics, see [Retrieval benchmarks](knowledge-benchmark.md).

## Synthetic runtime benchmark

[The runtime fixture](../crates/sovra-app/tests/memory_lifecycle/performance.rs)
uses synthetic evidence, a deterministic loopback HTTP provider, and real disk
storage. It measures retrieval through `Client` at different corpus sizes and
concurrency levels. It makes no billable model requests.

```sh
SOVRA_RUNTIME_BENCH_OUTPUT=/tmp/sovra-runtime-benchmark \
cargo nextest run -p sovra-app --test memory_lifecycle --run-ignored only \
  -E 'test(synthetic_runtime_retrieval_measurements)'
```

Choose a fresh output directory. This test measures exact synthetic source
retrieval and runtime costs, not natural-language recall or answer accuracy.
Record the revision, machine, build profile, corpus sizes, and raw report when
comparing runs.

## Live selected-model evaluation

[The live fixture](../crates/sovra-app/tests/memory_lifecycle/live.rs) uses the
selected development connection and its credential with synthetic data in a new
directory. It does not use the existing chat/memory database or launch another
GUI. It can send multiple requests and incur provider charges.

First configure and select a model through a debug development launch. Then run:

```sh
SOVRA_MEMORY_EVAL_DEV_DATA=/path/to/development/app-data \
SOVRA_MEMORY_EVAL_OUTPUT=/tmp/sovra-memory-evaluation \
SOVRA_MEMORY_EVAL_SUITE=general \
cargo nextest run -p sovra-app --test memory_lifecycle --run-ignored only \
  -E 'test(synthetic_selected_model_evaluation)'
```

Replace the input path with your development app-data directory and use a new
output directory outside the checkout. The suite accepts `smoke`, `general`
(default), `temporal`, and `old-recall`. It reads development credentials through
a read-only adapter and keeps its test database and report separate.

Inspect `report.json`, including partial reports and failed extraction. A passing
test means the driver completed; it is not a semantic-quality pass. Review
answers, interpreted facts, omissions, corrections, timings, and withdrawals
against the synthetic expectations. Do not publish credentials, personal
connection metadata, or unreviewed reports.

These ignored tests are not run by default CI. This documentation does not claim
current benchmark scores; regenerate results for the revision being assessed.
