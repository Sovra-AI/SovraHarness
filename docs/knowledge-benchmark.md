# Retrieval benchmarks

The repository includes optional source-retrieval and file-index diagnostics.
They are separate from ordinary correctness tests and do not measure generated
answer accuracy. Keep data and reports outside the checkout.

## LongMemEval source retrieval

[prepare_longmemeval.py](../tests/evaluations/prepare_longmemeval.py) pins and
checks the upstream corpus revision and SHA-256, then prepares a selected subset.
The default is a small smoke subset; `--per-category 500` selects the full set.
The script downloads public corpus data unless `--dataset` supplies a local copy.

[The store diagnostic](../crates/sovra-store-surreal/tests/store/retrieval_evaluation.rs)
measures source-ID recall through the real store and context budget. It excludes
model extraction, embeddings, runtime scheduling, disk durability, and generation.
It scores labeled user evidence; imported assistant turns lack certified provider
input lineage. Unscorable and abstention cases are reported separately.
Returning the right source ID does not prove that its bounded excerpt contains
the answer. The application also supports reformulated lookup batches; this
single-query diagnostic does not evaluate that complete path.

```sh
python3 -m unittest discover -s tests/evaluations -p 'test_*.py'
python3 tests/evaluations/prepare_longmemeval.py \
  --per-category 500 --output /tmp/sovra-longmemeval-s.json
SOVRA_RETRIEVAL_EVAL_INPUT=/tmp/sovra-longmemeval-s.json \
SOVRA_RETRIEVAL_EVAL_OUTPUT=/tmp/sovra-longmemeval-report.json \
  cargo nextest run -p sovra-store-surreal --run-ignored only \
  longmemeval_source_retrieval_measurements
python3 tests/evaluations/summarize_retrieval.py \
  /tmp/sovra-longmemeval-report.json
```

Use fresh output paths. `--dataset` accepts an already downloaded upstream file;
its hash must still match. `--per-category` changes the declared subset size.
Keep the fixture hash, revision, individual misses, corpus size, machine, build
profile and raw timings with each report. Do not compare subset recall with a
vendor's full-corpus QA percentage. Tune on a separate development sample before
reporting held-out improvement.

The summary command refuses incomplete runs. Macro recall averages only questions
with labeled user evidence, while displaying excluded and abstention counts next
to the denominator. It also reports how many questions returned every labeled
source. None of these metrics is generated-answer accuracy.

## File-index measurement

The [file-index measurement](../crates/sovra-files/src/index/tests/performance.rs)
uses 100/1,000/10,000 synthetic short UTF-8 files. It measures initial/incremental
capture, unchanged-file reuse, lexical searches and exhaustive cosine scans over
768-dimensional deterministic fixture vectors. Twenty distinct lookups at each
size must retrieve the exact source. Replacement, reopen and root revocation are
checked after the largest stage. These vectors isolate indexing cost; they are
not model embeddings and cannot measure semantic quality. Query timings exclude
source and vector generation. Scan timings include reading, decoding and persistence,
but exclude creation of the synthetic source files.

```sh
SOVRA_FILE_BENCH_OUTPUT=/tmp/sovra-file-index-report.json \
  cargo nextest run -p sovra-files --run-ignored only \
  file_capture_and_search_measurements
```

## Interpret reports

Record the revision, corpus hash, machine, build profile, workload, timing scope,
and failures with every result. Compare equivalent workloads and budgets. Do not
publish an old observation as the current version's score, or compare source-ID
recall with a vendor's generated-answer accuracy.

For runtime and live selected-model evaluations, see [Memory evaluation](agent-memory.md).
