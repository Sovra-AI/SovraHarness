# Stage 1 — A person (Today)

> **Today.** A private team of helpers on their own laptop that remembers their
> life and keeps their files at home.

This document records what ships now, the honest gaps, and the **groundwork**
that should land on a single laptop *before* stage 2 starts. Each groundwork
item is worth having on its own, and each one removes a blocker for sharing.

## 1. What ships today

Counts are from the repository on 2026-11-05.

| Area | Today | Where |
| --- | --- | --- |
| App | Native Rust desktop app (GPUI), macOS / Windows / Linux, one codebase. About 154k lines of Rust in 12 crates, about 1,100 test attributes, `unsafe` forbidden. | `crates/*`, `xtask/src/deps.rs` |
| Helpers | 12 named teammates (8 default + 4 ready-made extras) and a 23-role specialist roster, gated by which tools are connected. 49 built-in skills, all on the Skills page. | `sovra-core/src/teams.rs`, `specialists.rs`, `skills/builtin.rs` |
| Memory | Embedded SurrealDB (`surrealkv://`). Immutable episodes. Bi-temporal facts that are superseded, never deleted. Entities and `mentions` / `derived_from` / `supersedes` edges. Hybrid recall: HNSW + BM25 fused by RRF (pure `fuse_rrf` in core). A 2-second budget with keyword fallback. | `sovra-core/src/memory*.rs`, `sovra-store-surreal/src/*` |
| Files | Folders read in place, live watch plus a 15-minute full check, version history, an ontology mapped to DCMI / schema.org / NEPOMUK / IPTC, and human-reviewed relationships. | `docs/files.md`, `docs/file-ontology.md` |
| Models | OpenRouter, Ollama (including in-app pulls), any OpenAI-compatible server (LM Studio, vLLM, llama.cpp), Anthropic. Routing: Laya, OpenJev, or an offline heuristic. | `sovra-core/src/models.rs`, `routing_tree.rs` |
| Channels | WhatsApp (built in), Telegram bot, Google (Gmail drafts only, Calendar, Drive, Contacts, Tasks), MCP servers, terminal with Allow once / Always / Don't. | `docs/communications.md`, `docs/google.md`, `docs/terminal.md` |
| Trust | Keys in the OS keychain. Untrusted senders get general answers only, with no memory, files or tools. Artifacts are never executed in-app. | `sovra-runtime/src/connections.rs`, `sovra-adk/src/lib.rs` (`for_untrusted_sender`) |

## 2. Honest gaps (true today, fix independently of stage 2)

- The app does not update itself yet. Velopack builds packages and feeds, but
  there is no update check or in-app update action.
- Signing and notarisation run only when CI secrets are present. The
  documented package is an unsigned Linux build.
- Meaning search (embeddings) is opt-in and, by default, uses a remote
  endpoint. There is no bundled local embedding model.
- Google restricted scopes need verification (CASA). Until then there is a
  100-user lifetime cap and an "unverified" warning.
- Schedules exist only in UI state. There is no persistence, no runtime
  executor and no `Command` variant (`sovra-core/src/schedules.rs`).
- Account creation and session restore still need UI work
  (`docs/architecture.md`).
- No LongMemEval run has been done (`docs/agent-memory.md`).

## 3. Why the data model needs groundwork before sharing

The codebase map (2026-11-05) found these blockers for any multi-computer
feature:

1. **No ownership on records.** The SurrealDB namespace/database (`sovra`/`memory`)
   is the only boundary. `memory/ingestion.rs` says "the store handle fixes
   ownership; models cannot supply it". There is no person, device or
   workspace id on `Episode`, `Fact`, `Artifact`, `TeamMember` or chats.
2. **Colliding ids.** These ids are counters or timestamps and are unique only
   on one machine:
   - artifacts: `art-{millis}-{seq}`
   - accounts: `account-{n}-{millis}`
   - schedules: `sched-{slug}-{n}`
   - chats: `chat-{namespace}-{n}`
   - directly recorded episodes: SurrealDB auto ids

   Facts already use ULIDs, and admitted episodes use SHA-256 keys.
3. **Only one permission notion.** `MemorySourceScope{Private, Unknown}` in
   `memory/admission.rs` is the only scope.
4. **Wall-clock time only.** `Timestamp(i64)` is Unix milliseconds, supplied
   by callers. Two computers cannot order events reliably with wall clocks
   (sleeping laptops, skew).
5. **One-blob stores.** The helper roster is one JSON blob at `team:roster`,
   and `team:connections` and `team:auth` reuse the `team` table as a
   key-value store. They have no per-item author, version or origin.
6. **`workspace_label` is a hard-coded string** ("Personal Workspace",
   `workspace.rs`). There is no workspace entity.

None of this needs networking to fix, and each fix improves the single-laptop
product too: backups, exports, cleaner corrections, and multiple profiles on
one computer.

## 4. Groundwork milestones

> These are **Next** work. Nothing below ships today.

### P1 — Identity on one computer

Goal: every install has a durable **device identity**, and every signed-in
account has a **person identity**, so later stages have something to sign
with.

- [ ] **Core types** (`sovra-core`, new `identity.rs`).
  - `DeviceId` holds an Ed25519 public key, 32 bytes.
  - `PersonId` is the root key of the person's device group.
  - `DeviceProfile{device, display_name, platform, created_at}`.
  - All pure, serde, with no crypto dependency in core: core holds bytes,
    adapters sign and verify.
- [ ] **Key generation and storage** in an adapter.
  - Generate a 32-byte Ed25519 secret on first run.
  - Store it as a binary secret through the existing `CredentialStore`
    (`sovra-providers/src/storage.rs`, keyring v4), in a new keychain entry
    such as `com.Sovra.SovraHarness.device`.
  - The development exception (`SOVRA_DEV=1`, `dev-credentials/`) applies
    unchanged.
  - Fail loudly on Linux machines with no Secret Service. Never fall back to
    writing the key to plain disk in release builds.
- [ ] **One-time recovery code.** Show it at person creation (BIP-39 words) and
  explain it in plain words. It becomes the offline admin key in stage 2.
- [ ] **Settings → "This computer"** shows the device name and a short
  fingerprint. Add `.test_support()` IDs for the dev MCP.

### P2 — Stable, collision-free ids (decision D5)

- [ ] Replace counter and timestamp ids with ULID / UUIDv7:
  - artifacts (`artifacts.rs:708`)
  - accounts (`sovra-runtime/src/auth.rs:426`)
  - schedules
  - chats (`workspace.rs:325`)
  - direct `record_episode` (SurrealDB auto ids)
- [ ] Migrate existing records. Keep old ids as `legacy_id` aliases, and add a
  schema bump in `sovra-store-surreal/src/schema.rs` (`SCHEMA_VERSION` 12 → 13).
- [ ] Add a unit test in core that checks the id formats.

### P3 — Ownership and lanes in the data model (single computer)

Goal: every record knows **who authored it** and **which lane it lives in**,
even though there is only one lane today.

- [ ] Core: `LaneId` (opaque 32-byte id) and `Lane{id, kind: Private |
  Group(GroupId) | Pair(PersonId), name}`. Extend `MemorySourceScope` from
  `{Private, Unknown}` to carry a `LaneId`. Keep it an exhaustive enum.
- [ ] Add `author: DeviceId` and `lane: LaneId` to these records:
  - `Episode`, `Fact`, `Artifact`, `TeamMember`
  - chat threads
  - schedules, once persisted (see P5)

  Default every existing record to the person's private lane.
- [ ] Add a local-only `received_at` column next to `created_at`. It means
  "when this computer learned it", and stage 2 needs it to keep
  `created_at` / `expired_at` author-defined.
- [ ] Retrieval takes a `LaneFilter` and applies it **inside** the
  SurrealDB query, before BM25 and HNSW scoring, not after. Add tests in
  `sovra-store-surreal`.
- [ ] Split the `team` key-value table: `team:roster` becomes one row per
  helper with `author`, `lane`, `version` and `content_hash`.

### P4 — Logical time

- [ ] Add a **hybrid logical clock** (HLC) value type in core,
  `Hlc{wall_ms, counter, device}`, with pure `tick` and `merge` functions
  (Kulkarni et al., <https://cse.buffalo.edu/~demirbas/publications/hlc.pdf>).
  The runtime owns the clock state and passes values in. Core still never
  reads a clock.
- [ ] Record `hlc` on every new write next to `created_at`.
- [ ] Tolerate large drift: laptops wake after days. `uhlc`'s default 500 ms
  guard is far too strict
  (<https://docs.rs/uhlc/latest/uhlc>). Do not depend on that crate unless
  the guard is configurable.

### P5 — Persist schedules

- [ ] Add `Command::Schedules(ScheduleCommand)` / `Event::Schedules(..)`, a
  runtime executor, and storage through the `MemoryStore` port. Use the new
  id format, author and lane. This is needed in its own right, and stage 2
  shares schedules.

### P6 — Write-ahead operation log (local)

Goal: a single append-only list of **operations** becomes the source of truth
on one computer, with SurrealDB as a projection of it. Stage 2 then
replicates the log instead of the database.

- [ ] Add operation types in core:
  - `Op{id: ContentHash, author, lane, hlc, schema: u16, body: OpBody}`
  - `OpBody` variants: `RecordEpisode`, `SaveFact`, `Supersede{new, olds}`,
    `StopUsing`, `Redact{target}`, `SaveArtifactVersion`, `SaveHelper`,
    `SaveSchedule`, and the rest
- [ ] Add a pure `fold(ops) -> view changes` in core. The SurrealDB adapter
  applies the changes. Add a `rebuild_projection()` that replays the log into
  an empty store and is exercised in tests.
- [ ] Keep `Supersede.olds` as a **set**, so a merge supersession can retire
  several facts at once (stage 2 rule).
- [ ] Choose the on-disk log format early and version it. Devices may stay
  offline for months, so the format is a long-term commitment.

### P7 — Close the honest gaps that block a second computer

- [ ] In-app update check (Velopack) and signed builds on every release. A
  family running two versions needs updates, and protocol negotiation needs
  them too.
- [ ] Bundle or one-click install a **local embedding model** (via Ollama), so
  meaning search works with no cloud. Stage 2 re-embeds shared text locally
  on each computer.

## 5. Definition of done for stage 1 groundwork

- Every record has `author`, `lane`, stable id, `hlc` and `received_at`.
- The SurrealDB store can be deleted and rebuilt from the operation log.
- A device key exists, is stored in the OS keychain, and is shown in Settings.
- `cargo xtask ci` passes, and the new behaviour is covered by core unit tests
  and runtime `Client` tests.
- Skills page: register "See and name this computer" in
  `skills/builtin.rs` with friendly copy.
