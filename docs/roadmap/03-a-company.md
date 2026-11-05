# Stage 3 — A company (Direction)

> **Direction, not today.** A private fleet with ledger memory, lanes for each
> team, and a model trained on its own computers.

This is an outline. It is researched (2026-11-05) but intentionally looser than
[`02-a-team.md`](02-a-team.md), because what we learn in stage 2 will change it.
Treat the phases as a plan to revisit at the end of stage 2, not as a commitment.

## 1. What changes from a team to a company

| | Team (stage 2) | Company (stage 3) |
| --- | --- | --- |
| Size | 3–50 computers | hundreds to thousands |
| Identity | Invites, emoji check, recovery code | SSO (OIDC) + SCIM, device keys in TPM / Secure Enclave, MDM enrolment |
| Membership | Signed membership log, two admins | Directory groups become lanes. Policy is signed by IT. |
| Memory | Signed op logs per lane, peer catch-up | **Ledger memory**: per-lane Merkle logs, signed checkpoints, witnesses |
| Always-on nodes | Optional helper box | Probably required: a few **site nodes** per office or region |
| Search | Local index over replicated lanes | Fan-out to cleared nodes plus RRF fusion |
| Model | Each computer's chosen model | Optional **company adapters** trained by federated LoRA |

## 2. Ledger memory: a ledger without a coin

**Requirement.** The ledger needs:

- append-only order per lane
- tamper evidence anyone can check cheaply
- permissioned writers
- need-to-know replication
- no mining

The company runs the fleet, so a crash-fault-tolerant order plus cryptographic
auditability is enough. Byzantine consensus is only worth adding for multi-party
lanes, such as a joint venture.

**Design: one transparency log per lane.**

- Each lane is a **C2SP tlog-tiles** Merkle log
  (<https://github.com/C2SP/C2SP/blob/main/tlog-tiles.md>, RFC 9162). The
  log's **sequencer** is a small Raft group of site nodes using `openraft`
  (production in Databend, pre-1.0, <https://github.com/databendlabs/openraft>).
  For small lanes, one elected node is enough.
- Entries are stage 2 ops: signed by device keys, hash-linked, with encrypted
  payloads.
- The sequencer publishes **signed checkpoints** (tree size plus root).
  **Witnesses** (the security team, an auditor, optionally an outside party)
  co-sign them, using C2SP signed-note and tlog-cosignature, so no one can be
  shown a different history. This is the pattern behind Trillian Tessera and
  Sigstore Rekor v2 (GA 10 Oct 2025,
  <https://blog.sigstore.dev/rekor-v2-ga/>, <https://github.com/transparency-dev/trillian-tessera>).
- A **company root log** records lane checkpoints, policy changes and
  membership changes, so one root covers everything.
- **Every node verifies in the background.** It checks consistency proofs on
  every checkpoint and alerts if one fails. Amazon QLDB's shutdown on 31 July
  2025 showed that verification nobody runs, in one vendor's format, is a
  liability (<https://www.infoq.com/news/2024/07/aws-kill-qldb>). Use an
  open format and automatic checks.
- **Erasure without breaking the ledger.** Put only the hash on the shared log
  and keep the payload in the lane, where it can be purged. This is
  Hyperledger Fabric's private data collections pattern
  (<https://hyperledger-fabric.readthedocs.io/en/latest/private-data/private-data.html>).
  It maps onto stage 2's `Redact`: the purge and a tombstone are new entries,
  and the hash stays.

**Rust building blocks to evaluate:**

- `openraft`
- Cloudflare's Azul (a Rust Static CT / tlog-tiles implementation,
  <https://github.com/cloudflare/azul>; crate names unverified)
- `sigstore` (sigstore-rs)
- `tough` (TUF)
- `ed25519-dalek`
- iroh for transport

## 3. Fleet management

- **Enrolment via MDM.** Intune, Jamf and Kandji push managed app
  configuration:
  - macOS: managed preferences for the bundle id
    (<https://learn.microsoft.com/en-us/mem/intune/configuration/preference-file-settings-macos>)
  - Windows: ADMX policies under `HKLM\SOFTWARE\Policies`

  Sovra reads an enrolment token, the company root of trust and endpoints
  **only from managed configuration**, never from user-writable files. Velopack
  installs and update rings are pinnable by policy.
- **Identity.**
  - People sign in with OIDC (Okta, Entra ID). SCIM 2.0 (RFC 7644) provisions
    and deprovisions users and groups. **Directory groups map to lanes.**
  - The device key is non-exportable (Secure Enclave, TPM 2.0).
  - A company enrolment service or on-prem CA (for example step-ca, or ACME
    device attestation) issues short-lived device certificates that bind
    person, device and lane grants. MDM compliance gates issuance.
- **Policy** is signed documents on the company root log, enforced on each
  node. It covers allowed models and providers, cloud yes/no per lane,
  retention, and export rules.
- **Revoking access.** A SCIM deprovision or MDM retire does three things:
  1. writes a revocation entry
  2. rotates the lane keys
  3. sends a wipe command to the app's lane data

  Be honest that data already on a stolen laptop cannot be pulled back. Short
  certificate lifetimes and disk encryption are the real defences.
- **Audit export.** OCSF events go to Splunk HEC, Microsoft Sentinel (Logs
  Ingestion API) and Elastic, with checkpoint hashes so the SIEM copy can be
  verified (<https://schema.ocsf.io/>).

## 4. Search across the company

- Each node searches its local index, and the requester fuses results with
  **RRF** (`fuse_rrf` already exists in core, Cormack et al. 2009,
  <https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf>).
- **Routing.** Never fan out to thousands of laptops. Route only to lanes the
  person is cleared for, and use **lane index summaries** on site nodes (term
  and topic sketches, Bloom filters, centroids) to pick the top-k nodes.
  Return partial results on timeout.
- **Privacy of queries.**
  - The responder checks clearance too.
  - Snippets are filtered by the *requester's* permissions.
  - Queries are logged on the lane ledger, transparently.
  - Results from a "no cloud" lane never reach a cloud model.
  - PIR and private set intersection are research-grade, so don't promise
    them.
- **Target.** A p95 under 2 s across 500 nodes in a pilot.

## 5. Data residency

- Lane placement policy, for example "EU lane data stays on EU nodes", is
  enforced by replication rules and logged on the root log, so a **residency
  report** can be generated from the ledger itself. Local-first is the
  strongest possible answer to residency laws (see stage 4).

## 6. The company model (federated fine-tuning)

**Decide whether it is needed before building it.** RAG over shared lanes plus
per-lane prompts may deliver most of the value. Start with **federated
evaluation** to measure first.

- **Method.**
  - Federated LoRA. Naive FedAvg of LoRA A and B is inexact, so use
    **FFA-LoRA**, which freezes A and trains B, for exact aggregation, half the
    traffic and better behaviour under DP noise
    (<https://iclr.cc/virtual/2024/poster/18792>).
  - Add **FlexLoRA** for mixed ranks on weaker machines later
    (<https://arxiv.org/abs/2402.11505>).
  - Use **per-lane adapters** (legal, support, engineering) instead of one
    global model. They respect need-to-know and avoid non-IID drift.
  - Use FedProx where drift appears (<https://arxiv.org/abs/1812.06127>).
- **Privacy.**
  - **Secure aggregation** with SecAgg+ (<https://eprint.iacr.org/2020/704>),
    so the server sees only the sum.
  - **Central DP** after aggregation.
  - At hundreds of clients (not millions) DP costs real quality. Expect ε of
    roughly 4–10 for usable results, and treat DP as defence-in-depth. Gboard
    and Apple use DP at far larger scale
    (<https://arxiv.org/abs/2305.18465>, <https://machinelearning.apple.com/research/learning-with-privacy-at-scale>).
- **Release gates.**
  1. Federated evaluation on held-out lane sets: scores only leave the node.
  2. A regression suite: safety, PII canaries, membership-inference probes.
  3. Staged rings.
  4. A **signed promotion entry on the ledger** with model hash, parent hash,
     lanes, ε, rounds and scores. This is **model lineage**.
  5. **One-step rollback.** Policy points at the previous adapter hash, and
     nodes keep the last N adapters.
- **Hardware reality.**
  - A 32 GB Apple Silicon Mac does QLoRA on an 8B model at about 100 tokens/s
    with packing (one author-reported benchmark,
    <https://pypi.org/project/mlx-train-perf/0.8.0/>), roughly 3 hours per 1M
    tokens overnight.
  - Most corporate Windows laptops can only handle 0.5–1.5B models, or
    evaluation only.
  - Plan for 10–30% of a fleet to be useful trainers. Train only on AC power,
    idle, cool and unmetered. Checkpoint often.
- **Implementation.**
  - Rust training (candle, burn) is immature. Train in a sandboxed sidecar:
    MLX on Mac, PyTorch/PEFT on CUDA.
  - Keep aggregation, the DP accountant, SecAgg, gating and the ledger in Rust.
  - Flower (flwr) is the reference framework, with FlowerTune benchmarks
    (<https://flower.ai/docs/>, <https://neurips.cc/virtual/2025/poster/121489>).
    Reimplementing its server side in Rust is feasible, because it is adapter
    aggregation.

## 7. Phases (to revisit after stage 2)

- [ ] **A0, managed install** (the bridge from team to company): managed
  preferences, OIDC SSO, SCIM group → lane sync, hardware-backed device keys,
  signed policy, OCSF audit export. *Exit:* a 50-seat pilot deployed by IT
  without touching each machine.
- [ ] **A1, ledger memory v1**:
  - per-lane tlog-tiles logs and a company root log
  - witnesses and background verification
  - purge-able encrypted payloads, key rotation on revoke
  - 1–2 site sequencers

  *Exit:* an external auditor verifies a lane's history from checkpoints alone.
- [ ] **A2, company search**: lane-scoped fan-out, site index summaries, RRF,
  permission-filtered snippets, query audit. *Exit:* p95 < 2 s at 500 nodes.
- [ ] **A3, data residency**: placement policy, enforcement, and a residency
  report from the ledger.
- [ ] **A4, company model**: federated **evaluation** first. Then per-lane
  FFA-LoRA on 1–3B models with SecAgg+, central DP, gates, lineage and
  rollback. *Exit:* a lane adapter beats the base model on lane evaluations,
  with a documented ε.
- [ ] **A5, scale-out**: 1–8B adapters, mixed ranks, cross-lane merging,
  external witnesses, and BFT lanes for multi-party use.

## 8. Risks

- Laptops sleep, so sequencers and training need always-on site nodes. That is
  a server product, and it changes how Sovra is sold.
- The Python training sidecar adds packaging and security cost.
- DP utility at small N may be poor. Federated LoRA may not beat good RAG, so
  measure first.
- GDPR erasure must be designed into the ledger from day one: hash on the log,
  payload purge-able.
- Fan-out search can feel like surveillance unless query audit is visible to
  staff.
- Stolen-laptop exposure.

## 9. Open questions

- Is a company model needed at all, or are RAG and per-lane helpers enough?
- Will customers accept a small on-prem site node, or must everything stay
  peer-to-peer?
- Which base models are licensed for fine-tuning and redistribution (Apache-2.0
  Qwen/Mistral vs community licences)?
- Who acts as the outside witness?
- What is the minimum "trainer" hardware?
