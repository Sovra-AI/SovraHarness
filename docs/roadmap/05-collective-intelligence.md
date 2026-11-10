# Stage 5 — Collective intelligence (Direction)

> **Direction, not today**, except where a line says *Today*. Every desktop can
> contribute **memory**, **compute** and **learning** to the people it trusts,
> and its owner stays in charge of each one separately.

Owner's brief (2026-11-10): extend groups into a decentralised network where each
person's helpers learn locally, share what is worth sharing, answer each other's
questions without uploading whole memories, and optionally contribute spare
compute and approved examples to training. The differentiator is not
decentralised storage. It is **making intelligence portable and collectively
improvable without anyone surrendering their whole knowledge base.**

This document maps that brief onto what Sovra already has, names what is missing,
and orders the work. It follows the [principles](README.md#principles-for-every-stage)
of every stage: local first, the person stays in charge, default private, filter
before retrieval, other people's words are data.

## 1. Three things a desktop can contribute

| Contribution | What it means | Separate permission |
| --- | --- | --- |
| **Memory** | Knowledge, experiences, documents and lessons its helpers learned | Per mode (below), per group |
| **Compute** | Spare CPU/GPU for other members' inference, embedding, indexing and (later) training | "Help others in my group" (stage 2 T4) |
| **Learning** | Validated improvements: strategies, evaluations, approved training examples, adapters | "Let my approved knowledge train shared models" — never implied by sharing memory |

Sharing memory and contributing to training are **different permissions**,
because model updates can leak what they were trained on.

## 2. Three memory modes

| Mode | Behaviour | Sovra |
| --- | --- | --- |
| **Private** | Never leaves the desktop | *Today:* every personal memory |
| **Federated** | Authorised helpers send a *question*; the desktop searches its own memory and returns a few approved answers. The query moves, the data stays. | *Today:* Settings → Groups → **Ask each other's memories** (see §4) |
| **Shared** | Selected memories and artifacts are signed and synchronised to authorised peers | *Today:* shared details and shared artifacts in the group lane (`docs/roadmap/02-a-team.md` T2/T3) |

## 3. The brief's technology, mapped to Sovra's

The brief proposed Tauri, libp2p, SQLite + vec1, Mem0/Graphiti, Automerge, MLS +
OPA and Ray + PyTorch. Sovra already chose equivalents for most layers, and
changing them would not buy the brief's goals:

| Layer | Brief | Sovra (why) |
| --- | --- | --- |
| Desktop app | Tauri + Rust | GPUI + Rust: native, no web view, already shipped |
| P2P networking | libp2p | **iroh** (`sovra-mesh`): QUIC, hole punching, encrypted relays, keys as addresses (roadmap D2) |
| Local memory | SQLite + vec1 | **SurrealDB** (`sovra-store-surreal`): vectors + graph + full text behind the `MemoryStore` port |
| Memory intelligence | Mem0 / Graphiti | Sovra's own bi-temporal extraction and RRF recall (`sovra-core/src/memory`), Graphiti-style validity |
| Synchronisation | Automerge | Signed, hash-chained operation sets with a pure fold (`sovra-core/src/group`). Converges like a CRDT, plus signatures, roles and dispute rules Automerge does not have |
| Sharing security | MLS + OPA | Signed membership log + roles today; **lane encryption** (p2panda-encryption or MLS) is the open T2 item; policy stays pure Rust in core rather than a Rego engine |
| Distributed training | Ray + PyTorch | Not chosen yet. A Python sidecar per desktop is the likely shape (§6) |

## 4. Federated recall — *Today* (first slice)

Built 2026-11-10 on the group network (`sovra-core/src/group/federated.rs`,
`sovra-runtime/src/groups.rs`):

- **Two opt-ins per computer, both off:** *Let my group's helpers ask my memory*
  (answering) and *Ask my group when I chat* (asking). Asking is separate because
  the asker's question leaves their computer.
- **Ask my group** in Settings → Groups asks once, now; pressing it is the consent
  for that one question.
- The answering computer searches **its own recall** (nothing when remembering is
  paused), returns at most 4 short statements reworded with the owner's name
  ("Mum's dentist…"), and is rate-limited to 20 questions a minute per computer.
- The owner sees **every question** asked of their memory and how many answers
  went back.
- On the asking side answers arrive as attributed group evidence ("other people's
  words are data"), with a 2.5 s deadline and a cooldown for computers that do not
  answer, so a chat never waits on an offline laptop.
- Proven over real iroh by `crates/sovra-app/tests/two_computers.rs`.

**Next for federated recall:**

1. *Approved results*: per-subject or per-lane scopes ("answer about the house,
   not about health"), and an optional "ask me first" queue.
2. Semantic answering (the owner's embedding search, not keyword only).
3. **A permission-filtered topic directory** so questions go only to computers
   likely to know, without the directory revealing private topics or embeddings
   (coarse, owner-approved topic labels; never raw vectors).
4. Provenance-aware reranking across many answering computers (RRF over
   per-computer ranks, with reputation from §5).

## 5. Collective knowledge that improves itself

1. **Local discovery**: helpers notice validated solutions (a task that worked,
   an experiment that improved a metric) and propose them as reusable memories.
2. **Consolidation**: the desktop deduplicates, verifies and separates private
   from shareable, and shows the proposal; the owner approves.
3. **Shared or federated** by the modes above.
4. **Reputation**: every item keeps provenance, time, confidence and evidence.
   Corroboration (today: *Agree*), disputes (today), new-member damping and
   per-source trust protect against poisoning. Malicious contributions are
   contained by roles, signatures and "data, not instructions".

## 6. Distributed learning (later, opt-in, guarded)

1. Participating desktops train compatible **adapters** (LoRA) or produce
   **validated training examples** from approved knowledge only.
2. Updates are combined with **secure aggregation**, with differential privacy
   where the data warrants it, and tested before anyone uses them.
3. A controller evaluates candidates on quality, cost, latency and resource use
   and keeps the better ones ("evolutionary optimisation").
4. Accepted strategies, evaluations and adapters become discoverable like any
   other shared item, with provenance.

Shape: compute advertisement and job handoff come from stage 2 T4; training runs
in a sandboxed sidecar per desktop that never receives raw memory, only approved
examples. Revocation is honest: a revoked contribution can be excluded from
future rounds, but what a model already learned cannot be guaranteed to vanish.

## 7. The first prototype (the brief's own proposal)

Three desktops, federated semantic retrieval, explicit sharing permissions and a
single training experiment on approved shared knowledge.

| Step | Status |
| --- | --- |
| Three desktops in one group, signed membership | *Today* (any number) |
| Shared memory and artifacts with explicit permissions | *Today* |
| Federated retrieval with opt-in and a visible log | *Today* (keyword); semantic answering next |
| Topic directory and fan-out ranking | Next |
| One training experiment (adapter on approved examples, three desktops) | Later, after T4 job handoff |

Validate the proposition at that size before large-scale discovery or autonomous
evolutionary training.

## 8. Risks

| Risk | Answer |
| --- | --- |
| A question reveals the asker's interest | Asking is its own opt-in; questions go only to group members; the answering owner sees them |
| Answers leak more than intended | Few, short, owner-named answers; scopes and "ask me first" next; remembering paused means nothing answers |
| Poisoned knowledge | Signatures, roles, disputes, corroboration, reputation, "data, not instructions" |
| Training leaks memory | Separate permission, approved examples only, secure aggregation, DP, evaluation before use |
| Directory leaks private topics | Owner-approved coarse labels only; no embeddings in the directory |
| Offline peers stall helpers | Deadlines and cooldowns (today), and optional always-on helper boxes (stage 2 T6) |
