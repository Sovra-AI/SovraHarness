# Audit and training internals

This is a developer reference for code under `sovra_core::swarm`, the mesh
adapter, and `sovra-train`. It is not a guide to a desktop training feature.

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
| `sovra-train::LinearModel` | Burn autodiff for a linear model with squared-error loss |

Source: [core modules](../crates/sovra-core/src/swarm/mod.rs),
[learning modules](../crates/sovra-core/src/swarm/learn/mod.rs), and
[training adapter](../crates/sovra-train/src/lib.rs).
The training adapter uses the CPU backend by default; its `gpu` feature enables
CubeCL/wgpu. It is a workspace member but is not a dependency of `sovra-app`.

## Integration and security limits

The desktop has no training screen, distributed training scheduler, or general
outbound-message gate using `EgressGate`. Ordinary messaging uses the separate
review and trusted-recipient rules in [Messaging](communications.md). Department
spaces in Groups do not establish a company-wide policy system.

Library tests exercise synthetic training rounds and protocol invariants. They
do not prove language-model fine-tuning quality, large-fleet performance, or
production differential-privacy guarantees. Secure noise requires an OS-backed
random source; deterministic test seeds are not suitable for private workloads.
The aggregation protocol's coordinator assumptions and survivor-list handling
must be reviewed before any deployment. Revoking training input cannot guarantee
that an already-trained model has forgotten it.

Run `cargo test -p sovra-core swarm` and `cargo test -p sovra-train` for the
relevant library tests. Full repository verification remains `cargo xtask ci`.
