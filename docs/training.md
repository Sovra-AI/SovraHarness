# Train together on computers you already own

Sovra's scavenger grid lets computers in a group contribute short training steps
to a shared model. Each helper works independently; a slow or missing computer
does not stop the others. Training runs inside the native Rust app, with progress
saved on the computer that owns the run. There is no Python environment or
separate training service to install.

**The 0.5.0 preview trains a tiny next-byte practice model on CPU.** It does not
fine-tune a pretrained small language model (SLM), produce a replacement chat AI,
or change the model answering your chats. No chat provider or API key is needed.

## Try it on one computer

1. Sign in to your local Sovra account and open **Training** in the sidebar.
   You can also find training on **Skills** or in the guided tour.
2. Leave **Try the garden sample** selected for a first run. To choose your own
   source, select **Use my text** and paste 32 bytes to 64 KB of text.
3. Select **Start training**. This starts a local run without sharing it.
4. Wait for saved progress to appear. The page reports completed rounds and a
   learning loss value. A lower loss on the current training batch does not
   establish general language ability.
5. Choose **Pause** to stop after the current short step, then
   **Resume saved training** to continue. A finished run remains saved.

Training gives chat priority between short steps. It is not an operating-system
idle detector and does not yet provide CPU, power or temperature budgets.

## Train with your group

Use compatible Sovra versions with this preview on all participating computers.
First [create or join a group](groups.md#start-or-join). Keep Sovra open on the
computer that owns the run and on any computers contributing work.

1. On the owner computer, open **Training** and select sample or pasted text
   that you intend to share with group members.
2. Select **Start with my group**. This starts a new run and enables
   **Help the group train** for that run.
3. On another group computer, open **Training**, use **Refresh computers** if
   needed, find the owner's computer and
   select **Help this computer**. Helping preserves that computer's own saved
   training run; it does not replace it with the group model.
4. Wait for the helper to report work and the owner to show saved progress.
   Selecting a button alone does not mean a training result has been received.

The owner also trains locally. Helpers receive bounded batches and model
parameters, compute updates and send them back. The owner validates and saves
accepted updates before acknowledging them. Helpers need not finish in lockstep.
The group has no demonstrated speedup or production-quality model result yet.

### Pause or stop

| Control | Effect |
| --- | --- |
| **Pause** on the owner | Pauses local training and rejects remote work while paused. The current short step can finish first. |
| **Resume saved training** | Continues a paused run. An existing sharing grant can resume within the same app session unless it was revoked. |
| **Stop sharing** | Refuses further assignments and merges; the owner can continue training locally. |
| **Stop helping** | Ends this helper's participation after the bounded exchange or short step already in progress. An update already sent can have been accepted. |
| **Groups → Borrow an AI → Help the group train**, off | Revokes training sharing. Switching it on alone does not grant access to a local-only run. |
| Close and reopen Sovra | Restores unfinished training paused, with sharing and helping off. Group membership does not restart training. |

If a helper is waiting, check that the owner is online, has used
**Start with my group**, has not paused, and still allows training. A helper
that sees its own final result accepted stops when the run is complete. If
another computer finishes the run first, a waiting helper currently needs
**Stop helping**.

## What is shared

**Group training sends the chosen source text and model to participating
computers.** Encryption protects the connection; the receiving computer can
read its training batches. This is not a promise that training data stays on
the owner's machine. Updates can also reveal information about their source.

- Only the text selected for the run is used. Chats, saved memories, shared
  notes and local files are not automatically included.
- Training, answering group questions and lending a chat AI have separate
  permissions. A member must explicitly choose to help.
- Peer work is constrained training data and parameters, not arbitrary commands
  or programs to execute on the helping computer.
- Stopping sharing or removing a member prevents subsequent authorized work;
  it cannot retract text or model data the member already received.
- Group connections may use encrypted public relays. For direct local reach,
  follow [Groups networking](groups.md#network) on every participating computer.

## Saved progress

The Training page shows the saved-file location. Runs live under `training/`
inside the [app data directory](configuration.md#data-location):

- `run-<id>.json` contains the chosen text, model, optimizer state and progress.
- `latest.json` points to the latest run. Starting another run keeps earlier
  files, but there is no saved-run picker yet.

Checkpoints are written atomically after short steps; group assignments and
accepted updates are saved before delivery or acknowledgement. Only one app
instance can own a training directory at a time. Keep backups private: these
files contain the training text, and they are not an encrypted secret store.
Close Sovra before copying them. There is no supported checkpoint migration or
one-click restore flow; retain originals when testing recovery.

## Implementation and current limits

The preview uses Burn's native CPU backend, a fixed byte vocabulary and a small
model. Pretrained SLM import, tokenizers, useful model evaluation, resource
budgets and fleet performance measurements remain work to do. The default
desktop build does not offer GPU training controls.

For work leases, merge validation, research references and integration tests,
see [Swarm internals](swarm.md). For other outbound traffic and local-operation
settings, see [Network access](network.md).
