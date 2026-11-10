# Groups of people and computers

**Settings → Groups** connects computers in a family or small team and lets you
explicitly share details, documents, helpers and schedules their helpers can use.
A **group** contains people and computers; a [team](teams-and-artifacts.md)
contains AI helpers in a chat.

## Start or join

1. Open **Settings → Groups** and enter a group name, your name and this
   computer's name under **Start a group**.
2. Choose **Start the group**. Add a second admin so membership can still be
   managed if your computer is unavailable.
3. Invite someone as a member or admin, or choose **Add another of my computers**.
4. Either send the generated `sovra://join/…` link (also shown as a QR code)
   privately to the intended person — it works once and expires — or, in the same
   room, choose **Show a code** and read out the six digits.
5. On the other computer, under **Join a group**, paste the link and choose
   **Join**, or type the six digits and choose **Join with code**. With a code,
   both screens then show the same four pictures; each person presses **They
   match** before the computer joins. Keep the inviting computer online.

A code allows one try within two minutes; a wrong try ends it. Codes find the
other computer on the local network (mDNS), so both must be on the same network.

## Share

- **Details:** write one under **Shared with the group**, or open a saved detail
  in **Knowledge** and choose **Share with …** (Sovra rewords “User's …” with your
  name; check the words first).
- **Documents, lists and pictures:** open an artifact and choose **Share with my
  group**. Others find it under **Shared in …** on the Artifacts page and can
  **Keep a copy**; sharing an edited copy makes it the newest version, and two
  edits made at once are both kept.
- **Helpers, skills and schedules:** **Share with my group** on your own helper
  (Team), skill (Skills) or schedule (Schedules). Others read the full words under
  **Shared helpers and schedules** and choose to use their own copy. No tools are
  shared, copied skills are not applied automatically, schedules that send
  messages are never shared, and a schedule only runs on the computer that adds it.
- **Just one person:** choose **Just Dad** instead of **Everyone**. That space
  has its own keys, and its history only travels between your two computers.
- **Ask each other's memories:** with **Let my group's helpers ask my memory**
  on, others' questions are searched here and a few short answers go back with
  your name; you can see every question. **Ask my group** asks once;
  **Ask my group when I chat** lets helpers ask during chat. Both start off.

Nothing in your private chats or files is copied into the group automatically.
Shared results identify who supplied them. Other people's words remain data;
sharing never grants tools or permission to run commands.

You can agree with a detail, stop sharing your own items, or erase their words.
Corrections and disagreements retain their status; withdrawn, replaced and
former-member items are excluded from normal use. Erasure applies through the
replicated state and cannot retrieve copies someone already saved elsewhere.

Admins can change roles and remove people or computers. Removal asks for a second
confirmation. Removed computers stop receiving new group information, but keep
what they already received. **Leave this group** leaves on this computer.

## Security, network and current limits

Everything shared is sealed with a group key before it is saved or sent; each
member computer gets its own sealed copy of the key (X25519 from the device key,
AES-256-GCM). Removing someone retires every key they held, so new shares use a
fresh key only the remaining computers can open.

The adapter uses iroh for authenticated, encrypted connections. The default
Internet reach uses n0 discovery/relay facilities; encrypted traffic may pass
through a relay. This is not a guarantee that packets stay on the local network.
`SOVRA_GROUP_REACH=local` selects direct connections without those discovery and
relay services. Set it before launching each participating app; see
[configuration](configuration.md).

Still planned (see the [group roadmap](roadmap/02-a-team.md)): handing heavy
work to another computer, Sovra-run relays, presence, and an external security
review of pairing and lane encryption before general release.

Group records live under `groups/` in app data. The device secret lives in the
credential store. If it cannot be stored safely, Groups stays unavailable.
Copying only the group files does not move that identity to another computer.

## Implementation

[Core group rules](../crates/sovra-core/src/group.rs) own membership, knowledge,
spaces, lane keys, pairing and templates, all pure. [The mesh
adapter](../crates/sovra-mesh/src/lib.rs) owns transport, signatures, encryption
and nearby discovery; [the native screen](../crates/sovra-ui/src/groups_view.rs)
exposes the controls. Tokio and task ownership remain in `sovra-runtime`; the
mesh library receives a spawner instead of directly depending on tokio.
