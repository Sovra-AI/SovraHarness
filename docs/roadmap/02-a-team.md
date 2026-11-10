# Stage 2 — A team (Next)

> **Next, not today.** A few computers share helpers and memory. A busy laptop
> hands heavy work to a free one.

This is the build-ready plan for the next stage. It is deliberately detailed:
every milestone lists the files, types, protocols and tests it needs. The
first milestone is **creating a group** (T1). Memory sharing, helper sharing
and work handoff build on it.

Research date: 2026-11-05. Crate versions and licences come from crates.io on
that date. Sources are linked inline and collected in §19.

---

## 0. Summary

- **What a group is:** a small, private set of **people** and their
  **computers** (a family, or a 3–20 person business) with no central server.
- **How computers talk:** they dial each other by public key over
  **iroh 1.x** (QUIC, hole punching, encrypted relay fallback).
- **How membership works:** it is a **signed, hash-chained log** of
  operations. Computers pair by **QR code or link**, with a short-code
  fallback for two computers in the same room.
- **How memory is shared:** in **lanes** (Only me / the group / me and one
  person). A lane is an encrypted group space whose key rotates when someone
  leaves.
- **Why sharing is safe to merge:** shared records are *signed, immutable
  operations*, so replicas converge without a server. A pure fold in
  `sovra-core` turns them into today's bi-temporal view and decides
  conflicts deterministically. When two people correct the same fact
  differently, both versions stay and the fact is shown as **disputed**.
- **Helpers and skills:** they travel as **signed bundles** that only
  *request* tools. Each person grants tools on their own computer.
- **Work handoff:** a busy computer sends a **whole request** (chat,
  embeddings, indexing) to a free computer in the group. That computer runs
  it on its own loopback Ollama or llama.cpp and streams the answer back.
  Model ports are never opened to the network.
- **Where the code goes:**
  - one new adapter crate, **`sovra-mesh`**, the only crate that names `iroh`
    and `p2panda`
  - pure rules and ports in `sovra-core`
  - long-running tasks in `sovra-runtime`
  - screens in `sovra-ui`

## 1. Scope

**Goals (v1)**

1. Create a group, add your own other computers, invite people, set roles, and
   remove computers or people (T1).
2. Share memory through lanes, with offline catch-up and visible "who taught
   this" (T2).
3. Share helpers, skills, artifacts and schedules safely (T3).
4. Hand LLM, embedding and indexing work to a free computer (T4).
5. Person-to-person lanes, a calm change digest, and dispute resolution (T5).
6. Production relays, an optional always-on "helper box", and a hardened beta
   (T6).

**Non-goals for stage 2**

- **Real-time co-editing of artifacts.** Edits stay versioned per author.
  Loro or yrs can come later.
- **Splitting one model across computers** (tensor parallelism). llama.cpp RPC
  calls itself "proof-of-concept" and "insecure"
  (<https://github.com/ggml-org/llama.cpp/blob/master/tools/rpc/README.md>).
- **Groups larger than about 50 computers.** Larger fleets are stage 3.
- **Any central Sovra database, or any account with Sovra-the-company.**

## 2. Who it is for — user stories

**Family (the Smiths: Mum, Dad, Sam, Gran)**

- *Mum* creates "Family" on her laptop, adds her iMac by scanning a QR code,
  and sends Gran a link. Gran taps it, sees "Mum's laptop invited you", checks
  that the four emoji match what Mum reads out on the phone, and is in.
- *Dad* tells his helper "the boiler service is booked for 12 March" and taps
  **Share with Family**. Mum's helper can answer "when is the boiler being
  serviced?" with "12 March — Dad told me on 3 Feb".
- *Sam's* notes stay **Only me** unless Sam shares them. A helper never reads
  another person's private lane, because the index does not contain it.
- *Gran's* old laptop is slow. When she asks something long, the answer comes
  from Mum's iMac ("Answered by Mum's iMac") because it is plugged in and idle.
- Sam's phone is lost. Mum, an admin, removes it from any of her computers.
  The app says plainly: "Sam's phone will stop getting new family memories.
  What it already had stays on it."

**Small business (Smith Plumbing: 6 people, 9 computers)**

- The office manager creates "Smith Plumbing" and invites staff as **members**.
  The owner and the manager are **admins**, because the app requires two.
- A shared **"Quote writer"** helper is published once. Each person sees what
  it will be told and which tools it asks for, and switches it on themselves.
- Price list changes are corrections in the business lane. When two people
  change the same price differently while offline, the app shows "Two
  different updates — choose one", and nothing is lost.
- The workshop desktop with a GPU does the heavy indexing of job photos and
  PDFs overnight for everyone.

## 3. Words used here

| Word (UI) | Code | Meaning |
| --- | --- | --- |
| Group | `group` | People plus their computers. Decision D1 in the roadmap README: "Team" stays the AI helpers. |
| Person | `PersonId` | A human. Their identity is the root of a *nested group of their own devices* (Keyhive/Jazz pattern). |
| Computer | `DeviceId` | One install, identified by an Ed25519 key (the iroh `EndpointId` *is* this key). |
| Lane | `LaneId` | An encrypted space with members. **Only me**, **Group**, or **Me and Sam**. Every record lives in exactly one lane. |
| Admin / Member / Viewer / Contributor | `Role` | Manage / Write / Read / write-only. p2panda-auth levels are Pull < Read < Write < Manage. |
| Helper box | — | An optional always-on computer (e.g. a Mac mini) that relays ciphertext and does heavy work. It is never an authority. |

## 4. User experience

All copy follows the plain-words rule. Every control has a stable id, an
accessible label and `.test_support()`. Layouts follow the responsive rules in
`docs/styling.md`.

### 4.1 Create a group (T1, the first thing to build)

1. Settings → **Groups** → **Create a group**.
2. Name it (e.g. "Family"). Choose a picture, optional.
3. The screen explains what a group is in one sentence: "People in a group can
   share memories, helpers and work between their computers. Nothing is shared
   until you choose to share it."
4. **Recovery code** (if the person has none yet, from stage 1 P1). Show 12
   words, ask the person to write them down, then confirm 3 of them.
5. Done. The founding computer is an **admin**. The roster shows one computer,
   with a banner: "Add a second admin so you're never locked out."

Behind the scenes, the computer creates `GroupId` (32 random bytes) and the
first membership operation, `Genesis{group, founder_person, founder_device,
name}`, signed by the device key.

### 4.2 Add one of *my* computers

1. On the computer already in the group: **Add a computer → One of mine**. It
   shows a **QR code** and a **copyable link** (`sovra://join/…`). Both expire
   in 10 minutes and can be used once.
2. On the new computer: **Join a group → Scan or paste**.
3. Both screens show "Connecting…" and then "Joined Family as *Mum's iMac*".
4. **Same room, camera not available:** choose "Use a code instead". The first
   computer shows **6 digits**. The second types them. Both then show **4
   emoji**, and the person taps "They match" on both screens.

### 4.3 Invite a person

1. **Add a person** → name and role (Admin / Member / Viewer / Contributor).
2. Send the link (any chat app) or show the QR code.
3. The invitee sees who invited them, the group name, and the role. They
   check the **4 emoji** with the inviter by voice or in person. This catches a
   forwarded or intercepted link. The QR path in the same room may skip it.
4. The new person's computer becomes the first device of their own nested
   device group inside the group.

### 4.4 The roster

- People, each with their computers, a role and an online dot.
- Actions: **Rename**, **Change role**, **Remove computer**, **Remove person**,
  **Leave group**.
- Removing shows the honest message: "*Sam's phone* will stop receiving new
  memories and work. Things it already had stay on it. Change anything
  sensitive it knew."
- Removing an admin when only two admins exist needs the other admin's
  confirmation (two-admin rule).

### 4.5 Lanes in everyday use

- Every memory shows a **lane chip**: *Only me* (default), *Family*, *Me & Sam*.
  On the shared-smart-speaker problem, see Huang et al., CHI 2020,
  <https://lersse.ece.ubc.ca/?p=696>.
- Actions: **Share with Family…** copies the fact into the lane (it never moves
  it), **Make private** stops sharing new versions, and **Who taught this**
  shows the history, for example "Sam, from a chat on 3 Oct · changed by Mum on
  5 Oct · Undo".
- Every helper shows which lanes it can read.
- **Forget** has two strengths:
  - **Hide** supersedes the fact, so it no longer appears in answers.
  - **Erase the words** drops the payload and sends a redaction to every
    computer. The copy says plainly that offline or misbehaving computers may
    keep a copy.

### 4.6 Shared helpers

- **Publish to Family** on any helper. Recipients see a card with: who
  published it, what it will be told (the full prompt), the tools it asks for,
  and its version.
- The helper does nothing until the recipient taps **Switch on** and grants
  tools on their own computer. When an update asks for more tools, it shows a
  diff and asks again.

### 4.7 Sharing work

- Settings → **This computer** → **Help others in my group**. The options are
  *When plugged in and idle* (default), *Always*, or *Never*, plus which models
  to offer.
- Answers that came from another computer show a chip: **Answered by Mum's
  iMac**.
- The helping computer shows a quiet status: **Working for Gran's laptop ·
  Stop**.

### 4.8 What's new and disputes

- A calm daily digest, "What's new in Family", lists new shared facts,
  corrections, and **disputes that need a decision**. There are no per-fact
  pop-ups.
- A dispute card shows both versions with their authors. Choosing one creates
  a new merge correction. Nothing is erased.

## 5. Architecture

### 5.1 Crates and layering

```
sovra-app  ──►  sovra-ui ──► sovra-runtime ──► sovra-core
    │                             ▲               ▲
    └──► sovra-mesh ──────────────┼───────────────┘   (adapter: iroh, p2panda)
         (wired in the composition root, passed to the runtime as trait objects)
```

- **`sovra-core`** holds the pure parts:
  - types: identity, group, lane, op envelope, job messages
  - the **ports**: `GroupNetwork`, `GroupLog`, `JobExchange`
  - membership fold, pairing state machine, SAS derivation, memory fold and
    conflict rules, scheduler scoring, lane policy
- **`sovra-mesh`** (new adapter) owns:
  - the iroh endpoint and the ALPN handlers (`sovra/pair/1`, `sovra/sync/1`,
    `sovra/job/1`)
  - gossip presence, blob transfer and p2panda logs/auth/encryption
  - `impl` of the core ports
- **`sovra-runtime`** owns:
  - the sub-runtimes `GroupRuntime`, `SyncRuntime` and `HandoffRuntime`
  - leases, retries and forwarding to the local model provider
  - projection updates via the `MemoryStore` port
- **`sovra-ui`** owns the screens in §4.
- **`sovra-app`** creates the mesh adapter and passes it in through
  `RuntimeServices` as `group: Option<Arc<dyn GroupNetwork>>`. This is the same
  pattern as `MessagingChannels`.

**Layering change (decision D2, approved 2026-11-05).** iroh requires a tokio runtime, and
`xtask/src/deps.rs` lets only `sovra-runtime` use `tokio`. The change, written
with its reason as `AGENTS.md` requires:

- `LAYERS`: add `("sovra-mesh", &["sovra-core"])`, and add `sovra-mesh` to the
  `sovra-app` list.
- `EXTERNAL_OWNERS`:
  - `iroh*` and `p2panda*` → `["sovra-mesh"]`
  - `tokio` → `["sovra-runtime", "sovra-mesh"]`, with the reason: "iroh's
    endpoint needs a tokio runtime. sovra-mesh never builds its own runtime.
    It receives a `tokio::runtime::Handle` from sovra-runtime and spawns only
    owned, cancellable tasks returned to the caller."
- `clippy.toml` already disallows bare `tokio::spawn`. Mesh tasks go through a
  small `Spawner` the runtime provides, so every task has an owner and
  cancellation.

*Rejected alternative (for the record):* putting the iroh code inside
`sovra-runtime` behind a module boundary. That is simpler for the layering
check, but it bloats the runtime and makes the transport harder to swap.

### 5.2 Ports in `sovra-core` (new `group/` module)

```rust
/// Transport + membership: everything that talks to other computers.
#[async_trait]
pub trait GroupNetwork: Send + Sync {
    async fn create_group(&self, name: &str) -> Result<GroupSnapshot, GroupError>;
    async fn invite(&self, group: GroupId, invite: InviteRequest) -> Result<Invite, GroupError>;
    async fn join(&self, ticket: JoinTicket, sink: GroupSink) -> Result<JoinOutcome, GroupError>;
    async fn confirm_sas(&self, pairing: PairingId, matches: bool) -> Result<(), GroupError>;
    async fn apply(&self, op: SignedMembershipOp) -> Result<(), GroupError>;
    fn presence(&self, group: GroupId) -> Vec<PeerPresence>;
}

/// Replicated operation log per (lane, device): the source of truth for shared records.
#[async_trait]
pub trait GroupLog: Send + Sync {
    async fn append(&self, lane: LaneId, body: OpBody) -> Result<OpRef, GroupError>;
    async fn ops_since(&self, lane: LaneId, cursor: LogCursor) -> Result<OpPage, GroupError>;
    fn subscribe(&self, sink: OpSink);  // live delivery after catch-up
}

/// Work handoff.
#[async_trait]
pub trait JobExchange: Send + Sync {
    async fn offer(&self, to: DeviceId, offer: JobOffer, sink: JobSink) -> Result<JobHandle, GroupError>;
    fn serve(&self, handler: Arc<dyn JobHandler>);
}
```

Names are illustrative. Rules for the port types:

- Every type crossing a port is plain serde data.
- Signatures are `[u8; 64]`.
- Verification happens in the adapter, and core re-checks invariants
  (chain continuity, role at causal point) in pure functions with unit tests.

### 5.3 Protocol additions (`sovra-core/src/protocol.rs`)

- `Command::Group(GroupCommand)`:
  - `Load`, `Create{name}`, `Invite{group, role, kind}`, `Join{ticket}`
  - `ConfirmSas{pairing, matches}`, `Rename`, `ChangeRole`
  - `RemoveDevice`, `RemovePerson`, `Leave`
  - `SetSharing{policy}`, `ShareToLane{record, lane}`
  - `ResolveDispute{fact_ids, choice}`
- `Event::Group(GroupEvent)`:
  - `Loaded(GroupState)`, `InviteReady{qr, link, expires}`
  - `PairingProgress{stage}`, `SasToConfirm{emoji, digits}`
  - `Joined`, `MemberChanged`, `Removed`, `PresenceChanged`
  - `DisputeRaised`, `Digest`
  - `Error`
- `Event::Handoff(HandoffEvent)`: `AnsweredBy{session_id, device}`,
  `WorkingFor{device, job}`, `Fallback{reason}`.
- The enums stay exhaustive, so the compiler lists every `match` to update.
- Pure reducer `GroupState::apply(event)` in core. The UI renders it.

## 6. Identity and the membership log

### 6.1 Keys

| Key | Held by | Purpose |
| --- | --- | --- |
| Device key (Ed25519) | OS keychain, one per install | Transport identity (the iroh `EndpointId`) and signing of ops. Never leaves the device. |
| Person root | Derived from the recovery code (offline) | Signs "this device belongs to this person". Recovers access when every device is lost. |
| Lane keys (symmetric, epoch-numbered) | p2panda-encryption group state | Encrypt lane payloads. Rotated on any removal. |

### 6.2 Membership operations

The membership log is one hash-chained, signed log per group, modelled on
Keybase sigchains, Tailscale's Tailnet Lock and Apple's keychain circle.

```text
MembershipOp = {
  group: GroupId, seq: u64, prev: Hash, previous: Vec<Hash>, // causal deps (DAG)
  hlc: Hlc, author_device: DeviceId, body, signature }
body =
  Genesis   { name, founder_person, founder_device }
| AddPerson { person, role, invited_by }
| AddDevice { person, device, display_name, platform }   // signed by person root or an existing device of that person
| RemoveDevice { device, reason }
| RemovePerson { person, reason }
| ChangeRole { person, role }
| Rename    { name }
| SetRelay  { relay_urls }
```

**Validation** is a pure function in core, run on every computer for every op:

1. The signature is valid, and `author_device` is a current device of a person
   whose role allows this op at its causal point.
2. Chain continuity holds. If one device signs two different ops with the same
   `seq`, that is a **fork**: quarantine the device and warn admins (the
   Holochain lesson, <https://developer.holochain.org/concepts/7_validation/>).
3. Admin-only ops: add or remove persons, change roles, relays. A person may
   add or remove *their own* devices.
4. Two-admin invariant: an op that leaves the group with fewer than 2 admins
   needs a co-signature from another admin, or the recovery key.

**Concurrent edits** (two admins acting while offline) merge as follows:

- The merge is the set union of valid ops, ordered by the causal DAG first,
  then HLC, then hash.
- **Remove wins over Add** for the same device or person.
- **Strong removal**: ops by an actor who was removed concurrently are
  invalid. This follows p2panda-auth's default `StrongRemove` resolver
  (<https://docs.rs/crate/p2panda-auth/0.7.0/source/README.md>). Ops that come
  causally *after* a removal are simply unauthorised, so someone added back
  can act again.
- **Never zero admins** (as built, 2026-11-06): two admins who remove each
  other concurrently do **not** both leave. The first removal in the fixed
  order applies, and the second fails because its author is gone, so the
  group always keeps an admin. This deliberately differs from p2panda's
  "both removed" to avoid locking a family out. The UI still warns before an
  admin removes another admin.

### 6.3 Recovery

- **All devices lost:** use the recovery code to recreate the person root, sign
  `AddDevice` for a new computer, and ask an admin (or use the code, if the
  person is an admin) to accept it.
- **Founder lost and no second admin:** the recovery code acts as an offline
  admin key, as with Tailnet Lock's disablement secrets and Keybase paper keys.

## 7. Pairing protocol (`sovra/pair/1`)

### 7.1 Invite (QR or link, the primary path)

```text
Invite = { v:1, group, role, kind: OwnDevice|Person,
           inviter: EndpointTicket { id, relay_url, direct_addrs },
           secret: [u8;16] (random), expires_at, single_use: true }
encoded as  sovra://join/<base32(postcard(Invite))>   and as a QR code
```

1. The joiner dials `inviter.id` with ALPN `sovra/pair/1` over iroh (direct
   path or relay).
2. **Joiner → inviter:** `Hello{invite_id, joiner_device, joiner_person?,
   display_name, platform, proof = HMAC(secret, transcript)}`.
3. The inviter checks that the invite is unused and unexpired, then verifies
   the proof.
4. **Inviter → joiner:** `Accepted{membership_log_snapshot, lane_welcome}`.
   The inviter has appended `AddDevice` or `AddPerson` + `AddDevice`, signed
   by itself.
5. The joiner verifies the whole log from genesis, stores it, and connects to
   the other online members.
6. The invite is burnt as soon as it is used.

The 128-bit secret makes guessing impossible, so no PAKE is needed on this
path. The link carries the inviter's iroh ticket, and iroh tickets "expire
when an endpoint changes networks or relays"
(<https://docs.iroh.computer/concepts/discovery>). After 10 minutes, or on
failure, the UI offers "Make a new invite".

### 7.2 Short code (same room, no camera)

1. The inviter advertises "pairing open" over **mDNS** for 2 minutes. It uses
   `iroh-mdns-address-lookup`, so discovery and transport share one identity.
   The inviter shows a 6-digit code (about 20 bits).
2. The joiner finds the inviter on the LAN, connects on `sovra/pair/1`, and
   both run **SPAKE2** with the identities bound to both `EndpointId`s:
   `spake2::Ed25519Group`, identity = `"sovra-pair-v1" ‖ inviter_id ‖
   joiner_id`. Wormhole rule: **one guess, then the code is burnt**
   (<https://magic-wormhole.readthedocs.io/en/latest/welcome.html>).
3. A shared key from SPAKE2 authenticates the same `Hello` / `Accepted`
   exchange as in §7.1.

### 7.3 Short authentication string (SAS)

Both screens show 4 emoji (or 6 digits for accessibility) derived from
`BLAKE3(transcript ‖ sorted(endpoint_ids))`. Signal safety numbers and KDE
Connect's fix for CVE-2020-26164 show why: a device must never be trusted by
its user-chosen *name* alone
(<https://seclists.org/oss-sec/2020/q4/169>, <https://signal.org/blog/safety-number-updates/>).

- **Required** for person invites sent as links, and for the short-code path.
- **Optional** for QR scanned in the same room.

### 7.4 Threats and answers

| Threat | Answer |
| --- | --- |
| A link is forwarded to the wrong person | It is single-use and expires in 10 minutes. The SAS check by voice is required for person invites. Admins see the joiner's name and platform and can remove them. |
| Someone guesses a short code | SPAKE2 gives one guess per code. It is LAN-only and lasts 2 minutes. |
| A fake device names itself "Mum's iMac" | Identity is the key, not the name. The SAS check catches it, and the roster shows fingerprints. |
| A relay sees the traffic | Relays are blind: QUIC is end-to-end encrypted (<https://docs.iroh.computer/concepts/relays>). The relay still sees IP metadata, so a per-group relay override is offered. |
| `spake2` is unaudited | The crate README warns "USE AT YOUR OWN RISK" (<https://github.com/RustCrypto/PAKEs>). The high-entropy QR/link is the primary path, the short code is LAN-only, and an external review is booked before GA. |

## 8. Transport and discovery

- **iroh 1.4.x.** Pairs connect directly in about 90% of networks, and the
  rest use encrypted relays
  (<https://docs.iroh.computer/concepts/relays>, <https://www.iroh.computer/blog/v1>).
  It uses rustls over ring and no OpenSSL.
- **Admission:** an incoming connection is accepted only if the remote
  `EndpointId` is a current device in some group this computer belongs to.
  The pairing ALPN is the only exception, and it accepts only holders of a
  live invite.
- **ALPNs:**
  - `sovra/pair/1`
  - `sovra/sync/1` (p2panda log sync, via the p2panda-net handler)
  - `sovra/job/1`
  - `sovra/presence/1` (iroh-gossip topic `blake3(group ‖ "presence")`)

  Each is versioned, and computers negotiate the highest common version. Do
  not copy exo's "same build only" rule (<https://github.com/exo-explore/exo>).
- **Relays (decision D4):**
  - Development uses n0's public relays, which are "for development and
    testing, not production", rate-limited, and subject to change
    (<https://docs.iroh.computer/concepts/relays>).
  - Production uses Sovra-operated `iroh-relay` in at least 2 regions.
  - Each group can use its own relay URL (`SetRelay` op), for example a
    business's server or a member's always-on computer.
- **Discovery:**
  - The default is a signed Pkarr record published through
    `iroh-dns-server`. A self-hosted option is offered with the relay.
  - **mDNS is an optimisation only:** off by default except while pairing,
    and on for "home network" groups if the person agrees.
  - Never make discovery a requirement: macOS Local Network prompts are
    unreliable, Windows networks default to *Public*, and Microsoft advises
    enterprises to block inbound mDNS
    (<https://developer.apple.com/forums/thread/814226>, <https://techcommunity.microsoft.com/blog/networkingblog/mdns-in-the-enterprise/3275777>).
- **OS packaging tasks:**
  - **macOS:** add `NSLocalNetworkUsageDescription` and `NSBonjourServices`
    to Info.plist, plus a first-run explainer for the prompt (TN3179,
    <https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy>).
  - **Windows:** a firewall rule for the app in the Velopack install hook,
    because otherwise the first inbound UDP listener triggers the Defender
    dialog.
- **Battery:** back off gossip heartbeats on battery, respect low-power mode,
  and keep the endpoint asleep when the person chooses *Never help others*.

## 9. Shared memory

### 9.1 What is replicated

The source of truth is the **operation log** from stage 1 P6, replicated per
`(lane, device)`. SurrealDB stays each computer's **projection and index**,
rebuildable from the log (decision D6). SurrealDB changefeeds are
retention-windowed and single-source, and have no multi-writer merge
(<https://surrealdb.com/docs/learn/querying/real-time/changefeeds>). They are
**not** used for sync.

```text
Envelope (p2panda-core header or Sovra's own; postcard/canonical CBOR)
  author_device, person, lane, log_seq, backlink: Hash, previous: Vec<Hash>,
  hlc, schema: u16, payload_hash: BLAKE3, signature
Body (encrypted with lane key epoch E)
  RecordEpisode { id, kind, content, occurred_at, source }
  SaveFact      { id, content, entities, valid_at, derived_from, extraction }
  Supersede     { new: FactId, olds: Vec<FactId>, reason }
  Corroborate   { fact }
  StopUsing     { fact }
  Redact        { target: Hash }            // payload-only erasure
  SaveArtifactVersion { artifact, version, content_hash }   // big content via blobs
  PublishHelper { bundle_hash }             // see §10
  SaveSchedule  { … }
Local-only (never replicated): received_at, embeddings, BM25 index, trust_label, adopted grants
```

**Rules**

- **Ids are content hashes** for shared records, so re-delivery is idempotent.
- **`created_at` / `expired_at` are author-defined** (from the author's HLC),
  so every computer derives the same history. `received_at` is local.
- **Private ids never leak.** When a fact is shared, a `derived_from` that
  points at private-lane episodes is replaced by an opaque per-share token.
- **Payloads are separable from headers**, so `Redact` drops the words while
  keeping the log valid. p2panda-core supports this
  (<https://docs.rs/p2panda-core/latest/p2panda_core/>).

### 9.2 Lanes = encrypted spaces

- *Only me* is a space whose members are only my devices. My own computers
  sync it, and nobody else holds it.
- *Group* is a space whose members are persons. Each person is a nested group
  of their devices.
- *Me & Sam* is a two-person space. For a one-off file, an `age`-encrypted
  drop is enough (`age` 0.12.1).
- **Encryption** uses p2panda-encryption's *data* scheme: a symmetric group
  key (XChaCha20-Poly1305) that late joiners can use to read history, and that
  **rotates on removal**
  (<https://p2panda.org/2025/02/24/group-encryption.html>).
- **Limits.** Post-compromise security heals only after every member syncs
  the removal. There is no post-quantum protection yet. The audit was
  reported as pending, so we track it and pin versions.

### 9.3 Sync

- **On connect:** p2panda LogSync, pull-based. Exchange heads per log, then
  fetch the missing ops, then switch to live mode
  (<https://docs.rs/p2panda-net/latest/p2panda_net/>). If p2panda is
  replaced, use Negentropy range-based set reconciliation over iroh
  (`negentropy` 0.5.3, MIT).
- Large payloads (artifacts, attachments, file excerpts) move as **iroh-blobs**
  with BLAKE3-verified streaming, and only over direct paths when the payload
  is big.
- **Ordering:** authority comes from causal links and signatures, never from
  wall clocks. HLC is an ordering hint only.

### 9.4 The fold and conflict rules (pure, `sovra-core/src/group/fold.rs`)

1. **Validate first.** Each op needs:
   - a valid signature
   - chain continuity (a fork means quarantine)
   - an author who held at least *Write* in that lane at the op's causal point
   - a known schema
2. Episodes and facts form a grow-only set (G-Set). The fold is deterministic
   over (DAG order, HLC, hash).
3. **A supersession is a set.** F1 is expired if any valid `Supersede` names
   it. `expired_at(F1)` is the minimum author-HLC over those ops.
4. **Concurrent siblings** happen when F2 and F3 both supersede F1 and neither
   is causally after the other.
   - If they are equivalent, they become **corroboration** (one fact, two
     authors). Equivalence means normalised text equality, or a local
     embedding plus a local-model equivalence check.
   - Otherwise they are **Contested**. The display winner is the later
     `valid_at`, then role (Admin > Member), then HLC, then hash. Retrieval
     may use the winner but **labels it disputed** and includes the sibling.
     A later `Supersede{new: F4, olds: [F2, F3]}` resolves it.
5. **Strong-removal filter.** Ops from authors who were removed concurrently
   are dropped, along with dependants.
6. **Lane isolation.** A `Supersede` may only target facts in its own lane.
   Cross-lane correction creates a new fact that references the old one.

**Property tests** (with `proptest` as a dev-dependency) check that for any
permutation and any duplication of a set of valid ops, the fold produces an
identical view. This is the convergence guarantee.

### 9.5 Retrieval

- **Lane filter before scoring**, inside the store query, before BM25 and
  HNSW. GateMem (2026) found that retrieval-based memories leak unauthorised
  or deleted items when filtering happens afterwards
  (<https://arxiv.org/abs/2606.18829>).
- RRF gets two extra weights:
  - **provenance trust**: mine > my devices > group member > external
  - **new-member damping**: facts from a person or device added less than 7
    days ago are labelled "new — not yet confirmed" until someone corroborates
    them
- **Vectors are never accepted from other people.** Each computer re-embeds
  shared text with its own model, in a background, resumable queue. Vectors
  from *my own* devices are accepted only when the model fingerprint matches.
  The reason is AgentPoison: optimised triggers in embedding space
  (<https://neurips.cc/virtual/2024/poster/94715>). Each index stores
  `embedding_model`, and a model change triggers a blue-green re-index.

## 10. Shared helpers, skills, artifacts and schedules

**Helper and skill bundles** (T3)

```text
HelperBundle = { id: BLAKE3(manifest ‖ files), version, author: PersonId,
                 manifest { name, icon, role, instructions, expertise,
                            requested_tools: [..], requested_lanes: [..] },
                 files (skill folders: SKILL.md + assets), signature }
HelperAdopt  = { bundle, granted_tools, granted_lanes }   // LOCAL only, never replicated
```

- **Grants are never in the bundle.** Grants work like browser-extension
  permissions: a local decision, re-asked when a new version requests more.
- The preview shows the **full instructions** text before switching on. The
  2026 malicious-skill wave delivered natural-language payloads that code
  scanners miss: 157 malicious skills out of 98,380 scanned
  (<https://www.arxiv.org/pdf/2602.06547>, <https://snyk.io/blog/toxicskills-malicious-ai-agent-skills-clawhub>).
- **Today's model:** `TeamMember` (`teams.rs`) is already serde. Wrap it,
  rather than inventing a new shape. Built-in helpers are never published,
  because every computer has them.

**Taint rules for tools** (T3, CaMeL-style, <https://arxiv.org/abs/2503.18813>)

- Anything retrieved from another person's lane content **taints** the turn.
- In a tainted turn, side-effecting tools need a fresh confirmation card that
  shows *which shared memory* influenced the call. Side-effecting tools are:
  send a message, write a file, terminal, Gmail draft, calendar add, payments.
- Group-lane content enters the prompt inside a quoted-data block labelled
  with its author.
- A helper may never promote a fact from a lower-trust lane into a higher one
  on its own.
- **Rationale:** MINJA planted memories through queries alone, with 98.2%
  injection success (<https://arxiv.org/abs/2503.03704>). OWASP's Agentic Top
  10 lists memory poisoning as ASI06.

**Artifacts and schedules**

- **Artifacts** are shared per version:
  - each `SaveArtifactVersion` is an op, and content moves as a blob
  - concurrent edits produce two versions with authors
  - **Keep both** / **Choose one** is offered
  - deletes stay soft and ask twice, as today
- **Schedules** in a group lane:
  - only the author's computer executes a schedule, unless it is handed off
    explicitly
  - timed messages are never executed by another person's computer

**Untrusted senders keep their rule.** WhatsApp and Telegram strangers still
get `for_untrusted_sender()`, with no memory or tools. Group membership does
not change that.

## 11. Work handoff (`sovra/job/1`)

### 11.1 What moves

v1 moves **whole requests** only (the LocalAI "federated" model,
<https://localai.io/docs/features/distribute/>):

| Kind | Worker does | Notes |
| --- | --- | --- |
| `Chat` | Calls its own `127.0.0.1:11434/api/chat` (Ollama) or local OpenAI-compatible server, and streams tokens back. | Tools stay on the requester. The worker returns model output only, and tool calls are executed by the requester's computer. |
| `Embed` | `/api/embed` batches. | Used for re-embedding shared text and indexing. |
| `Summarise` / `Extract` | Memory extraction jobs (`HttpSummarizer`). | Returns structured output, which the requester validates as today. |
| `Index` (T4b) | Text extraction plus embeddings of files the *requester* sends as blobs. | Only files the person chose to send. Never the worker's own files. |

### 11.2 Protocol

1. **`Offer`**:
   - `job_id` (UUIDv7, the idempotency key)
   - `kind`, `model`, `payload_hash`, `max_tokens`, `deadline`, `priority`
   - `requester`
2. **`Accept{lease_ms}`** or **`Decline{reason}`**. Reasons: on battery, user
   active, model not present, queue full, sharing off.
3. **Stream**:
   - `Token` / `Progress` frames
   - a heartbeat at least every `lease_ms / 3`
   - if the lease expires, the requester resubmits **the same `job_id`**
     elsewhere or runs locally
   - workers de-duplicate by `job_id`, caching results for a short TTL
4. **Finish**: `Done{usage, result_hash}` or `Error`.
5. **Cancellation**: a `Cancel` frame or a QUIC stream reset. The worker
   aborts the upstream HTTP request. Test this per backend.
6. **Large data**: inputs and outputs above 1 MB move as iroh-blobs hashes.

### 11.3 Scheduler (pure, `sovra-core/src/group/schedule.rs`)

- **Score candidate computers by:**
  - model loaded (`/api/ps`) > on disk (`/api/tags`)
  - free memory (unified on Apple Silicon: RAM plus `size_vram`) or VRAM
  - plugged in
  - user idle ≥ N minutes
  - queue depth
  - measured RTT
  - direct vs relayed path: bulk work only over direct paths
  - the person's sharing policy
- **Fallback:** local → another computer → cloud, according to the person's
  existing routing policy. A cloud model is never used for a lane whose
  policy says "stay on our computers".
- **Visibility:** always show **Answered by …**.

### 11.4 Where it plugs into today's code

- **Option (a), recommended:** a new `ProviderKind::Group` (`models.rs`). The
  compiler then lists every match site: `sovra-adk/src/lib.rs`
  (`from_selected_model`), `sovra-adk/src/memory.rs`,
  `sovra-providers/src/summaries/protocol.rs`, and `sovra-runtime/src/models.rs`.
  - A `ModelProvider` impl lists the models that group computers advertise.
  - `ModelKey.connection_id` names "Family computers", so routing, per-helper
    model choice and `SimulateRoute` work unchanged.
  - Inside `from_selected_model`, build an `adk_model::Llm` that forwards over
    `JobExchange` and keeps the existing `admission::wrap`.
- **Option (b):** an `Llm` wrapper that transparently offloads when the local
  machine is busy. Later, after (a) works.
- **Not in v1:** forwarding whole `RunPrompt`s or team turns, because tools
  and memory would then run on someone else's computer.

### 11.5 Capability and power detection (no `unsafe`)

| Need | Crate | Version (licence) |
| --- | --- | --- |
| AC/battery | `starship-battery` (maintained fork; `battery` has had no release since 2020) | 0.12.0 (ISC) |
| User idle (macOS, Windows, X11) | `user-idle` or `system-idle-time` | 0.6.0 (MIT/Apache) / 1.0.4 (MIT) |
| Linux power and idle via D-Bus (UPower `OnBattery`, logind `IdleHint`) | `zbus` | 6.2.1 (MIT) |
| CPU and RAM | `sysinfo` | 0.40.1 (MIT) |
| NVIDIA VRAM (optional) | `nvml-wrapper` | 0.13.0 (MIT/Apache) |

On Wayland, idle is "unknown". The person can choose "help when plugged in"
instead. These crates live in an adapter, not core. They can go in
`sovra-mesh`, or in a small `sovra-device` crate if the dependency set is
large.

### 11.6 Security posture

- **Never** ask anyone to set `OLLAMA_HOST=0.0.0.0`. Internet scans in 2026
  found 12k–175k unauthenticated Ollama servers
  (<https://blog.leakix.net/2026/02/ollama-exposed/>).
- Only group computers can reach the worker, and only over iroh, which
  forwards to loopback.
- Job payloads are untrusted:
  - size limits, model allow-list, per-device quotas
  - **no tools or code ever run for a remote job**
  - jobs never read the worker's memory or files

## 12. Presence

- iroh-gossip on topic `blake3(group ‖ "presence")` carries
  `Heartbeat{device, seq, capabilities, sharing_policy}`. It is sent every 15
  s on AC and every 60 s on battery, plus an immediate delta on change.
- Liveness: 3 missed heartbeats → *away*, 10 → *offline*.
- A full mesh of direct connections is fine up to about 20 devices. Keep
  `foca` 2.0.0 (SWIM, MPL-2.0) in reserve above about 50.

## 13. Threat model

| # | Threat | Mitigation | Milestone |
| --- | --- | --- | --- |
| 1 | Stolen laptop holds a valid device key | Remove from any admin device. Lane keys rotate. The copy is honest about what is already on it. Encourage OS disk encryption. | T1, T2 |
| 2 | Malicious or compromised member poisons shared memory | Provenance trust in RRF, new-member damping, lane-before-retrieval, taint-gated tools, "Who taught this" with undo | T2, T3 |
| 3 | Malicious helper or skill bundle | Signatures, a full-prompt preview, local-only grants, a diff on update, no grants in bundles | T3 |
| 4 | Remote job abuse (GPU burn, injection to reach tools) | Quotas, model allow-list, no tools on remote jobs, visible "Working for …" status | T4 |
| 5 | Forged or forked membership | Signed hash chain, validation at the causal point, fork quarantine, two-admin rule | T1 |
| 6 | Admins race to remove each other | Strong removal removes both. A warning prompt, and the recovery key restores access. | T1 |
| 7 | Relay metadata (IPs, timing) | Blind relays, a per-group relay override, mDNS off by default | T1, T6 |
| 8 | Unaudited crypto crates (`spake2`, p2panda-encryption) | High-entropy primary path, version pinning, an external review before GA | T6 |
| 9 | Unbounded log growth | Payload redaction, snapshots plus prune points, blob offload | T5 |
| 10 | SurrealDB licence (BUSL-1.1) | It stays an internal component. Never expose schema or table control to users or helpers. Ship the licence text (<https://github.com/surrealdb/surrealdb/blob/main/LICENSE>). | All |

## 14. Dependencies (proposed; pin exactly; check `cargo deny`)

| Purpose | Crate | Version (date) | Licence |
| --- | --- | --- | --- |
| Transport, hole punching, relay client | `iroh` | 1.4.1 (2026-10-21) | MIT OR Apache-2.0 |
| Invite tickets | `iroh-tickets` | 1.0.0 | MIT OR Apache-2.0 |
| LAN discovery (pairing) | `iroh-mdns-address-lookup` | 0.6.1 | MIT OR Apache-2.0 |
| Presence | `iroh-gossip` | 0.102.0 (pre-1.0) | MIT OR Apache-2.0 |
| Bulk transfer | `iroh-blobs` | 0.104.1 (pre-1.0) | MIT OR Apache-2.0 |
| Signed logs, LogSync | `p2panda-core`, `p2panda-net`, `p2panda-sync`, `p2panda-store` | =0.7.1 | MIT OR Apache-2.0 |
| Roles and strong removal | `p2panda-auth` | =0.7.1 | MIT OR Apache-2.0 |
| Lane encryption | `p2panda-encryption` (data scheme) | =0.7.1 | MIT OR Apache-2.0 |
| Short-code PAKE | `spake2` | 0.4.0 (unaudited) | MIT OR Apache-2.0 |
| One-off drops, exports | `age` | 0.12.1 | MIT OR Apache-2.0 |
| Key storage | `keyring` (already present) | 4.2.0 | MIT OR Apache-2.0 |
| QR | `qrcode` (already present, owned by `sovra-whatsapp`) | 0.14.1 | MIT OR Apache-2.0 |
| Power, idle, system | `starship-battery`, `user-idle`/`system-idle-time`, `zbus`, `sysinfo` | see §11.5 | ISC / MIT |
| Fallback reconciliation | `negentropy` | 0.5.3 | MIT |
| Self-hosted relay (ops binary) | `iroh-relay` (server feature) | 1.4.1 | MIT OR Apache-2.0 |

**Notes**

- **QR codes:** `qrcode` is owned by `sovra-whatsapp` in `deps.rs`. Either
  extend the owner list, or move QR generation into a shared helper with a
  stated reason.
- **Do not use:**
  - `magic-wormhole`: EUPL-1.2 is not on the licence allow-list
  - `battery`, `cpace`: unmaintained
  - `tailscale-rs`: experimental, and needs a control server
  - `libtailscale`: Go toolchain
  - `boringtun`: needs a TUN device and admin rights
  - `openmls`: assumes an ordering server; decentralised variants weaken
    forward secrecy (<https://datatracker.ietf.org/doc/draft-kohbrok-mls-dmls/>)
  - `ucan`: no stable release since 2023
  - Willow: deletes old data by design, which clashes with history
- **Watch list:** `keyhive_core` 0.10, `beekem` 0.7 and `subduction_core` 0.21
  (Ink & Switch) have the strongest design but are labelled unstable
  (<https://github.com/inkandswitch/subduction>).

## 15. Milestones and to-do list

Each milestone ends with: `cargo xtask ci` green, unit tests in core, runtime
tests through `Client`, headless UI tests for new screens (at 640×420, 2560px
wide and 200% text), live verification through Development mode on **two dev
instances**, Skills-page cards, and CHANGELOG/README updates.

### T0 — Foundations (after stage 1 P1–P4 and P6)

Status checked against the code on 2026-11-10.

- [x] Decisions D1–D6 approved by the owner (2026-11-05): "Group",
  `sovra-mesh`, own envelope with p2panda behind it, Sovra-run relays,
  ULID/UUIDv7 and content-hash ids, SurrealDB as projection only.
- [x] Create the `sovra-mesh` crate (lib, `//!` doc, `[lints] workspace = true`),
  with its `LAYERS` and `EXTERNAL_OWNERS` rows in `xtask/src/deps.rs`.
- [x] Add `sovra-core/src/group/`. Pure, serde, no IO. It ended up as `keys`,
  `membership`, `fold`, `dag`, `knowledge`, `shared_artifact`, `pairing`, `sas`,
  `link`, `session` and `wire` instead of the five files first planned.
- [x] A `Spawn` handle from the runtime, so mesh tasks have owners and
  cancellation.
- [x] `RuntimeServices.groups: Option<GroupServices>` wired in
  `sovra-app/src/main.rs`. With no mesh configured, behaviour is unchanged.
- [x] Spike: two iroh endpoints in one test process connect directly and
  exchange messages (`sovra-mesh` tests). Binary size and compile time were
  not measured.

### T1 — Create a group (first user-visible milestone)

**Core**

- [x] `MembershipOp` types, `fold(ops) -> GroupState`, strong removal,
  remove-wins, the never-zero-admins rule (plus `needs_second_admin` for the
  UI banner) and fork detection. Built in `sovra-core/src/group/`, with
  unit tests and a convergence test over 300 shuffled orders with
  duplicates. *Not yet:* a second admin's co-signature for removing an
  admin.
- [x] Pairing state machine (`PairingStage` / `PairingStep`), single-use
  10-minute invites with a transcript-bound proof (`InviteBook`), the
  one-guess short-code window, and SAS derivation (4 emoji with spoken
  names, plus 6 digits).
- [x] `GroupCommand` / `GroupEvent` and the `GroupsView` reducer.

**Mesh**

- [x] iroh endpoint lifecycle:
  - device key kept in the OS keychain (dev builds: `dev-credentials/`)
  - n0 relays and key-based lookup (`Reach::Internet`), or direct only
    (`SOVRA_GROUP_REACH=local`). *Not yet:* a configurable relay URL.
- [ ] `sovra/pair/1`: the `sovra://join` link invite (§7.1) works. *Not yet:*
  QR code display, the short code with SPAKE2 (§7.2), and showing the SAS
  emoji check (§7.3) — link joins skip it.
- [x] Membership log storage: Sovra's own signed envelope, saved as
  `groups/group.json` (owner-only permissions) through the `GroupStore` port.
- [x] Admission check on every incoming request (`admits_request`).

**Runtime**

- [x] Group runtime covering create, invite, join, rename, role, remove and
  leave, with push on change and background sync every 20 s.
- [ ] Presence via gossip.
- [ ] Persist group state through the `MemoryStore` port. *Today:* a JSON
  file through `GroupStore`, which is simpler and enough for now.

**UI**

- [ ] Settings → Groups: Create, Roster, Add a computer, Add a person, link
  screen, Join screen, Remove dialogs with honest copy, and the second-admin
  banner are done. *Not yet:* QR code and the emoji confirm screen.
- [x] Development-mode ids on every control.

**Packaging**

- [ ] macOS Info.plist keys.
- [ ] Windows firewall rule in the Velopack hook.

**Skills page**

- [x] "Share with your family or team" (opens Settings → Groups).
- [ ] "Add another computer to my group" as its own card.

**Acceptance criteria** (not yet measured on two real computers)

- Two dev instances on different networks (one behind NAT) pair by link in
  under 30 s.
- On the same LAN, they pair by 6-digit code.
- Removal takes effect on all online computers within 10 s.
- Offline computers converge on reconnect. *(Covered by the runtime test
  with a switched-off computer.)*

### T2 — Share memory in lanes

- [ ] Lanes as encrypted spaces (p2panda-encryption data scheme): create, add
  and remove members, rotate on removal. *Today:* one whole-group lane,
  carried over encrypted QUIC and stored owner-only, without its own key.
- [x] Operation envelope plus sync: signed, hash-chained operations with
  Sync / Push, sent in rounds of at most 6 MB so large histories arrive.
- [x] Fold and conflict rules (§9.4) in core: replacement, corroboration,
  disputes, former members hidden, Viewers cannot add. Order-independence
  tests over shuffled orders.
- [ ] Retrieval: helpers get group details ranked by BM25 with agreement and
  dispute weights, and who shared them. *Not yet:* the SurrealDB lane filter,
  new-member damping, local re-embedding.
- [ ] UI:
  - [x] Share with… from Knowledge (reworded for the group, checked first)
  - [x] Who shared it, Agree, Correct, Withdraw, Erase the words
  - [ ] lane chips, Make private, which lanes each helper can read
- [ ] Rebuild-projection-from-log recovery tool (Settings → Repair).
- [x] Skills page: "Share something Sovra remembers with your group".
- **Acceptance criteria:**
  - Dad's shared fact answers Mum's question, with attribution. *(Runtime
    test.)*
  - Sam's private fact is never retrievable by Mum's helpers, as verified by
    test. *(Private facts never leave the computer; per-person lanes are T5.)*
  - Concurrent corrections produce a dispute card. *(Core test; the Groups
    list labels it "People disagree".)*

### T3 — Share helpers, skills, artifacts and schedules

- [ ] `HelperBundle` and `HelperAdopt` (§10). Signing, verification, preview,
  local grants, update diff.
- [ ] Skill folders as bundle files (`sovra-skills::LocalSkillStore`).
- [ ] Taint tracking in a turn, and confirmation cards for side-effecting
  tools in tainted turns (`sovra-adk` harness, runtime approval flow).
- [x] Artifact versions as ops (inline, up to 512 KB each; blobs later).
  Share with my group, Shared in <group>, Keep a copy, edits shared back as
  new versions, "two versions" kept when edits cross, Stop sharing.
- [ ] Group schedules (they depend on stage 1 P5): author-only execution.
- [x] Skills page: "Share a document or list with your group".
- [ ] Skills page: "Share a helper with your group".

### T4 — Hand off work

- [ ] Job protocol (§11.2) in `sovra-mesh`, with pure messages and scheduler
  in core.
- [ ] Capability advertisement in heartbeats: models from `/api/tags` and
  `/api/ps`, memory, AC, idle, queue, sharing policy.
- [ ] `ProviderKind::Group` plus a `ModelProvider` impl and an `Llm` forwarder
  (§11.4). Handle every compiler-listed match.
- [ ] Embedding and extraction offload (`HttpEmbedder`, `HttpSummarizer`
  destinations).
- [ ] UI:
  - "Help others in my group" policy
  - "Answered by…" chip
  - "Working for…" status with Stop
- [ ] Skills page: "Let a free computer do the heavy work".
- **Acceptance criteria:**
  - A laptop on battery gets a streamed answer from a plugged-in desktop.
  - Cancel stops generation on the worker within 1 s.
  - Kill the worker mid-stream and the job falls back locally without
    duplicating side effects.

### T5 — People-to-people and calm change

- [ ] *Me & Sam* lanes, plus `age` drops for one-off files.
- [ ] A "What's new in <group>" digest, and dispute resolution cards.
- [ ] Snapshots, prune points and storage budgets.
- [ ] A write-only *Contributor* role (e.g. kids add to the shopping list).

### T6 — Production readiness

- [ ] Sovra-operated `iroh-relay` (2+ regions) and a self-hosted relay guide.
- [ ] Optional **helper box**: run Sovra headless on an always-on computer
  that relays ciphertext and accepts jobs (the Ink & Switch "cloud peer"
  idea, <https://www.inkandswitch.com/essay/local-first/>).
- [ ] External security review: pairing, membership log, lane encryption,
  job protocol.
- [ ] Version negotiation tests across N−1 releases.
- [ ] A beta with 10 families and 3 small businesses. Measure pairing success,
  direct-path rate, battery impact and dispute frequency.

## 16. Testing strategy

- **Core:**
  - unit tests for every rule
  - property tests: fold convergence under permutation and duplication;
    strong-removal invariants; two-admin invariant
  - SAS test vectors
- **Mesh:**
  - in-process multi-endpoint tests: iroh endpoints bound to loopback with
    relays disabled
  - a fault-injection wrapper for packet loss, reordering and disconnects
- **Runtime:** tests through `Client` with two runtimes in one process (one per
  "computer"), asserting `Event::Group` sequences.
- **UI:** headless GPUI tests with `Client::loopback()` for every screen in §4,
  at the minimum window, a wide window and 200% text.
- **Live:** two `cargo xtask dev` instances with separate data directories,
  driven through Development mode (`docs/development-mcp.md`). Pair, share,
  hand off, remove, and capture evidence.

## 17. Open questions

*Answered on 2026-11-05:* the name is "Group" (D1). The log layer is Sovra's
own envelope with p2panda behind it (D3). Relays are Sovra-run with a
per-group override (D4).

Still open:

1. Relay operations detail: which regions, cost model, and the privacy policy
   for relay metadata (IP addresses, timing).
2. Should *Only me* sync between a person's own computers by default (it is
   very useful), or only once the person turns it on?
3. The minimum hardware to offer work, and the default idle threshold.
4. Per-lane cloud policy: should a group be able to say "never send our shared
   memory to a cloud model"? This is recommended: yes, as a lane setting.

## 18. Risks

1. **Pre-1.0 dependencies.** iroh-gossip, iroh-blobs and iroh-docs (0.10x) and
   p2panda (0.7) change roughly monthly. Mitigation: core ports, our own
   envelope, exact pins.
2. **Relay cost and operations.** Free relays are not for production.
3. **About 10% of pairs relay-only.** Large transfers there are slow, so they
   are scheduled over direct paths only.
4. **OS network permissions.** macOS and Windows behaviour is fragile, so
   pairing must not need the LAN.
5. **Memory poisoning across people** is the largest product risk. It is
   mitigated by provenance trust and taint, not by consensus. Consensus
   defences were beaten by flooding (<https://arxiv.org/html/2607.05029v1>).
6. **Revocation limits** must be explained honestly in the UI.
7. **Wire formats live forever.** Version and sign from day one.
8. **Battery impact** of always-on networking.
9. **Linux keychains** missing on minimal desktops.
10. **Scope creep** into real-time co-editing or model sharding. Both are
    explicitly out of scope.

## 19. Sources

Networking and pairing
- iroh v1 announcement <https://www.iroh.computer/blog/v1> ·
  relays <https://docs.iroh.computer/concepts/relays> ·
  discovery <https://docs.iroh.computer/concepts/discovery> ·
  local lookup <https://docs.iroh.computer/connecting/local-address-lookup> ·
  repo <https://github.com/n0-computer/iroh> · gossip <https://github.com/n0-computer/iroh-gossip>
- libp2p DCUtR measurement (70% ± 7.1%) <https://arxiv.org/abs/2604.12484>
- Syncthing security and introducer <https://docs.syncthing.net/users/security.html>, <https://docs.syncthing.net/users/introducer.html>
- Keybase key exchange and sigchain <https://book.keybase.io/docs/crypto/key-exchange>, <https://book.keybase.io/docs/teams/sigchain>
- Apple keychain circle <https://support.apple.com/guide/security/secure-keychain-syncing-sec0a319b35f/web>
- Tailnet Lock <https://tailscale.com/kb/1226/tailnet-lock>
- Magic Wormhole <https://magic-wormhole.readthedocs.io/en/latest/welcome.html>
- Signal safety numbers <https://signal.org/blog/safety-number-updates/> · KDE Connect CVE-2020-26164 <https://seclists.org/oss-sec/2020/q4/169>
- RustCrypto PAKEs <https://github.com/RustCrypto/PAKEs>
- macOS Local Network TN3179 <https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy> · mDNS in the enterprise <https://techcommunity.microsoft.com/blog/networkingblog/mdns-in-the-enterprise/3275777>
- LM Studio LM Link <https://lmstudio.ai/docs/lmlink> · LocalAI distribution <https://localai.io/docs/features/distribute/> · exo <https://github.com/exo-explore/exo>
- llama.cpp RPC <https://github.com/ggml-org/llama.cpp/blob/master/tools/rpc/README.md> · Ollama API <https://github.com/ollama/ollama/blob/main/docs/api.md>
- Exposed Ollama servers <https://blog.leakix.net/2026/02/ollama-exposed/>
- Ink & Switch, Local-first software <https://www.inkandswitch.com/essay/local-first/>

Memory, sync and permissions
- p2panda: core <https://docs.rs/p2panda-core/latest/p2panda_core/> · net <https://docs.rs/p2panda-net/latest/p2panda_net/> · auth <https://docs.rs/crate/p2panda-auth/0.7.0/source/README.md> · group encryption <https://p2panda.org/2025/02/24/group-encryption.html> · releases <https://github.com/p2panda/p2panda/releases>
- Keyhive notebook <https://www.inkandswitch.com/keyhive/notebook/> · Subduction <https://github.com/inkandswitch/subduction>
- DMLS draft <https://datatracker.ietf.org/doc/draft-kohbrok-mls-dmls/>
- SurrealDB changefeeds <https://surrealdb.com/docs/learn/querying/real-time/changefeeds> · licence <https://github.com/surrealdb/surrealdb/blob/main/LICENSE>
- Graphiti temporal model <https://mintlify.com/getzep/graphiti/concepts/temporal-model>
- HLC paper <https://cse.buffalo.edu/~demirbas/publications/hlc.pdf>
- Negentropy / NIP-77 <https://nips.4rs.nl/nips/77>
- Holochain validation <https://developer.holochain.org/concepts/7_validation/>
- Jazz groups and invites <https://jazz.tools/docs/react/groups/intro>

Safety and UX
- MINJA <https://arxiv.org/abs/2503.03704> · AgentPoison <https://neurips.cc/virtual/2024/poster/94715> · provenance trust <https://arxiv.org/html/2607.05029v1>
- CaMeL <https://arxiv.org/abs/2503.18813>
- Malicious skills study <https://www.arxiv.org/pdf/2602.06547> · Snyk ToxicSkills <https://snyk.io/blog/toxicskills-malicious-ai-agent-skills-clawhub>
- Shared smart speakers (CHI 2020) <https://lersse.ece.ubc.ca/?p=696> · GateMem <https://arxiv.org/abs/2606.18829> · Upwelling <https://www.inkandswitch.com/upwelling/>
- OpenAI memory FAQ <https://help.openai.com/en/articles/8590148-memory-faq>
