# File reading in chat

The native paperclip opens the system file picker. Users can select multiple files,
watch preparation, remove a file, and send a question or just the files. There is
no upload to a provider during preparation. The composer explains that sending
shares the prepared content with the selected service. Image thumbnails and file
names remain visible in saved messages.

## Choosing the right model

Model purposes are **Read documents**, **Understand pictures**, **Listen to audio**,
and **Watch videos**. Documents fall back to the active chat model. Media needs an
explicit saved choice. The same model may fill several purposes; choosing a purpose
does not change the chat model.

Missing setup appears beside the attachment, with a **Help me choose an AI** action.
The question and files stay in the composer while Models shows compatible models.
Saving the choice returns to chat, where the user still controls when to send.
Unknown capability metadata does not qualify a media model. Manual-model settings
allow users to enter capabilities confirmed by their service.

| Input | Models and transports |
| --- | --- |
| Extracted document text | Any configured chat model; an optional separate document model |
| Raster pictures / scanned PDF pages | A model advertising image input and text output, on OpenRouter, Anthropic, Ollama's compatible endpoint, or another compatible endpoint |
| Audio | A model advertising audio input, through OpenRouter or OpenAI-compatible chat; the latter encodes WAV/MP3 only |
| Video | A model advertising video input through OpenRouter |
| Mixed inputs | The selected purpose's model must support every attached input; otherwise send the kinds separately or choose a model that supports the combination |

These rules reflect the pinned ADK 2.3.0 conversion code, not provider/model name
heuristics. Model/service-specific size and duration limits may be lower than
Sovra's local limits. The user sees provider failures through the existing chat
error path. There is no hidden OCR service or automatic model download.

PCM WAV and ordinary, non-fragmented MP4/QuickTime files also supply local duration
hints to the shared input budget. Longer recordings reserve more room before any
provider request; reopening an older attachment derives the hint again from its
stored bytes. Unsupported or malformed timing retains the previous estimate and
adds a reader note, preserving any other notes. The original media is unchanged.
These timing fields do not prove that a codec is valid or bound every provider's
token use; other audio/video formats and live-model calibration remain incomplete.
No FFmpeg executable or other media service is required by the app.

## Readers and limits

Every regular file can enter the reader regardless of its name or extension, subject
to the size/resource limits below. Acceptance, format recognition, and complete
semantic decoding are different outcomes. A novel, proprietary, encrypted or
unsupported binary format is **inspectable**, not automatically understood.

| Family | Actual behavior |
| --- | --- |
| Text/source/config/markup | Content-first UTF-8, BOM-marked UTF-16/32, conservative BOM-less UTF-16, and detected legacy encodings. No extension allowlist or downloaded syntax grammar. Source whitespace and line endings are preserved. Legacy encoding guesses are labelled. |
| PDF and documents | Xberg 1.3.10 with native PDF, Office, spreadsheets, email, HTML/XML, iWork, HWP, notebook and MDX features. Text-like formats without a specialized reader remain readable as source. |
| ZIP and packaged files | Container signatures distinguish Office from generic archives even under a misleading name. Generic ZIP packages such as JAR/APK are traversed as member inventories. |
| HWPX | Bounded ZIP/XML member reading with original XML text. Embedded media and page layout are not rendered. |
| TAR, GZIP, XZ, LZMA, BZip2, Zstandard | Bounded in-memory decompression; GZIP/XZ/Zstandard include concatenated streams/frames. Zstandard skips metadata frames. BZip2 currently decodes the first stream and explicitly warns about later concatenated streams. |
| SQLite/GeoPackage | Read-only ordinary tables and column-labelled JSON rows. Views, virtual tables, triggers, and generated columns are not evaluated. Binary cells are identified by size, not decoded. |
| Binary Apple property lists | Streaming dictionaries/arrays/scalars with nesting and output limits. Binary values are identified by size. XML/ASCII property lists remain readable as text. |
| Raster images | PNG/JPEG/GIF/WebP/BMP/TIFF/ICO/PNM/TGA/QOI/HDR/DDS/OpenEXR/farbfeld codecs. Decode success is required before picture-model routing. |
| SVG | Rasterized locally with system fonts, external resources disabled, and bounded embedded rasters. No scripts or nested SVG resources are executed/resolved. |
| Audio/video | Actual original media bytes are supplied to a compatible model/transport. Container recognition does not imply every codec is accepted by that service. |
| Executables/object files | Non-executing architecture, section and symbol metadata where `object` recognizes the binary, plus byte inspection. No running, loading or decompiling the program. |
| Other binary formats | Labelled inspection: format, size, SHA-256, bounded hex header and ASCII/UTF-16 string fragments with original byte offsets. Original bytes remain available to the read-only byte tools. Includes unsupported CAD/3D/scientific/font/disk-image codecs, encrypted files, RAR/7z and unavailable HEIC/AVIF decoders. This is not full document or visual extraction. |
| Empty files | Accepted with an explicit empty-file note. |

- At most eight attachments per turn; each source is at most 20 MiB. Source files
  exceeding that limit still require splitting; an arbitrary extension is not a
  reason for rejection.
- Documents retain at most 4 MiB of extracted text. Archive expansion is at most
  64 MiB across nested members, at most 100 entries and two nested archive levels.
  Expansion is charged during reads, not merely trusted from archive headers.
  XML/property-list nesting is at most 64. Xberg fixed-layout page count is capped
  at 500 where the selected parser enforces it.
- XZ/LZMA decoder memory and Zstandard windows are capped at 64 MiB. Unsupported or
  encrypted ZIP members are labelled while readable siblings remain available.
  Archive paths are labels, never extraction destinations. Archive media is
  identified as unread; attach it separately to use a media model.
- SQLite runs on an in-memory, read-only copy with defensive mode, untrusted schema,
  extension loading unavailable, SQL limits and a five-second progress deadline.
  Extraction caps ordinary tables at 100, rows at 10,000 per table, columns at 256,
  and output at 4 MiB; reached caps are labelled. No database is modified.
- Raster decoding limits allocation to 64 MiB and dimensions to 16,384 per side;
  normalization applies orientation and caps the longest output side at 2048.
  Thumbnails are 160 pixels. Animated GIF and multi-frame TIFF use the first frame
  with a visible note. SVG input is at most 4 MiB with at most four embedded rasters;
  over-budget or failed rendering falls back to binary inspection.
- A PDF with virtually no text is rendered to pictures and requires the picture
  purpose. Scanned PDFs are limited to 12 pages per attachment. One PDF render
  session is reused for its pages. Text PDFs and Office documents use text/tables;
  embedded pictures are not included, and that limitation appears on the attachment.
- Across a turn, normalized media is capped at 20 MiB. Preparation concurrency is
  two. Started synchronous codecs cannot be forcibly interrupted inside Rust;
  removing a pending attachment suppresses its completion and discards its result.
  These in-process budgets are not a process sandbox or a guarantee of a hard
  wall-clock timeout for every third-party parser.

The chat orchestrator receives real ADK inline media and a bounded selection of text
sections. Matching passages come first, with opening context when there is room;
a broad question with no matching terms starts at the beginning. A small text
budget takes a Unicode-safe window around a matching line, instead of discarding
an answer late in the section. Partial sections and omitted content are labelled.

The case-insensitive lexical search includes short identifiers and numeric values,
uses word boundaries to keep `93` distinct from `193`, and gives rare terms more
weight than repeated boilerplate. It does not perform semantic embedding search.
The prompt and file tool share one index per turn, built in an owned blocking task.
Rebuilding a reader replaces its prior file tool, including when tools are disabled.
Text sections borrow slices rather than copying the full document for each read.
Line citations refer to **extracted text**; source files retain their original line
breaks, while a PDF's extracted-text line numbers are not physical page numbers.

With a tool-capable model and the user's tools setting enabled, `read_attachment`
can continue beyond initial excerpts:

| Operation | Arguments and continuation |
| --- | --- |
| Sequential text | `section` is 1-based; returns up to three full sections with `start_line`/`end_line`. Continue using `next_section`. |
| Text search | `query` returns relevance-ranked matching sections, `total_matches`, and `next_result`. Continue with `result_offset=next_result`; an absent term returns no matches. |
| Original-byte read | `mode=bytes`, `offset`, and `length`; returns at most 4096 bytes as hex with `next_offset` and `end_of_file`. Available for inspection attachments. |
| Original-byte search | `mode=byte_search`, `query`, `encoding=utf8` (default) or `hex`. Returns up to 32 non-overlapping offsets; continue from `offset=next_offset`. A null cursor marks the final page. |

Byte patterns are limited to 1024 decoded bytes. Hex accepts complete byte pairs
with optional whitespace, such as `00 ff 4d 5a`; malformed patterns, negative
positions and invalid numeric argument types receive an error. Text queries inspect
at most 4096 characters and 64 terms. Byte search uses `memchr`; it never executes
the inspected file. The tool can only access IDs in that turn's supplied inventory.
Without tools, answers remain limited to initial excerpts. The agent is instructed
to disclose omissions and never infer whole-document understanding from excerpts or
binary fragments. File contents remain untrusted evidence, never instructions.

## Folder indexing versus attachments

The limits above apply to chat attachments. Folder indexing now streams complete
large UTF-8 text/source files with fixed-size buffers, including files beyond
20 MiB and text beyond 4 MiB. Its two-pass hash check and transaction prevent
partial or changing sources from becoming current citations. Refresh reuses an
unchanged version; cancellation retains the prior version. Binary documents and
non-UTF-8 decoding still use the bounded document readers and report unsupported
or oversized content explicitly. The broader indexing UI and exclusion controls
remain under active development; see [folder indexing](skills-and-file-knowledge.md).

## Storage and ownership

The app creates `attachments/` inside its OS data directory. Each attachment has
an opaque validated ID, a manifest, extracted text and normalized media. Inspection
attachments also retain `source.bin`; older manifests without this field still load. Writes
use a staging directory and atomic rename. Unix directories/files are owner-only
(0700/0600). Original file paths are not persisted in chat messages. Prepared
content can be loaded again even after the original is moved or removed.

Removing a composer attachment deletes its managed copy. Sent attachments are
retained for saved chat history; soft-deleting a chat does not delete evidence.
Abandoned drafts are not restored across process restarts and may leave managed
copies. No attachment content is copied into application logs or used to generate
conversation titles. Text-only follow-ups use the latest file-bearing turn.

## Research and decisions (2026-10-25)

- [Xberg 1.3.10](https://crates.io/crates/xberg/1.3.10) provides a maintained Rust
  document reader with a native PDF engine. The published manifest/source and
  selected features were inspected locally. Its broad document support avoids
  introducing Python, Pandoc, PDFium installation, or a hosted extraction service.
- [Kreuzberg v4 LTS policy](https://docs.kreuzberg.dev/lts/) says its maintenance
  window ends in 2026; new development moved to Xberg. This ruled out starting
  new work on its older PDFium-based line.
- [pdf-extract 0.12.1](https://docs.rs/crate/pdf-extract/0.12.1) and
  [Calamine](https://docs.rs/calamine/latest/calamine/) are narrower alternatives.
  Xberg already integrates spreadsheet parsing and adds the other document families.
- The exact sources under `adk-model-2.3.0/src/{openrouter,openai,anthropic}` were
  checked for inline image/audio/video conversion. OpenRouter's generic PDF path
  enables a file-parser plugin; Sovra extracts locally or sends rendered page
  images instead, avoiding an implicit hosted parser choice.
- Xberg's archive feature brings a bzip2 license outside the workspace allowlist.
  Its optional HWPX reader introduces the same dependency through `unhwp`.
  Both optional paths are disabled; HWPX uses bounded ZIP/XML member reading.
  That archive feature is disabled; the adapter owns bounded archive readers instead.
  `finl_unicode` 1.5.0 has stale manifest license metadata: its packaged
  `LICENSE-UNICODE` is **UNICODE LICENSE V3**. `deny.toml` records that factual
  clarification to the already-approved `Unicode-3.0`, without changing the
  allowlist. Recheck this clarification when upgrading that crate.

Synthetic fixtures cover DOCX, XLSX, email, ZIP, text PDF and scanned PDF. Tests also
cover expansion limits, normalized pictures, persistent copies, capability setup,
real image bytes on a mock HTTP transport, retrieval and native chat guidance.

- [Apache Tika detection](https://tika.apache.org/3.2.3/detection.html) documents why
  signatures and container inspection precede filename hints. Its
  [format list](https://tika.apache.org/3.2.3/formats.html) explicitly distinguishes
  recognition from metadata/text extraction; Sovra makes the same distinction.
- [encoding_rs](https://docs.rs/encoding_rs/0.8.42/encoding_rs/) and
  [chardetng](https://docs.rs/chardetng/1.0.0/chardetng/) support strict decoding and
  detection. UTF-32 is handled explicitly because encoding_rs does not support it.
- [SQLite defensive guidance](https://sqlite.org/security.html) informed the
  read-only configuration, schema distrust, disabled features and limits.
- Exact local sources for `file-format` 0.29.0, `resvg` 0.46.0, `lzma-rust2` 0.23.0,
  `ruzstd` 0.9.1, `bzip2-rs` 0.1.2, `object` 0.40.0 and `plist` 1.10.3 were checked.
  The optional file-format ZIP reader is disabled: its `mimetype` probe allocates
  a header-declared size before reading. Sovra uses a 256-byte bounded probe instead.
  Plist's streaming API is pinned exactly to enforce depth before building a tree.
  The Zstandard reader is explicitly looped because its default streaming wrapper
  reads only one frame. BZip2's first-stream limitation is surfaced, not hidden.
- `sevenz-rust2` 0.24.1 was evaluated but not enabled: its header decompression path
  exposed no caller-supplied memory ceiling. Enabling it would need bounded
  preflight/process isolation before accepting hostile compressed headers.
- [memchr byte search](https://docs.rs/memchr/2.8.3/memchr/memmem/index.html) avoids
  naive per-offset substring comparisons over a potentially 20 MiB binary.

The `.rs` regression was reproduced before the content-first fix. Synthetic tests
exercise source filename variants, misleading container extensions, Unicode/legacy
encodings, compressed streams, actual SVG pixels, read-only SQLite, binary plists,
empty files, malformed input, expansion limits, byte-tool bounds and managed reload.
These tests establish concrete coverage, not universal semantic decoding.

### Local search measurement

A synthetic 4,194,288-byte Rust/Unicode document (repeated
`fn invoice_total() -> u32 { 93 } // multilingual 你好` lines) was queried 30 times
for `invoice total multilingual`, returning three sections. On this Mac with the
dev-profile core library, the initial index took 189.0 ms; cached searches averaged
1.06 ms versus 203.9 ms when rebuilding the index for each query. This is a local
microbenchmark during concurrent compilation, not a release-performance guarantee
or a benchmark of document codecs/network inference. No private file was used.

The retrieval ranking follows the [inverse-document-frequency principle](https://nlp.stanford.edu/IR-book/html/htmledition/inverse-document-frequency-1.html): terms present in fewer sections are more discriminating. The implementation uses smoothed presence weighting, not a claim to implement all of BM25. Offline ADK harness tests exercise actual tool calls, rejected unattached IDs, later result pages, line citations and disabled tools.
