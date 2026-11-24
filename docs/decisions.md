# Decision records

Keep a decision that should outlive the chat in **Artifacts → Decisions**.
The **Keep a company decision and its reasons** card on Skills opens the same
page, as does `/decisions`.

## Save a draft

1. Choose **New decision**, or **Record a decision** under a completed chat reply.
2. Write the question, your proposed decision and your company's criteria.
3. Add the source reference and exact wording of each cited passage. Include
   the file, page or lines, and date where useful. These are quotations entered
   by the person saving the record; Sovra does not certify their accuracy.
4. Set **Review on** as `YYYY-MM-DD`. Dates use UTC. Write what would make the
   decision no longer valid.
5. Add any company-written hard stops. Check **This condition has been met**
   only when it has. A check applies to the exact wording reviewed; check a
   changed condition again. An unchecked condition blocks acceptance.
6. Choose **Save decision draft**. The record is saved only after storage succeeds.
   A failed save keeps your fields. Back asks before discarding unsaved changes.

A chat-started record retains the original reply and its conversation reference,
separately from the decision wording you edit. Quoted passages are saved with
the record; later changes to the source files do not change those quotations.

## Accept a company position

A second person signs in with their own Sovra account on this computer, opens
the record, reviews it, and chooses **Accept as company position**, then
**Confirm acceptance**. The runtime obtains their identity from its active
session; there is no field for typing an approver's name. Continuing as a guest
does not count as that person. A guest can draft a decision, and cannot accept
or end one.

Use **Sign out** at the bottom of the sidebar to switch people. If the reviewer
has no account here yet, choose **Create another account** on the sign-in page.
**Use an existing account** switches back without changing saved accounts.

Anyone who drafted a version of the record cannot accept it. Acceptance also
requires every written hard stop to be cleared and a future review date.
The record keeps who drafted each version, who accepted it and when.

These are the app's existing local accounts, including name-only accounts.
A different account is not independent identity verification: someone with
access to this computer can reopen a name-only account. Use the company's own
account practices. Records are shared by local accounts, not isolated per
account, and are not sent through Groups. This is not a digital-signature or
organisation-wide approval service.

## Review, revise or end a decision

**Save a new version** retains the previous wording, evidence and acceptance.
The new version is a draft and needs fresh acceptance. A person who edits the
record becomes one of its writers, so they cannot then accept it.

At the start of the UTC review day, an accepted record is **Review due** and
no longer counts as an accepted current position. Save a reviewed version
with a future date and have another person accept it.

When a written ending condition happens, enter the reason and choose **End this
decision**, then confirm. The record becomes **No longer valid**. The reason,
person and date remain alongside its acceptance and evidence. This also allows
withdrawal of an unaccepted draft without deleting its history. Earlier versions
remain readable through **Earlier versions**; **Copy record and history** copies
a dated, readable record of them all.

Sovra checks the explicit hard-stop checkboxes and review date. It does not infer
whether an ending condition has happened, watch external business events, send
reminders, or supply rules for any industry. The company writes its own criteria
and stops and records when those conditions have been met or broken.

## If acceptance is unavailable

| What you see | What to do |
| --- | --- |
| A second person must sign in | Sign out and let someone who has not drafted any version use their own account. Creating a new version yourself does not make you its reviewer. |
| Sign in with your own account before accepting or ending | A guest visit cannot accept or end a company decision. Sign out, then use your own account. |
| Blocked by your hard stops | Review each condition your company wrote. Save a new version with a condition checked only when it has been met, then ask the second person to accept it. |
| The review date has arrived | Save a reviewed version with a future date, then obtain fresh acceptance. |
| Already accepted or no longer valid | Keep the existing record as history. Use **Save a new version** when a new company position is needed. |

## Storage and tests

The decision table lives in the existing local memory database, independently
of memory extraction and automatic recall. Decision records are not fed to chat
or shared automatically. Records have no delete action. Each holds at most 128
versions, 16 cited passages per version and 32 hard stops. At the version limit,
start another decision and cite the earlier record. Lists load 100 summaries at
a time and open one complete history when requested.

Core owns validation, status, versioning and acceptance. The auth actor orders
decision commands with account changes; the UI supplies no author or approver
identity. The store atomically checks revisions before saving, so stale actions
cannot overwrite a newer decision. Events acknowledge only committed writes.
Schema version 13 adds the decision table without changing existing saved work.

Tests cover pure acceptance and review rules, real store round-trips and stale
writes, the Client/account/disk lifecycle, and native forms at the minimum and
wide window sizes and 200% interface size in both themes. Follow
[Development MCP](development-mcp.md) for live verification; all normal gates
still run through `cargo xtask ci`.
