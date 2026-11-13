# Messaging from chat

How WhatsApp, Telegram, and sending a message from the desktop chat work, including
who is answered and what stays private.

The desktop orchestrator offers `CommunicationsAgent` when a messaging adapter is
available. Its tools reach the existing connections actor through a core port
bound to the desktop conversation. Normal chat and attachment turns both carry
the port; other specialists cannot acquire it by supplying a session id. The
Google helper receives the actual connected Google tools through the same
capability-provider registry, preserving that integration's draft-only email API.

`messaging_status`, `list_recent_chats`, and `read_recent_messages` inspect current
connection state and the bounded local recent-message snapshot. This is not a
complete phone-history API. `find_people` looks up saved people by name. An empty
question returns a count and does not list everyone. The same phone number or the
same email joins one person. A shared first name never does. After `WhatsApp` is
linked and online, the people list keeps every one-to-one contact and chat the
phone names: saved address-book rows, chats that were never saved, the name
shown on that chat, and chats that are pinned, archived, muted, or marked read.
A hidden-id chat is kept when that same sync includes its phone number. A phone
number is never invented when WhatsApp does not provide one. Groups the linked
account is in stay on the group list, with a stable group id and subject, as
soon as the live session can list them. A new link asks the phone for the full
chat backfill (365 days, including chats it can fetch later) while Linked
Devices still says "Sovra connection". A computer that was already linked asks
again for its contact and chat lists the next time it connects. Saved versions
of those lists are set back to the start so the lists are sent again, and the
link itself is kept. Older message history is only offered when you unlink and
connect again. Message bodies are
not downloaded. There is no separate address-book download. An empty or failed
refresh does not invent people or groups and does not erase anyone already
saved. Deleting a chat drops that chat only. Removing a contact on the phone
drops that number. Status updates, broadcast lists, and newsletters are not
stored. A group is a chat, not a person: group participants are not turned into
saved people.

`prepare_message` accepts `me`, an exact recent chat id, an explicit international
`WhatsApp` number, one saved person's full name, `person:<id>` from a card the
person chose, a unique stored `WhatsApp` group subject, or that group's exact id
(`…@g.us`). More than one matching person or group is refused so the helper asks.
A phone number that is not a `WhatsApp` handle is not used as a `WhatsApp`
recipient. Telegram still needs a chat that has already written to the bot, or a
chat added on that person. Sovra does not download a Telegram address book.
`list_recent_chats` also returns stored `WhatsApp` groups, and `find_people`
returns up to eight `whatsapp_groups` whose subject contains every word asked
for (an empty question returns none), so "message the school group" finds the
exact group id. A group review shows its subject and "WhatsApp group" rather than
the raw id. Looking up people is a safe read. Preparing a message stays ordered and does not send. Groups are never
answered automatically.

A confirmed timed WhatsApp or Telegram text is a schedule job with
`send_channel`, `send_recipient`, and `send_text`. Consent is the create step
(who, which app, exact wording, when). `send_recipient` is the international
number starting with `+`, `me`, an exact chat id, a `WhatsApp` group id, or
`person:<id>` from `find_people`. A first name is refused at create so the job cannot fail only
when it is due. When that job fires, the runtime transmits on the same
`send_text` path native Send uses. There is no fire-time review and no model
approval tool. Live `prepare_message` is unchanged. A failed fire keeps the
reason on the run so Schedules can show why it didn't finish.

`prepare_message` validates a single text message of up to 4,096 characters and
creates a native review in the originating chat. It does not send. Repeated
identical requests in the same turn reuse a review that is still waiting or
sending. A cancelled, accepted, or expired review can be prepared again. The
people list shows each phone number, email, and Telegram chat. Message on
WhatsApp opens that chat so Send is on screen. The review shows
the sender, recipient and exact text, and offers Send, Cancel and Save. Knowledge
→ People lists stored `WhatsApp` groups separately from people; **Message this
group** opens the same review. No model tool can approve it or replace its
content. Changing the message requires a new review. `message_receipt` is limited
to the originating desktop conversation.

### Sending without asking

After a message is accepted, its card asks "Next time, send messages to … without
asking you first?" **Yes, don't ask again** sends the native-only
`ConnectionCommand::TrustRecipient`, which no tool can issue. The core rule
(`ConnectionsState::set_trusted`) only trusts an exact chat id that already has an
accepted message on that channel, keeps at most 50, and persists them in
`trusted_recipients`. A later `prepare_message` from a chat turn to that exact
chat records Sending and transmits at once; its result carries
`sent_without_review`, and the helper still checks `message_receipt`. A turn may
skip review at most three times; later requests in that turn fall back to a normal
review. Drafts the person opens from People always show the review. The same card
then offers **Ask me first next time**, and Settings → WhatsApp lists these chats
under **Sends without asking** with **Ask first** on each.

The runtime rechecks the conversation, ten-minute deadline, online status and
exact linked session before consuming approval. It records Sending before IO;
when storage is configured, failure to persist that state prevents sending.
Duplicate clicks cannot resend. Up to eight reviews/sends may be outstanding;
the actor owns the send tasks and each has a 30-second deadline. Errors and
timeouts report uncertainty and never automatically retry. Service acceptance
does not establish delivery or reading. Restart expires unapproved reviews and
marks interrupted sends uncertain. Receipts and drafts remain in the bounded
connections history and can be saved to Artifacts.

Phone-originated sessions never receive outbound messaging tools. Authenticated
WhatsApp self-chat messages and the Telegram owner established by linking retain
private tools and memory. Other direct-message senders receive a sanitized harness
without private standing instructions, files, memories, imported skills, connected
services or account tools, and their messages do not enter owner memory capture.
Sender names and retrieved text are untrusted data. Pairing credentials and tokens
are never passed to the model. Existing automatic answers still use the separate
per-channel settings and never answer groups or offline backlog.

This design follows the human-control and input-validation guidance in the
[MCP tools specification](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/server/tools.mdx).
The shared message limit follows Telegram's
[`sendMessage` contract](https://core.telegram.org/bots/api#sendmessage).
WhatsApp uses the existing pinned Web-client adapter; this change does not add a
Business API, bulk sending, media sending or arbitrary protocol access. A new
link registers as **Sovra connection** under Linked Devices on the phone. An
already-linked session keeps its previous label until you unlink and connect
again.

Verification uses fake transports and loopback model providers, never real
outbound messages. Domain tests cover recipient validation and review scope;
ADK tests execute real delegation; runtime tests exercise native approval,
duplicate clicks, cancellation, uncertainty, link changes and disconnects.
Headless UI tests cover both themes, 640×420 and 2560×1440 windows, and 200% text.
The running app must also be inspected through Development MCP before completion.
