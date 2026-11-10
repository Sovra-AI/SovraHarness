# Groups of people and computers

**Settings → Groups** connects computers in a family or small team and lets you
explicitly share details their helpers can use. A **group** contains people and
computers; a [team](teams-and-artifacts.md) contains AI helpers in a chat.

## Start or join

1. Open **Settings → Groups** and enter a group name, your name and this
   computer's name under **Start a group**.
2. Choose **Start the group**. Add a second admin so membership can still be
   managed if your computer is unavailable.
3. Invite someone as a member or admin, or choose **Add another of my computers**.
4. Copy the generated `sovra://join/…` link and send it privately to the intended
   person. It works once and expires; the UI shows its remaining lifetime.
5. On the other computer, paste it under **Join a group**, name the computer,
   and choose **Join**. Keep the inviting computer online during pairing.

The current flow uses pasted links. QR pairing and the other richer pairing
flows in the roadmap are plans, not controls in this screen.

## Share and manage knowledge

Write a detail under **Shared with the group**, then choose **Share with the group**.
Nothing in your private chats or files is automatically copied into that list.
Shared results identify who supplied them. Other people's words remain data;
sharing a detail does not grant tools or permission to run commands.

You can agree with a detail, stop using your own detail, or erase its words.
Corrections and disagreements retain their status; withdrawn, replaced and
former-member details are excluded from normal use. Erasure applies through the
replicated state and cannot retrieve copies someone already saved elsewhere.

Admins can change roles and remove people or computers. Removal asks for a second
confirmation. Removed devices stop receiving new group information, but keep
what they already received. **Leave this group** leaves on this computer.

## Network and current limits

The adapter uses iroh for authenticated, encrypted connections. The default
Internet reach uses n0 discovery/relay facilities; encrypted traffic may pass
through a relay. This is not a guarantee that packets stay on the local network.
`SOVRA_GROUP_REACH=local` selects direct connections without those discovery and
relay services. Set it before launching each participating app; see
[configuration](configuration.md).

The implemented foundation includes membership, signed operations, peer sync
and explicitly shared text knowledge. The wider stage-2 plan includes encrypted
private lanes, helper/skill bundles, work handoff to another computer, and
production relay operations. Do not assume those are available because the
Groups page exists. The [group roadmap](roadmap/02-a-team.md) retains that design.

Group records live under `groups/` in app data. The device secret lives in the
credential store. If it cannot be stored safely, Groups stays unavailable.
Copying only the group files does not move that identity to another computer.

## Implementation

[Core group rules](../crates/sovra-core/src/group.rs) own membership and
knowledge validation and folding. [The mesh adapter](../crates/sovra-mesh/src/lib.rs)
owns transport and signatures; [the native screen](../crates/sovra-ui/src/groups_view.rs)
exposes the controls. Tokio and task ownership remain in `sovra-runtime`; the
mesh library receives a spawner instead of directly depending on tokio.
