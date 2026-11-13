# Phone-control protocol and reference client

The desktop implements encrypted remote control, pairing, remembered phones,
inspection, screenshots, and native input. A command-line reference client is
included. **There is no iOS or Android application or push-notification service
in this repository.** This guide documents the implemented desktop protocol.

## Enable and stop access

Open **Skills → Use Sovra from your phone** or use `/remote-control`. The action
works without a model and opens the pairing card. The computer must be awake and
running Sovra. Pairing is explicit; remembered clients can reconnect while access
is enabled. **Stop phone control** closes access; **Forget** revokes a phone.
Every app restart starts access off, even when phones remain remembered.

Phone control uses its own device key, endpoint, and authorization. It does not
grant access through group membership or Development MCP. The ALPN is
`sovra/remote-control/1`. `SOVRA_REMOTE_REACH=local` disables Internet discovery
and relay use; the default allows encrypted relay fallback. See [Network access](network.md).

Implementation: [core rules and wire types](../crates/sovra-core/src/remote_control),
[mesh transport](../crates/sovra-mesh/src),
[runtime](../crates/sovra-runtime/src/remote_control.rs), and
[composition root](../crates/sovra-app/src/main.rs).

## Pairing and authorization

Use a fresh 256-bit invitation secret from OS randomness, expiring after five
minutes on the desktop's monotonic clock. The QR carries protocol version,
desktop public key, relay/address hints and the invitation secret. Never publish
it through discovery, logs, metrics or push payloads. Discovery hints are routing
information, not authorization. Pin the desktop key from the QR when connecting.

The phone generates a persistent key and sends the invitation over a completed,
encrypted handshake. The desktop binds the grant to that connection's public key,
consumes the invitation once, and durably saves the phone before acknowledging.
A second claimant, wrong secret, expired code, unsupported version or disabled
desktop receives a refusal. A new invitation replaces the old one. Limit the
allowlist to eight phones and names to 80 UTF-8 bytes without control characters.
Treat names as untrusted display text, never as identifiers.

The QR is a bearer invitation: anyone who captures it before use can attempt
pairing. Show it only on explicit first setup or **Pair another phone**, and show
the connected phone with an immediate **Forget** control. Do not add a guessable
six-digit fallback without a separately reviewed PAKE or throttled rendezvous
service. Pairing over the Internet works by sharing this invitation privately;
do not promise remote first-time pairing without an existing trusted channel.

Persist desktop and phone private keys in their platform secure stores. On the
desktop, use the existing release keychain policy and approved development-store
exception. Persist only paired public keys and display names in an owner-private,
atomically replaced file. A failed save must not grant a phone. A failed revoke
save must stop remote access and report the failure rather than silently restore
the phone on restart. Corrupt pairing storage fails closed; it is not silently
replaced with an empty success state.

Each activation has a fresh random generation. Bind pending requests to the
generation and authenticated phone. Recheck permission immediately before input
dispatch and before publishing a reply; stop/revoke invalidates queued work.
Transport encryption cannot replace these application-level checks.
While a pairing invitation is available, existing phones cannot inspect, capture
screenshots or send input: a screenshot must not reveal the new bearer invitation.
Control resumes once pairing consumes the invitation or it expires.

## Mobile wire contract

The version 1 JSON types live in
[`wire.rs`](../crates/sovra-core/src/remote_control/wire.rs), with checked
[Hello](../crates/sovra-core/src/remote_control/fixtures/hello-request.json),
[Pair](../crates/sovra-core/src/remote_control/fixtures/pair-request.json),
[Inspect](../crates/sovra-core/src/remote_control/fixtures/inspect-request.json),
[Ready](../crates/sovra-core/src/remote_control/fixtures/ready-response.json) and
[error](../crates/sovra-core/src/remote_control/fixtures/error-response.json)
fixtures. The keys and invitation secrets in these fixtures are synthetic test
values; production must generate fresh random values. Runtime handling is wired
to the native window. Validate the payloads against a real mobile binding before
freezing a mobile release.

One JSON request and reply per bidirectional QUIC stream, with EOF as the frame
boundary. Reject oversized frames before JSON allocation; reject unknown actions
and fields. Require version 1 in every request and return a typed unsupported
version response. Finish the handshake before accepting any mutations; no 0-RTT
input or pairing.

| Operation | Inputs | Result |
| --- | --- | --- |
| Pair | Invitation secret, phone display name; identity comes from TLS | Paired acknowledgment after durable save |
| Hello | Version; identity comes from TLS | Current activation generation, fresh connection session and protocol limits |
| Inspect | Generation, session and sequence | Bounded native control snapshot, observed target IDs and revision |
| Screenshot | Generation, session and sequence | Bounded image of the current Sovra window |
| Click, type, key, scroll | Generation, session, sequence, observed revision and validated action arguments | Delivered acknowledgment or typed refusal |

The outer request has `version` and `operation`; `operation.op` is `pair`, `hello`
or `control`. Control embeds the validated native `request`, whose `action` is
`inspect`, `screenshot`, `click`, `type`, `key` or `scroll`. Fixed 32-byte values
are represented by 64 hexadecimal characters. Requests cannot supply a claimed
phone identity. Replies use the `status` tag: `paired`, `ready`, `result` or
`error`. Error codes are stable machine-readable values; clients display the
accompanying message but must not parse it to decide behavior.

QR links use `sovra://remote-control?ticket=<percent-encoded JSON>`. The ticket
contains `version`, `address` (desktop public key plus hints) and `secret`. Parsing
rejects extra URL parameters, fragments, credentials, unsupported versions,
non-HTTPS relay hints and invalid socket addresses. Encoded links are capped at
2048 bytes to fit the QR encoder's medium error-correction capacity. The URI is
an invitation credential and must never be logged, indexed or saved in chat.

There is one shared Sovra window. Desktop and phone changes are visible to each
other. The phone uses screenshots and accessible control bounds to present the
current app and sends observed target IDs for clicks, not arbitrary coordinates
or internal commands. Phone controls must support composing chats, sending,
viewing replies and stopping ongoing work through real app controls. Sensitive
entry retains desktop masking. This is not a general filesystem, shell, raw
`Command` or credential API; existing confirmations remain visible and required.

Use one active controlling phone at a time. Serialize mutations per connection.
`SessionGuard` uses constant memory: require the exact next sequence, starting
at 1, and consume it even when validation or execution fails. Reject duplicates
and gaps instead of delivering input twice; requests whose result was lost
are **outcome unknown**, never automatically replayed. After a timeout, reconnect
or stale revision, inspect again. Read operations may retry. A version mismatch
offers an update instruction, not silent downgrade.

Enforced protocol limits: 32 KiB request frames, 8 MiB response
frames, 5 MiB PNGs, 1000 observed controls, 4096-byte text, 100 scroll lines,
16 connections, 8 pending UI calls, 5-second transport deadlines and 3-second UI
deadlines. After 4096 control requests, require a fresh Hello session with a new
random nonce; never reuse previous session IDs. Rate-limit handshake
and pairing attempts globally so new keys cannot bypass limits: the runtime
admits at most 16 handshakes and processes at most 32 requests per second across
all peers. The transport allows one incoming stream at a time per connection.
Slow readers/writers must not occupy all slots indefinitely. An idle connection
also closes after five seconds without a request: reconnect and issue Hello before
the next interaction, or send bounded foreground inspections more frequently.
Do not keep polling while the phone app is suspended.

### Native result payloads

A `result.value` for Inspect contains `elements`, `total`, `truncated`,
`input_values`, `appearance`, `interface_scale_percent` and `revision`. Each
element has an opaque `target`, optional `label` and `role`, `visible`, optional
`disabled`/`checked`/`selected`/`expanded`, and logical-coordinate `bounds`
(`x`, `y`, `width`, `height`). Input values are intentionally omitted. Do not parse
or construct target IDs; discard them on reconnect. A revision covers the observed
structure and focus, not asynchronous task completion. Inspect again after each
input and wait for a relevant visible result with a deadline.

Screenshot results contain `png`, a standard-base64 PNG thumbnail fitting within
1600×1600 pixels. This is a snapshot, not a video stream; capture can be unavailable
on some platform renderers. Do not assume image pixels equal logical control
coordinates. Version 1 clients should offer accessible labeled controls from
Inspect rather than deriving click coordinates from the image.

Input results contain `input_delivered: true` and a human-readable `next` hint.
Delivery is not proof a chat finished or a save succeeded. Scroll `lines` is signed:
negative moves down, positive moves up. The native request schema is in
[`development.rs`](../crates/sovra-core/src/development.rs); sharing these data
types does not share Development MCP authorization. Key names follow the pinned
GPUI keystroke parser, including `enter`, `escape`, `backspace` and modifier
combinations. Clients must handle unsupported text/key errors without replaying
a possibly partially delivered mutation.

## Runnable reference phone client

Build with `cargo build -p sovra-app`, then run the resulting `sovra` binary with
`--remote-control-client`. This starts a JSON-lines client, with no second GUI,
database or Development MCP connection. It uses the desktop's encrypted transport. Its separate `remote-control-reference-phone-key` uses the
normal OS credential store, or the approved `SOVRA_DEV=1` debug store. Public
routing hints are saved in `remote-control-reference/desktop.json`.

For an isolated development phone, set `SOVRA_DEV=1` and `SOVRA_DATA_DIR` to an
absolute temporary directory outside the checkout. Keep the same directory for
client restarts. `SOVRA_REMOTE_REACH=local` is useful on one network; omit it to
exercise Internet discovery and relay fallback, including changing desktop
addresses after stop/start. The desktop still uses its own data directory and
must be enabled through its real skill or local Settings controls.

Send one JSON object per line on stdin; each produces one `RemoteResponse` line
on stdout. Supply the pairing link directly from a scanner or clipboard through
stdin, never as a shell argument or log message. These examples use placeholders:

```json
{"op":"pair","link":"<link copied from the desktop pairing card>","name":"My reference phone"}
{"op":"control","request":{"action":"inspect"}}
{"op":"control","revision":"<revision returned by inspect>","request":{"action":"click","target":"<observed target>"}}
{"op":"control","revision":"<fresh revision>","request":{"action":"type","text":"/help"}}
{"op":"control","revision":"<fresh revision>","request":{"action":"key","key":"enter"}}
```

Inspect between each mutation to obtain a fresh revision and verify focus or
navigation. After the first Pair, restart the client and send Inspect: it loads
the remembered desktop, connects and sends Hello without another invitation.
Read operations reconnect once automatically after a connection loss. There is
no background polling. Mutations never reconnect or replay automatically. If a
reply is lost, inspect again and decide what happened before sending more input.
Pair retries first try Hello, so a lost successful pairing acknowledgment does
not require a new QR. EOF closes the client endpoint. Forget the test phone on
the desktop before removing its credential directory.

Run the production-adapter integration test with:

```sh
cargo nextest run -p sovra-app --test remote_control
```

An optional Internet test disables all direct IP transports on the client and
checks that a 100 KB reply crosses an actual relay path:

```sh
cargo test -p sovra-mesh remote_protocol_crosses_a_public_relay -- --ignored
```

This test is intentionally excluded from ordinary CI because it requires an
external public relay. It exercises relay transport, not mobile operating-system background execution
or cellular handover. The reference client API lives in
[`remote_reference.rs`](../crates/sovra-runtime/src/remote_reference.rs); its stdio
wrapper is in [`stdio.rs`](../crates/sovra-runtime/src/remote_reference/stdio.rs).
