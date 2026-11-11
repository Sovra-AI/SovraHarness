# Knowledge capture benchmark and tool assessment

Assessment date: 11 November 2026. Sovra already uses modern memory foundations:
immutable evidence, attributed interpretations, temporal corrections, hybrid
retrieval, and permission checks at recall. Keep those foundations. The priorities
are measuring realistic recall, preserving document structure, and addressing
the current dependency audit failures before considering a storage migration.

This assessment covers conversation memory, indexed files, and their retrieval
paths. It distinguishes code inspection, previous measurements, and new benchmark
work. A storage throughput result cannot establish whether a model remembered the
right fact or answered correctly.

## Current implementation

| Area | Evidence in the implementation | Assessment |
| --- | --- | --- |
| Conversation capture | [Admission](../crates/sovra-core/src/memory/admission.rs), [durable processing](../crates/sovra-store-surreal/src/ingestion.rs), [runtime capture](../crates/sovra-runtime/src/memory_capture.rs) | Source identity, authorship, durable receipts, resumable extraction and bounded jobs are appropriate. Imported assistant transcripts cannot be granted trustworthy lineage by relabeling them as user statements. |
| Grounding and changes | [Candidate validation](../crates/sovra-core/src/memory/ingestion.rs), [corrections](../crates/sovra-store-surreal/src/lifecycle.rs), [dependency withdrawal](../crates/sovra-store-surreal/src/reply_dependencies.rs) | Exact evidence quotes, retained history, temporal dates and transitive withdrawal are valuable safeguards. An exact quote validates provenance, not the truth of the model's interpretation. |
| Conversation retrieval | [Recall policy](../crates/sovra-core/src/memory/recall.rs), [source search](../crates/sovra-store-surreal/src/sources.rs), [semantic recall](../crates/sovra-runtime/src/memory_semantic.rs) | Facts can use lexical and semantic search plus related entities; raw episodes use lexical retrieval. Paraphrases of information omitted by extraction remain a gap. The eight-term query limit, bounded source excerpts and two-second runtime deadline need realistic relevance evaluation. |
| File capture | [Readers](../crates/sovra-files/src/lib.rs), [indexing](../crates/sovra-files/src/index/scan.rs) | Xberg 1.3.10 provides local format decoding. OCR and quality processing are disabled. The adapter retains extracted text, with reader warnings; it does not preserve the extractor's full structured document as the indexed representation. |
| File storage | [Catalogue and FTS](../crates/sovra-files/src/index/mod.rs), [chunking](../crates/sovra-files/src/index/scan.rs) | SQLite FTS5 and content-addressed chunks support local search and incremental reuse. FastCDC's 512/2048/8192-byte chunks preserve bytes and reuse, but do not guarantee intact headings, table headers or semantic units. |
| File semantic search | [Vector reader](../crates/sovra-files/src/index/semantic.rs) | Current-version and grant checks are present, but cosine ranking scans each eligible vector with a five-second limit, and embedding storage caps each folder at 20,000 unique passages. Measure this path separately; conversation HNSW results do not establish folder-search scalability. |

## Tools worth using

These are fit recommendations for this Rust desktop application, not a measured
cross-product ranking. No alternative backend has been benchmarked here.

| Purpose | Recommended choice | Why and adoption condition |
| --- | --- | --- |
| Conversation source of truth | Retain SurrealDB behind `MemoryStore` | Its integrated [full-text](https://surrealdb.com/docs/learn/data-models/full-text-search/overview), vector and graph capabilities match the existing boundary. A replacement would need to preserve atomic corrections, provenance, permissions and migrations. No measured result currently justifies that migration. |
| Local document extraction | Retain Xberg; compare structure on representative documents | [Xberg](https://docs.xberg.io/) is Rust-native and already integrated. Evaluate extracted tables, reading order, headings and scanned pages before selecting a replacement. Upstream benchmark claims are not Sovra results. |
| Structure-aware document challenger | Docling and its Rust port | [Docling](https://github.com/docling-project/docling) preserves a structured document. Its [hybrid chunker](https://docling-project.github.io/docling/concepts/chunking/) combines hierarchy and token limits and can repeat table headers. [docling.rs](https://github.com/docling-project/docling.rs) is a Rust candidate; validate exact APIs, dependency weight and equivalent fixtures before adoption. |
| Memory architecture reference | Graphiti and Hindsight | [Graphiti](https://github.com/getzep/graphiti) is useful for temporal graph design, but introduces a Python/backend deployment path. [Hindsight](https://github.com/vectorize-io/hindsight) documents parallel semantic, lexical, graph and temporal retrieval, followed by fusion and reranking. Use these as controlled challengers if their added model and service costs improve Sovra's measured cases. |
| Managed memory alternative | Mem0 | [Mem0](https://github.com/mem0ai/mem0) offers memory infrastructure and an extraction/consolidation workflow. It is a candidate for a separately chosen service deployment, not evidence that the local store should be replaced. Preserve Sovra's withdrawal and authorship contracts in any comparison. |
| Larger vector workloads | Qdrant Edge as an optional local challenger | [Qdrant Edge](https://qdrant.tech/documentation/edge/) has an embedded Rust API and requires no background server, but is currently beta. Compare against the folder vector scan only when it misses the latency/resource budget. Another retrieval engine does not replace the provenance and version catalogue. |
| Local embeddings and reranking | FastEmbed-rs as an opt-in challenger | [FastEmbed-rs](https://github.com/Anush008/fastembed-rs) supports local embeddings and reranking through ONNX Runtime. Test model quality, cold/warm latency, RAM and packaging on supported platforms; account for model downloads and native runtime dependencies before adding it to the desktop app. |
| Conversational evaluation | LongMemEval-S | The [official benchmark](https://github.com/xiaowu0162/LongMemEval) has turn/session evidence labels and separates updates, temporal reasoning, multi-session questions and abstention. Use full distracting histories; oracle sessions cannot establish search quality. |
| Future workflow memory evaluation | LongMemEval-V2 | [LME-V2](https://xiaowu0162.github.io/longmemeval-v2/) evaluates multimodal web-agent experience, including workflows, state changes and gotchas, with both accuracy and latency. It complements conversational memory evaluation; it is not a drop-in score for Sovra's current private-chat source reader. |

[Contextual Retrieval](https://www.anthropic.com/engineering/contextual-retrieval)
supports testing contextualized chunks alongside BM25 and embeddings. Keep original
evidence separate from generated context, retain exact citations, and compare against
the current pipeline with equal retrieval budgets. Likewise, add a reranker only
after a held-out test shows an improvement within the latency budget. Neither a
graph nor a newer embedding model guarantees better answers.

## Measurement protocol

The existing [runtime measurement](../crates/sovra-app/tests/memory_lifecycle/performance.rs)
uses synthetic unique codes, real disk storage, `Client` and a deterministic local
HTTP provider. The retained [November 3 result](evaluations/memory-recall-2026-11-03/runtime-dependency-storage.json)
found all 576 expected source items at 1k/10k/100k corpus sizes and 1/4/16 concurrency.
That is historical exact-source evidence, not a current-machine rerun, semantic
retrieval score, extraction score or answer-quality score.

The [preparation script](../tests/evaluations/prepare_longmemeval.py) pins the
cleaned LongMemEval-S revision and SHA-256. The full run selects all 500 questions
with `--per-category 500`, retaining every distracting user turn in each history.
The default four-per-category selection remains a quick 28-question smoke test.
The script checks the complete upstream file before selecting. Expected evidence
IDs stay outside indexed text; reference answers are not copied into the fixture.
Public corpus text stays outside the checkout.

The [real-engine diagnostic](../crates/sovra-store-surreal/tests/store/retrieval_evaluation.rs)
measures admission time and source-ID recall at 1, 4 and 12 using `mem://`, plus
source IDs admitted by the real `MemoryContext` item/byte budget. It excludes
extraction, embeddings, runtime scheduling, disk durability and generation.
Assistant evidence is excluded because imported transcripts lack certified provider
input lineage. Non-abstention questions without labeled user evidence are unscored;
abstention cannot receive a correctness score without a reader. Returning a source
ID does not prove that its bounded excerpt contains the answer.

The [manifest](evaluations/knowledge-2026-11-11/manifest.json) records all 500
questions and 122,416 user turns. Nine empty turns cannot pass source admission;
none are labeled evidence, and their keys are reported separately. There are 419
scored questions, 51 non-abstention questions without user evidence, and 30 abstention
questions. The histories contain 7,525 user turns dated after their question. The
diagnostic retains these upstream dates and reports them; it does not claim temporal
filter accuracy. Repeated upstream session IDs are distinguished by occurrence,
preserving separate history entries. Fixture SHA-256:
`0d04bed9d052f08bf6a00572217bda84aa930b66f549ccd702545d76d9bcefac`.

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

## Current measurements

All 500 LongMemEval-S questions completed against the real in-memory store:
122,407 user sources admitted, nine empty turns explicitly excluded, and zero
model calls. These are user-source recall scores, not the official QA benchmark.
The 419 scored questions exclude 30 abstention questions and 51 without labeled
user evidence. Each question receives equal weight; categories are not reweighted.

| Question category | Scored questions | Recall @1 | Recall @4 / context | Recall @12 |
| --- | ---: | ---: | ---: | ---: |
| single-session-user | 64 | 70.3% | 90.6% | 98.4% |
| single-session-preference | 30 | 7.8% | 31.7% | 57.8% |
| knowledge-update | 72 | 30.1% | 85.2% | 94.4% |
| multi-session | 121 | 23.1% | 57.4% | 75.9% |
| temporal-reasoning | 127 | 28.6% | 57.8% | 71.9% |
| single-session-assistant | 5 | 20.0% | 60.0% | 80.0% |
| overall | 419 | 32.1% | 65.6% | 80.1% |

The context contained every labeled user source for
209/419 questions; top-12 retrieval did so for
290/419. Five questions in the assistant category also label user
evidence; only those user labels are scored. This is not assistant-memory coverage.
Raw case IDs, misses, denominators and timings are retained in the
[report](evaluations/knowledge-2026-11-11/retrieval.json) and
[aggregate](evaluations/knowledge-2026-11-11/retrieval-summary.json).

The run took 151.86 seconds. Store search p50/p95 were 8.80/17.24 ms; these omit
runtime, disk, embeddings and generation, and other workspace tests overlapped.
Treat them as observed diagnostic timings, not an isolated latency baseline.

Miss review found lexical mismatch in multi-session questions about siblings,
visits and baking: supporting turns describe specific relatives, venues or recipes
without matching the question's wording. Candidate recall also exceeds the current
four-source context substantially. The first controlled experiment should compare
semantic retrieval/query reformulation and reranking under the same context budget,
with new held-out cases. Increasing the budget alone cannot recover missing top-12
candidates or establish answer correctness. Do not tune on these 500 questions and
then report them as held-out improvement.

The disk fact-store run completed at 100, 1,000 and 5,000 facts, checking lexical,
entity and paginated reads at each size. See the retained
[disk measurements](evaluations/knowledge-2026-11-11/disk-growth.json).

The first file-index run failed at 10,000 files when a vector commit exceeded its
five-second SQLite deadline. The [failed baseline](evaluations/knowledge-2026-11-11/file-index-before.json)
retains the smaller completed stages. The fix prepares the insert once per batch
and checks the submitted chunk through existing reverse indexes, preserving
current-version, root-permission and exact-content checks. It does not extend the
deadline or add a storage dependency.

The [rerun](evaluations/knowledge-2026-11-11/file-index.json) completed all sizes:

| Files | Capture scan (s) | New fixture-vector writes (s) | Keyword p95 (ms) | Vector scan p95 (ms) |
| ---: | ---: | ---: | ---: | ---: |
| 100 | 0.08 | 0.07 | 2.85 | 10.00 |
| 1,000 | 0.75 | 1.24 | 2.19 | 79.85 |
| 10,000 | 10.02 | 167.48 | 6.40 | 769.26 |

All 60 keyword and 60 exact-vector checks found the expected source. Unchanged scans
reused every file; replacement, reopen and root revocation passed. Capture and vector
write stages are incremental (100, then 900, then 9,000 new files). These are test-profile
observations, with concurrent workspace activity; synthetic vectors do not measure
semantic quality. A read-only progress sample and a one-second profiler attempt occurred
during the largest write stage. At 10,000 files, the 167.48-second vector preparation/write
stage remains costly even though the timeout is fixed. Sampling identified pending-batch
selection as further work; optimize that path before deciding an engine replacement is
necessary. Exhaustive vector ranking also grows roughly with corpus size. Qdrant Edge
is a suitable controlled challenger, subject to its beta status and permission semantics.

The isolated selected-model run used **anthropic/claude-sonnet-5.5 through OpenRouter**,
100 synthetic distractors and 16 observations. All extraction jobs completed on
the first attempt. Answer latency p50/p95 was 4.08/8.00 seconds.
Token usage and cost were not instrumented; they are unknown, not zero. Answers
were in Arabic despite English fixture prompts; this run does not evaluate language
selection. See the [observations](evaluations/knowledge-2026-11-11/live.json) and
[manual review](evaluations/knowledge-2026-11-11/live-review.json).

Manual review found the requested recall, paraphrase reuse, current/historical city,
quoted-instruction rejection, unknown-answer abstention, person separation, newsletter
correction and withdrawal behaviors. Two observations have caveats: the historical
answer adds an unsupported claim about missing later evidence, and the return-move
extractor supersedes an earlier move event even though both events happened. The
current residence chain is correct, but immutable event capture needs a stronger
consolidation distinction. These findings prevent treating process success as perfect
answer/capture accuracy. This small synthetic run is not a public QA benchmark score.

## Verification checkpoint

The workspace diagnostics compile and pass Clippy. All eight Python preparation
and aggregation tests pass, including occurrence identity, atomic publication,
refusal to overwrite and rejection of incomplete scores. The first 28-case attempt
stopped after 10 cases at a repeated upstream session identity; its
[incomplete report](evaluations/knowledge-2026-11-11/retrieval-incomplete.json) is
retained as diagnostic history, not an aggregate score. The importer now distinguishes
session occurrences.

The current targeted nextest run covered the file adapter, memory store and runtime
memory lifecycle: **250 passed, one failed, seven opt-in tests skipped**. The failed
streaming pause/resume test passed alone in 13.34 seconds using the same executable.
Another workspace test run overlapped the first run; this does not establish a
production defect or an unconditional green suite. Existing format regressions cover
text/scanned PDF handling, Office/archive reading and UTF-8/legacy/multilingual
content, but do not score real-world table layout or OCR retrieval quality.

An earlier [disk-growth attempt](evaluations/knowledge-2026-11-11/disk-growth-incomplete.json)
failed at 5,000 facts when the filesystem filled during another build. Its partial
timings are diagnostic only. Current reruns use fresh paths after cache reclamation.

The repository's cache dry run initially found 118.6 GiB of artifacts and refused
reclaim because the existing development session owned its lock. The user then
authorized reclaim. With the app stopped, the repository cleanup command removed
112.4 GiB of debug artifacts and 7.5 GiB of release artifacts, leaving 1.5 GiB in
the target directory. `just free-cache` now aliases the same guarded cleanup.
The complete public-corpus rerun above followed that cleanup.

The latest `cargo xtask ci` passed formatting, Clippy, layering, styles and docs,
then stopped at the dependency audit before tests. The MCP dependency was updated
in concurrent, uncommitted work; its previous advisory no longer appears in this
working-tree run. That separate dependency update is not part of the benchmark
commit, so a checkout still on `rmcp 3.8.0` can also report `RUSTSEC-2026-0462`.
The remaining working-tree failure is:

```text
error[vulnerability]: libsqlite3-sys via C SQLite CVE-2026-119375
  RUSTSEC-2026-0459; libsqlite3-sys 0.37.0; upgrade to >=0.39.0
advisories FAILED, bans ok, licenses ok, sources ok
xtask: `deny` failed
```

The [SQLite advisory](https://rustsec.org/advisories/RUSTSEC-2026-0459.html)
concerns opening an attacker-controlled database/WAL pair. Update the compatible
SQLite consumers together (file index, Diesel and WhatsApp storage share one native
SQLite link); do not waive the audit or loosen dependency rules. This checkpoint
does not meet the repository's definition of done.

The current published WhatsApp SQLite adapter is still 0.7.0 with a `libsqlite3-sys
0.37` constraint; Diesel 2.3.14 requires `<0.39.0`. Thus updating only `rusqlite` to
0.41.1 cannot resolve the shared native link. A compatible upstream release or a
reviewed, reproducible dependency patch is needed; changing the lockfile alone is
not a fix. The MCP SDK has a published 3.9.0 fix.

## Follow-up priorities and open verification

The full public-corpus source diagnostic, disk growth run, 10,000-file rerun and
selected-model observations above are complete. A fresh 1k/10k/100k runtime run is
still in progress; its initial 1,000-source stage found all 192 expected source
items. No aggregate result is claimed until the whole run completes.

1. Distinguish immutable past events from replaceable current-state facts during
   consolidation. The live return-move observation is a concrete failure case for
   that distinction; retain exact evidence and both timelines.
2. Compare source-level semantic retrieval/query reformulation and reranking under
   the same context budget, using new held-out questions. FastEmbed-rs is a suitable
   local inference challenger; adopting it requires model/packaging measurements.
3. Optimize pending-vector selection and compare an ANN index if folder search misses
   its latency budget. Preserve current-version and permission checks across any engine.
4. Compare structure-preserving document/chunk retrieval with Xberg and Docling on
   text PDF, scanned PDF, tables, Office and multilingual fixtures. The current format
   regressions prove reading behavior, not comparative layout/OCR quality.
5. Resolve the SQLite advisory and rerun `cargo xtask ci`. Native Development MCP
   inspection/capture succeeded, but the bridge cannot operate the OS folder picker;
   large-folder semantic ingestion has adapter-level coverage, not a completed native
   UI scenario. The test-launched app was restored to Chat and closed.
