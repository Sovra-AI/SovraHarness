# Messaging from chat

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
same email joins one person. A shared first name never does. While `WhatsApp` is
linked, contacts the phone has synced are collected in the background. There is no
separate address-book download, and an empty or failed sync does not erase people
already saved.

`prepare_message` accepts `me`, an exact recent chat id, an explicit international
`WhatsApp` number, one saved person's full name, or `person:<id>` from a card the
person chose. More than one match is refused so the helper asks. A phone number
that is not a `WhatsApp` handle is not used as a `WhatsApp` recipient. Telegram
still needs a chat that has already written to the bot, or a chat added on that
person. Sovra does not download a Telegram address book. Looking up people is a
safe read. Preparing a message stays ordered and does not send.

A confirmed timed WhatsApp or Telegram text is a schedule job with
`send_channel`, `send_recipient`, and `send_text`. Consent is the create step
(who, which app, exact wording, when). `send_recipient` is the international
number starting with `+`, `me`, an exact chat id, or `person:<id>` from
`find_people`. A first name is refused at create so the job cannot fail only
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
the sender, recipient and exact text, and offers Send, Cancel and Save. No model
tool can approve it or replace its content. Changing the message requires a new
review. `message_receipt` is limited to the originating desktop conversation.

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
Business API, bulk sending, media sending or arbitrary protocol access.

Verification uses fake transports and loopback model providers, never real
outbound messages. Domain tests cover recipient validation and review scope;
ADK tests execute real delegation; runtime tests exercise native approval,
duplicate clicks, cancellation, uncertainty, link changes and disconnects.
Headless UI tests cover both themes, 640×420 and 2560×1440 windows, and 200% text.
The running app must also be inspected through Development MCP before completion.
