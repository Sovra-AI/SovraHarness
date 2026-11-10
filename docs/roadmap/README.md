# Roadmap: from one laptop to a whole company

Status: planning document, researched 2026-11-05. Only stage 1 ships today.
Everything under stages 2–4 is **direction, not current behaviour**. Use the same
labelling rule as `docs/decks/README.md`: a line that says *Today* describes
only what the desktop app already does.

The investor deck presents four sizes of the same program:

| Stage | Label | One line | Document | Depth |
| --- | --- | --- | --- | --- |
| 1 | **Today** | **A person.** A private team of helpers on their own laptop that remembers their life and keeps their files at home. | [`01-a-person.md`](01-a-person.md) | Baseline + groundwork to start now |
| 2 | **Next** | **A team.** A few computers share helpers and memory. A busy laptop hands heavy work to a free one. | [`02-a-team.md`](02-a-team.md) | **Build-ready, very detailed** |
| 3 | Direction | **A company.** A private fleet with ledger memory, lanes for each team, and a model trained on its own computers. | [`03-a-company.md`](03-a-company.md) | Outline (likely to change) |
| 4 | Direction | **Regulated work.** Health, law, finance, government: data that must not leave. Here, it never has to. | [`04-regulated-work.md`](04-regulated-work.md) | Outline (likely to change) |
| 5 | Direction | **Collective intelligence.** Desktops contribute memory, compute and learning to people they trust; federated recall moves the question, not the data. | [`05-collective-intelligence.md`](05-collective-intelligence.md) | Outline; federated recall started |

Stage 2 gets the deepest treatment because it is the next thing to build and
because every later stage reuses its parts: device identity, signed logs, lanes,
peer-to-peer transport and work handoff. The first milestone is the one the
product needs before anything else: **being able to create a team**.

## One idea runs through all four stages

Sovra's memory is already *append-only and bi-temporal*. Facts carry
`created_at / valid_at / invalid_at / expired_at`, and a correction **supersedes**
an old fact instead of editing or deleting it (`crates/sovra-core/src/memory.rs`,
`crates/sovra-store-surreal/src/facts.rs`). That data shape is what lets the
product grow:

- **Person → Team:** a grow-only set of signed records replicates between
  computers without a server and converges by construction. Only the *fold*
  (how the records are read back as "what is true now") needs rules, and that
  fold is pure code in `sovra-core`.
- **Team → Company:** the same signed records, put in per-lane Merkle logs with
  signed checkpoints, become a tamper-evident **ledger without a coin**.
- **Company → Regulated:** a history that is never rewritten is close to what
  auditors already ask for (for example the SEC 17a-4 audit-trail alternative).
  Purge-able payloads under permanent hashes reconcile that with erasure.

## Principles for every stage

1. **Local first, server optional.** A server may add availability, relaying or
   backup ("a cloud peer"), never authority. No stage may need a central
   database to work.
2. **The person stays in charge.** Nothing is sent, run or shared without a
   visible, revocable decision. This is the same contract as today's approval
   cards.
3. **Default private.** Sharing is an explicit copy into a lane, never a move,
   and never automatic.
4. **Filter before retrieval.** Permissions are enforced in the index and the
   query, never by telling a model "don't mention it".
5. **Other people's words are data, not instructions.** Memory, helpers and
   skills from someone else never grant tools by themselves.
6. **Honest about limits.** For example: "people you remove keep what they
   already had" is shown to the user in plain words.
7. **Decide in core, do IO in adapters, render in UI.** Every new rule is a
   pure, unit-tested function in `sovra-core`. Every new library sits behind a
   port so it can be swapped. That is the existing architecture in `AGENTS.md`.
8. **Plain words.** User-facing copy follows the Skills-page rule: new
   capabilities register a grandma-friendly card in
   `crates/sovra-core/src/skills/builtin.rs`.

## Decisions that cut across stages

These are recorded once here and referenced from the stage documents. **All
six were approved by the owner on 2026-11-05.** Changing one now needs a stated
reason in the same change that alters it.

| # | Decision | Decided | Gates |
| --- | --- | --- | --- |
| D1 | User-facing name for a group of *people and computers*. "Team" already means the AI helpers (`teams.rs`, the Team page). | **"Group"** in the UI (e.g. "Family", "Smith Plumbing") and `group` in code. "Team" stays the helpers. | Team T1 |
| D2 | Which crate owns peer-to-peer networking, and how it gets an async runtime, given that `tokio` is owned by `sovra-runtime` only (`xtask/src/deps.rs`). | New adapter crate **`sovra-mesh`**, the only crate naming `iroh`/`p2panda`. It gets `tokio` as a second owner, with the reason written in `deps.rs`: iroh requires tokio, and mesh tasks are spawned from a runtime-provided handle. | Team T0 |
| D3 | Log and sync layer: adopt p2panda (Rust, on iroh, pre-1.0) or build our own envelope on iroh plus Negentropy. | **Own envelope and port in core, p2panda behind it.** Pin `=0.7.x` exactly. The swap to Keyhive/Subduction or Negentropy stays possible. | Team T2 |
| D4 | Relays for NAT traversal in production. n0's public relays are for development only. | Sovra-run `iroh-relay` in two or more regions, plus a per-group override (self-host, or a member's always-on computer). | Team T1 beta |
| D5 | Stable ids. Several ids today are counters or timestamps (`art-{millis}-{seq}`, `account-{n}-{millis}`, `sched-{slug}-{n}`) and will collide across computers. | ULID/UUIDv7 for local records, content hashes (BLAKE3) for shared records. | Person P2 |
| D6 | SurrealDB's role once data is shared. | Each computer's **local projection and index**, rebuildable from the log. Never a sync layer. Pin the version (4.0.0 shipped 2026-11-03). | Team T2 |

## How to use these documents

- Each stage document ends with a **checklist** of milestones and tasks. The
  checklists are the to-do list. Tick items in pull requests that complete them.
- Each task names the files and types it touches, as of `main` on 2026-11-05.
  Re-check names before starting; the code moves.
- Research sources are cited inline with links. Crate versions were read from
  crates.io on 2026-11-05. Pin exact versions when a task adds a dependency,
  and run `cargo xtask ci` (licences, rustls-only, unused deps) before merging.
