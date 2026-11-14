# Groups of people and computers

The **Groups** tab connects the computers in a family or a small team and
lets each person explicitly share details, memories, documents, helpers,
skills and schedules that the helpers on those computers can use — with the
whole group or just one person — ask each other's memories, and let a computer
with no AI use a free one's. A **group** is those people and their computers. A
[team](teams-and-artifacts.md) is the AI helpers inside one chat. A WhatsApp
group, described in [messaging](communications.md), is a chat id (`…@g.us`)
and a subject; it is never turned into a saved person and it is never answered
automatically.

## Start or join

1. Open **Groups**. A computer can belong to one group at a time.
2. Choose **Start a new group**, then enter the group name, your name, and the
   name of this computer. Alternatively choose **Join a group** and paste an
   invitation link or enter the same-room code.
3. Keep the inviting computer online until joining finishes. Same-room pairing
   asks both people to compare the pictures and digits, then confirm they match.
4. Use the **Getting started** list to invite someone, share a note, and make a
   second admin if appropriate.

An admin can create an invitation link under **Invite someone**. It is copied
for sharing privately and also shown as a QR code. Links are single-use and
expire after ten minutes. Same-room codes allow one attempt within two minutes.
**Add another of my own computers** associates another computer with your identity.
Restarting the inviting app invalidates outstanding invitations.

## Share work

- In **Shared notes**, write a detail, choose **Everyone**, a person, or a smaller
  circle, then press **Share**. Wait for confirmation before assuming it is saved.
  The draft stays available on failure; **Write another note** clears a confirmed draft.
- In **Knowledge** or **Artifacts**, use the sharing action on the selected item.
  Review the audience. Others can use **Keep a copy** for shared artifacts.
- On **Shared helpers**, review shared helper/skill instructions before using them.
  **Add to my schedules** copies a shared schedule to this computer.
- **Smaller circles** lets admins choose a subset of members. The code calls these
  departments. Only members of that circle receive its new encrypted records.

Sharing a newer version preserves its original audience. Offline computers receive
changes on reconnect. **Stop sharing** and deletion use confirmation and wait for
saved results. Copies already kept separately are not recalled by these actions.
Private chat history and files are not automatically copied to Shared notes.

## Ask the group or borrow an AI

**Ask my group** sends a question to participating computers. **Answer questions
from my group** lets this computer answer from its memory; **Ask the group while
I chat** permits asking during chat. Both switches start off. Questions answered
by this computer are visible in the group history/requests views.

**Borrow an AI** has separate lending and borrowing switches. A computer without
an available model can hand off chat to a willing peer. The peer receives the
conversation text and answers without its private tools. This does not hand off
embedding or indexing jobs. The relevant computers must be online.

## People, leaving, and history

**People** lists members, roles, computers, and presence. Admins can change Admin/
Member roles or remove people/computers after confirmation. The last admin cannot
leave other people without an admin: appoint another first. If you are alone,
leaving ends the group. Leaving removes all computers belonging to your person.

Removed computers stop receiving new group data after applying the updated
membership. This cannot erase copies they retained elsewhere. **History** shows
signed records of sharing and federated memory access. Records contain content
hashes rather than the shared words. Checked peer copies and witness signatures
help expose conflicting histories; they do not prove that shared claims are true.
See [Audit internals](swarm.md).

## Words

| Word on the screen | What the code calls it | Meaning |
| --- | --- | --- |
| Group | `GroupId` | One set of people and their computers. |
| Person | `PersonId` | A human. Several computers can belong to one person. |
| Computer | `DeviceId` | One install of Sovra. Its network identity and its signing key are the same public key. |
| Admin, Member | `Role` | What the roster currently allows. See [Who may change what](#people-leaving-and-history). |
| Shared detail | `SharedDetail` | One paragraph someone chose to share, plus its history. |
| Team | `sovra_core::teams` | AI helpers in a chat. Unrelated to this roster. |
| WhatsApp group | `SyncedGroup` | A messaging chat. `ConnectionEvent::GroupsChanged` updates that address book. An empty snapshot does not wipe saved people. It does not create or update a Groups roster. |

`Role` also contains **Viewer** ("can read") and **Contributor** ("can add,
can't read"). The Groups screen never assigns those two. The knowledge fold
checks `Role::writes` for additions and rejects Viewers. `Role::reads` is not
a separate shared-text read gate, so Contributor must not be described as an
enforced write-only role. Admin checks are listed under [Who may change what](#people-leaving-and-history).

## Where a group's data lives

The composition root in `crates/sovra-app/src/main.rs` decides this once at
startup.

| Piece | Where | What happens if it fails |
| --- | --- | --- |
| Device secret | Credential entry `group-device-key`, 32 raw bytes stored as hex | Groups are not started. Release builds use the OS keychain. A debug build launched with `SOVRA_DEV=1` (`cargo xtask dev`) uses `dev-credentials/` in app data, owner-only on Unix. |
| Group record | `<app data>/groups/group.json` | A missing file is an empty record. A corrupt file is logged and the app starts with no group. |
| Outstanding invites | Memory of the running app only | Restarting the inviting app burns every invite that was still on screen. The pasted link then fails as unknown. |
| Reach | `SOVRA_GROUP_REACH` | `local` is direct connections only. Any other value, including unset, is Internet reach. Read at launch. See [configuration](configuration.md). |

App data paths are in [configuration](configuration.md). On Linux the usual
root is `$XDG_DATA_HOME/sovraharness` or `~/.local/share/sovraharness`.

A new shared note, artifact or helper is saved before it is published to peers
or exposed to helper recall. If saving fails, the attempted share, new key grants
and audit entry are rolled back and an error asks you to retry.

`group.json` is rewritten as a whole. The mesh store writes
`group.json.tmp` with mode `0600` on Unix, syncs it, then renames it over
`group.json`, so a crash mid-write does not leave a half record. Copying
`groups/` without the credential entry does not move the computer's identity:
the secret never sits in that JSON file.

The record (`GroupRecord`) holds this computer's person id and names, the
group id, the full membership log, the full knowledge log, and the last
address learned for each peer. Invites are not in it.

If the credential store cannot keep the device key, or the network endpoint
cannot bind, the Groups worker accepts **Load** and ignores it. The screen
then stays on "Getting your group ready…". The words for a missing keychain
exist on `GroupProblem::NoKeychain` and are not emitted today. A later
command, if one were sent, would say "Groups aren't available in this session."
or "Groups can't connect on this computer right now: …".

## Protocol and authorization

The UI sends `Command::Group` and folds `Event::Group` updates. Runtime tasks own
storage, synchronization, cancellation, and peer requests. Core folds signed
membership and knowledge operation histories; the mesh adapter supplies crypto,
network transport, and storage. Peer identities are device public keys.

The membership fold resolves roles, removals, concurrent changes, and conflicting
histories. Group, two-person, and department spaces have separate lane keys.
Membership changes rotate keys for subsequent records. Key rotation does not
retract plaintext or keys already held by a former member.

Shared evidence is attributed to its author and treated as untrusted data by
helpers. Sharing does not grant terminal permission, arbitrary tools, or control
of another computer. Domain rules and exact wire limits are in the source map
below; the public UI exposes only the controls described in this guide.

## Train together

A group's computers can train a small AI model together, and every member
receives the finished model. The full steps are in the
[Training guide](training.md#train-with-your-group); this section covers the
group's side.

- **Starting a shared run.** On **Training**, the person who owns the run
  leaves **Train with my group** on and presses **Start training**. That choice
  is made for each run and is never saved: after a restart, nothing is shared
  until a run is started with the group again.
- **Helping.** Under **Your choices for this computer**, **Help the group
  train** lets this computer help a run a group member started with the group.
  It is off until you turn it on and it is remembered across restarts. With it
  on, this computer helps by itself when it is not training its own model and
  chat is not busy. **Stop helping** on the Training page stops until Sovra
  restarts or the switch is turned off and on again.
- **Receiving the model.** Every current member's computer receives a finished
  shared model, whether or not **Help the group train** is on. Computers that
  were away fetch it when they are next online. A received file is checked
  against its fingerprint before it is kept, and it is never run as a program.
- **What it does not unlock.** Joining a group, **Answer questions from my
  group** and lending this computer's AI do not start training, send training
  text or make this computer help. The helper switch does not expose a run
  that was started without the group.
- **Membership is checked both ways.** Each training request is checked
  against the current member list before it is sent and before it is answered.
  A removed computer is refused and stops helping without being told to.

The owner sends helpers short pieces of the chosen text with the current
model, over the group's encrypted connection. Helpers can read those pieces.
Training messages are not sealed with the group's space keys, and they are not
written to the group's signed history. See
[Training: privacy and consent](training.md#privacy-and-consent).

Computers on Sovra 0.5.0 cannot receive models and can help only a Starter run
on its recommended settings. Use the same Sovra version across the group.

## Network

`sovra-mesh` is the only crate that names iroh. `Mesh::bind` builds an
endpoint with ALPN `sovra/group/1`.

| Reach | How it is selected | Behaviour |
| --- | --- | --- |
| Internet | Default. Any `SOVRA_GROUP_REACH` other than `local`, including unset. | iroh preset `N0`: discovery and n0's public relays. Encrypted traffic may pass through a relay. This is a development reach, not a production relay commitment. |
| Local only | `SOVRA_GROUP_REACH=local` before each participating app starts. | iroh preset `Minimal`: direct connections, no relays, no address lookup. |

The endpoint's current relay URL and IP addresses are what a new link
embeds. Hole punching and relay fallback are iroh's, on the Internet preset.
There is no Sovra server in the path, and there is no promise that packets
stay on the local network unless every participant was launched with `local`.

## Limits of this foundation

Shipped today: one group per computer, signed membership, one-time ten-minute
links with QR codes, same-room codes with the picture check, peer sync and
push in rounds, encrypted spaces with key rotation, two-person spaces, shared
details, memories, artifacts, helpers, skills and schedules, agreement,
withdraw, erase, helper recall of usable details and artifacts with
attribution, asking each other's memories, presence, and chat work handoff.

Current boundaries between protocol support and screen controls:

| Capability | State |
| --- | --- |
| Viewer and Contributor | The fold refuses additions from Viewers. No screen control sets either role yet. |
| Correct a detail from the screen | `GroupCommand::Correct` and the dispute fold exist. The screen has no correction button, and Share does not set `replaces` or `valid_at`. |
| Handed-off answers | Streamed, stoppable within a second, handed to the next free computer if the helper vanishes first. Chat only: no embedding or indexing jobs (another computer's vectors are not comparable), and no tools on the helping computer. |
| Same-room codes | Need both computers on one network (mDNS). No SPAKE2: one guess per code plus the picture check. |
| A second admin's extra signature to remove an admin | Not required. One admin's signature is enough. The last admin cannot be removed while other people remain, or demoted. |
| Production relays and a relay URL you configure | Internet reach uses n0's public relays. `local` turns relays off. There is no separate relay-URL setting. |

Removed computers stop receiving new operations once they are off the roster.
They keep whatever they already stored outside `group.json`. After they apply
the removal, their own group file is cleared.

## Implementation map

| Concern | Home |
| --- | --- |
| Ids, membership bodies, fold, knowledge fold, links, invites, SAS, screen state | `crates/sovra-core/src/group.rs` and the modules beside it |
| Commands the screen sends and the events it applies | `crates/sovra-core/src/group/session.rs` |
| QUIC messages and the `GroupStore` / `GroupTransport` ports | `crates/sovra-core/src/group/wire.rs` |
| iroh endpoint, Ed25519, `groups/group.json` | `crates/sovra-mesh/src/lib.rs`, `crates/sovra-mesh/src/store.rs` |
| Create, invite, join, admit, sync, broadcast, recall wrapper | `crates/sovra-runtime/src/groups.rs` |
| Device key and reach | `crates/sovra-app/src/main.rs` (`group_device_key`, `group_services`) |
| Groups tab (layout) | `crates/sovra-ui/src/groups_view.rs` |
| Groups tab wording, pages and Getting started | `crates/sovra-core/src/group/guide.rs` |
| Spaces, lane keys, shared artifacts, templates, federated recall, work jobs | `crates/sovra-core/src/group/{space,lane_keys,shared_artifact,template,federated,work}.rs` |
| Sealing, nearby (mDNS) discovery | `crates/sovra-mesh/src/cipher.rs`, `crates/sovra-mesh/src/lib.rs` |
| Handing a turn to the group, answering without tools | `crates/sovra-adk/src/group_work.rs` |
| Two real runtimes over iroh (pairing by link and code, sharing, asking) | `crates/sovra-app/tests/two_computers.rs` |
| Helper instructions when group evidence is attached | `crates/sovra-adk/src/recall.rs` |
| Evidence caps | `crates/sovra-core/src/memory/recall.rs` (`GroupEvidence`, six items) |

Tests that lock the rules live next to the core modules (`group/tests.rs`),
in `crates/sovra-runtime/tests/groups.rs` (two runtimes, through `Client`),
in `crates/sovra-ui/tests/groups.rs` (the screen), and in
`crates/sovra-adk/src/recall.rs` (`group_guidance_arrives_with_shared_details`).


## Optional group verification

Run `cargo test -p sovra-core benchmark_group_history -- --ignored` to exercise
2,000 sequential shares (1,000 notes and 1,000 approximately 1 KiB documents),
then rebuild that history three times. The fixture uses deterministic test
signatures and the production fold, including its signature/hash caches. It
asserts that all notes and artifacts survive without rejected or pending items.
This measures local CPU work, not disk persistence, Ed25519 or network delivery.

### Complete local network transfer

`cargo test -p sovra-mesh
notes_and_artifacts_cross_the_real_network_in_complete_signed_rounds` exercises
two real local iroh endpoints, real device signatures and bounded sync replies.
It transfers 20 Unicode notes and 20 documents containing 9,600,000 bytes of
source text, including quotes and line breaks that expand in JSON. It fetches
successive batches until every item arrives, verifies every signature and
compares the complete ordered content after each round. The operation chain is
valid; later requests advertise the hashes already received.

These fixtures exercise local CPU and network behavior, not public-relay
performance or a production service guarantee. Record the revision, workload,
machine, and build profile when measuring them.
