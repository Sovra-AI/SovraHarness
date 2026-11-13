# Audit and training internals

This is a developer reference for code under `sovra_core::swarm`, the mesh
adapter, and `sovra-train`. It also describes the early native Training preview.

## Integrated group audit

The desktop records group sharing and federated memory access in a signed,
hash-chained history. Group synchronization replicates checked copies, detects
conflicting histories, and uses signed checkpoints and peer witnessing.
**Groups → History** shows **History on this computer**. See [Groups](groups.md)
for the visible controls and limits.

Implementation: [audit rules](../crates/sovra-core/src/swarm/audit.rs),
[mesh adapter](../crates/sovra-mesh/src), and
[group runtime](../crates/sovra-runtime/src/groups.rs).

## Library components

| Component | Implemented responsibility |
| --- | --- |
| `swarm::department` | Department directory, membership, custody filters, and separate cloud/training consent |
| `swarm::ledger` and `swarm::merkle` | Signed ledger entries, Merkle roots, checkpoints, witnesses, and residency/model lineage folds |
| `swarm::egress` | Approvals bound to exact content hashes, checked and released once |
| `swarm::learn::galore` | Low-rank gradient projection and optimizer state |
| `swarm::learn::scaffold` | Local objective interface and control-variate training rules |
| `swarm::learn::privacy` | Discrete-Gaussian noise and zCDP accounting |
| `swarm::learn::secagg` | Masked aggregation and dropout handling |
| `swarm::learn::round` | Round planning, contributions, aggregation, and ledger events |
| `swarm::learn::scavenger` | Bounded independent work leases, token-weighted age-gated Adam merges, validated optimizer checkpoints |
| `sovra-train::ByteLanguageModel` | Burn next-byte cross-entropy/autodiff workload for CPU integration tests |
| `sovra-train::LinearModel` | Burn autodiff for a linear model with squared-error loss |

Source: [core modules](../crates/sovra-core/src/swarm/mod.rs),
[learning modules](../crates/sovra-core/src/swarm/learn/mod.rs), and
[training adapter](../crates/sovra-train/src/lib.rs).
The training adapter uses the CPU backend by default; its `gpu` feature enables
CubeCL/wgpu. The composition root links it into `sovra-app` through the core
`TrainingEngine` port; the runtime and UI have no Burn dependency.

## Scavenger-grid training: integrated preview

For first use, group setup, stopping controls and data handling, follow the
[Training guide](training.md). This section explains the implementation and
separates the working tiny-model preview from remaining pretrained SLM work.

Open **Training** (or **Skills → Teach a small model with your text**). Choose
**Try the garden sample** or **Use my text**, then **Start training**. The preview
accepts 32 bytes to 64 KB of pasted UTF-8 text. **Pause** finishes the current short
round; **Resume saved training** continues the same model and optimizer. Closing
Sovra stops work; reopening restores a paused run and never starts training by
itself. **Copy saved file location** locates the model checkpoint. A new run keeps
the earlier run's file.

This is a tiny next-byte practice model, not a chat model or pretrained SLM
fine-tuner. The page says so explicitly. **Start training** keeps work on this
computer. **Help the group train**, in Groups, starts off each time Sovra opens.
Answering questions or lending an AI does not
turn it on, and membership alone does not start training or send work. **Start
with my group** turns that choice on and shares small batches of the chosen text
and the practice model with current group members who choose to help. A helper
that disconnects before a valid update does not change the saved model, and that
same step can be assigned again. Text is also included in the saved file. Chat
history, memories and other group data are not ingested. New background rounds
wait while Sovra's chat model is busy; this is not operating-system idle detection.

Create or join a group in **Groups**, then use **Refresh computers** in Training.
On a second computer, choose **Help this computer** next to the job's owner.
**Stop sharing** prevents further assignments and merges while local training
continues. **Stop helping** stops after any bounded network/CPU operation already
in flight; an update already sent can still be accepted. Leaving the group also
stops helping. Your own saved model is preserved while you help someone else.
Sharing and helping stay off after restarting. **Pause** stops both local and
remote training work. A paused or offline host makes helpers wait and retry;
they can stop at any time. Helping finishes when it receives an acknowledgement
that completes the job; if another worker finishes it first, a waiting helper
currently needs **Stop helping**. The **Groups → Borrow an AI → Help the group
train** switch can revoke permission. Turning it on does not share a local-only
run: select **Start with my group** to choose the text and grant access for that run.

### Scheduling primitive
`GridSyncer` assigns short work leases to independent workers. A ready worker can
advance training without waiting for offline workers. Each lease binds the job,
worker, base model hash, round, step count, token budget and expiry. The caller
supplies monotonic time; expired slots can be reused. Leases are consumed once,
and altered assignments, duplicate submissions, invalid dimensions, non-finite
values and out-of-budget counts are rejected before any merge state changes.

The outer optimizer clips each pseudo-gradient, applies a cosine/exponential age
gate, and token-weights the available updates before updating Adam's moments.
Updates at or beyond the configured age cutoff are rejected without changing the
moments. This is a whole-model prototype capped at 16,777,216 parameters and 64
simultaneous workers, not yet a fragmented optimizer for billion-parameter SLMs.
The defaults are development settings, not a validated training recipe.

A serializable checkpoint contains parameters, moments, bias-correction powers,
round, token count and the next work ID. Restore validates the checkpoint and
invalidates in-flight leases, so restarted workers must fetch the current model.
The runtime must persist that checkpoint atomically **before** acknowledging an
accepted merge or assignment. The local runtime writes owner-private files through a temporary file, sync and atomic
rename, and only then publishes progress. Each new run has its own file; a small
index points to the most recent run. An OS-held lock excludes another training
runtime from that directory. Optimizer floats use exact JSON round-trip parsing.
Old checkpoint rollback is not protected by this primitive.

`ByteLanguageModel` uses Burn 0.22.1's cross-entropy and autodiff with a fixed
256-byte vocabulary. Batches are limited to 4096 next-byte predictions. Tests
exercise real local gradients, unequal participation, a disappearing worker and
coordinator restore, and assert lower language loss. It is a 65,536-parameter
bigram test workload, not a transformer, pretrained-model fine-tuner or chat model.
The test inner loop uses SGD; AdamW learner state remains to be integrated.

### Independent learner packets

The core `TrainingTask` and `TrainingResult` wire types split assignment, local
work and merge. A task carries a versioned built-in recipe, a lease, little-endian
model parameters and at most 512 bytes of explicitly selected text. Workers
validate the authenticated worker identity, base-model hash, finite parameter
values, exact model size, token budget and a maximum of 16 local steps before
allocating Burn tensors. No task can request a command, file read or download.

New runs use independent random identities, even when their text is identical.
The coordinator remembers the exact batch/token count for each outstanding lease.
Duplicate, altered, expired and stale updates cannot change a saved model. A
restored coordinator invalidates outstanding work and advances the lease counter;
workers must ask again. Earlier local checkpoints are still readable and upgrade
on their next save.

`PeerRequest::Training` uses the existing authenticated Groups protocol. The
group wire already carried this request. Core and runtime membership checks
cover these messages, and the desktop serves one only when **Help the group
train** is on. Otherwise it refuses and does not assign work. Helping is a
separately owned, cancellable runtime task. The coordinator's bounded inbox
rejects excess requests. It saves the lease counter before sending an assignment
and writes the model only after a matching finite result. A changed worker,
model, size, token count or non-finite update is rejected and leaves the saved
run unchanged. A helper that asks again after dropping out replaces its own
unfinished lease, so the old update cannot be merged. It checks membership again
before releasing a reply. Network exchanges use the Groups deadline; learner
work has fixed size/step caps.

Integration tests pass real Burn work over local iroh connections, including a
lost worker, unauthorized sender and replay. Two full desktop runtimes driven
through `Client` test consent, durable acknowledgements, stopping, group departure,
preservation of a helper's own model and coordinator restart with sharing off.
These run in one test process; cross-process interruption/rejoin testing and
large-fleet performance remain to be established.

### Implementation path

1. **Present:** pure scheduling/outer optimizer, Burn language workload, bounded
   local rounds, durable model/optimizer/text checkpoints and runtime progress.
2. **Present:** native Training page with sample/pasted-text choice, Start, Pause,
   Resume, progress and saved-model location; registered on Skills.
3. **Next:** larger dataset ingestion, idle/resource budgets, better model
   evaluation and saved-run selection. Pause currently occurs between short rounds.
   The learner uses SGD; persistent AdamW inner state is still pending.
4. **Grid integration:** bounded learner packets, the Groups switch **Help the
   group train** (off until the owner turns it on), durable assignment and merge
   acknowledgements, and a dropped helper that does not change the saved model.
   Next are resource budgets and reconnect tests across separate machines.
5. **SLMs:** transformer/pretrained model support, tokenizer/model artifacts,
   AdamW local checkpoints, fragments and measured memory/quality benchmarks.

The intended delivery stays within the Rust executable: Burn owns tensor code,
core owns decisions, runtime owns tasks and IO, and GPUI owns the native controls.
Python, Ray, Slurm, a separate trainer service and a browser UI are not dependencies.
Model weights, selected text and checkpoints are data files in the app data
directory’s `training/` folder. The training adapter is linked into `sovra-app`; no
separate training process is started.

### Research used

- [Decoupled DiLoCo paper](https://arxiv.org/abs/2604.21428) and
  [DeepMind overview](https://deepmind.google/blog/decoupled-diloco/): independent
  learners and infrequent exchange motivate avoiding a fleet-wide barrier.
- [Cosine-Gated Adam-Decay](https://arxiv.org/abs/2605.09126): gates incoming
  pseudo-gradients before Adam's moment buffers. Reported experiments and idealized
  bounds do not guarantee convergence or goodput for this application.
- [iroh authentication and protocol handlers](https://docs.rs/iroh/1.4.1/iroh/protocol/index.html):
  reuse the pinned Groups transport and authenticated connection identity; peer
  identity alone does not authorize training or sharing text.
- [Burn 0.22.1](https://docs.rs/burn/0.22.1/burn/): reuse the already-pinned Rust
  tensor/autodiff library with the pure-Rust Flex CPU backend. APIs were checked
  against the installed pinned source, including its cross-entropy implementation.

## Integration and security limits

The desktop's connected training preview uses only the fixed small-model recipe;
it has no general outbound-message gate using `EgressGate`. Ordinary messaging uses the separate
review and trusted-recipient rules in [Messaging](communications.md). Department
spaces in Groups do not establish a company-wide policy system.

The scavenger primitive does not authenticate peers, authorize training, or apply
the separate secure-aggregation/privacy protocol. Model changes can leak training
data; keeping raw examples local alone is not a privacy guarantee. Transport must
bound messages before decoding or allocating tensors.

Library tests exercise synthetic training rounds and protocol invariants. They
do not prove language-model fine-tuning quality, large-fleet performance, or
production differential-privacy guarantees. Secure noise requires an OS-backed
random source; deterministic test seeds are not suitable for private workloads.
The aggregation protocol's coordinator assumptions and survivor-list handling
must be reviewed before any deployment. Revoking training input cannot guarantee
that an already-trained model has forgotten it.

Run `cargo test -p sovra-core swarm` and `cargo test -p sovra-train` for the
relevant library tests. Full repository verification remains `cargo xtask ci`.
The real mesh fixtures share OS multicast discovery, so `.config/nextest.toml`
isolates those fixtures using a [Nextest test group](https://nexte.st/docs/configuration/test-groups/).
Each fixture still runs its peers together, and all tests and assertions run.
