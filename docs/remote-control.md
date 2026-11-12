# Remote control from a phone

Research and implementation contract, 12 November 2026. The intended experience
is **Skills → Use Sovra from your phone**, also available as `/remote-control`.
After pairing once, opening the phone app connects to an enabled desktop without
typing addresses, forwarding ports, or copying a fresh code. This controls Sovra's
window and chats; operating-system desktop control would require a separate design.

**Implementation status:** the desktop implements the Skills action, an independent
encrypted listener, remembered phones, expiring pairing QR/link, visible enabled
status, stop/forget controls and authenticated native inspection and input. It
starts disabled after every app restart. A runnable reference phone client and
real encrypted endpoint tests cover the desktop contract. Live native testing
verified skill activation, pairing, reconnect, typed input, stop/start, invitation
privacy and revocation. The mobile app and notification service are not built;
real iOS/Android and cellular handover remain future integration tests.

## What automatic connection means

The first invocation opens a pairing card. The phone scans a short-lived QR code,
which identifies the desktop and authorizes that phone's public key. Both devices
remember the relationship. Later invocations enable access for remembered phones;
the foreground mobile app reconnects using the saved desktop identity. Repeating
the skill while access is active must preserve the current session and invitation.
Pairing another phone is an explicit action, not a side effect of reconnecting.

An app that does not yet exist cannot receive this connection. Even once it is
installed, desktop activation cannot reliably open a closed mobile app. Apple's
background notifications are best effort, can be throttled and have a short
execution window. Android also restricts background work and recommends push
delivery instead of continuous polling. The product must say **Open Sovra on your
phone** when no phone is connected; a push notification can offer a shortcut but
cannot be the only connection mechanism.
Sources: [Apple background updates](https://developer.apple.com/documentation/usernotifications/pushing-background-updates-to-your-app),
[Android background tasks](https://developer.android.com/develop/background-work/background-tasks).

The computer must remain awake, online, and running Sovra. Closing Sovra stops
access. Starting the app restores remembered phones but leaves remote control
off until invoked. Stopping remote control cancels pending input and closes
connections. Forgetting a phone revokes its key, including existing connections.
Neither a phone nor an AI tool can silently re-enable remote control.

## Connectivity decision

| Approach | Fit and tradeoff | Decision |
| --- | --- | --- |
| Iroh QUIC with relay fallback | Already in `sovra-mesh`; authenticated device keys, encrypted streams, NAT traversal; native mobile bindings exist | Use a dedicated remote-control protocol and endpoint in the existing adapter |
| Hosted WebSocket relay | Straightforward mobile socket support, but all traffic crosses an operated backend; needs a separate end-to-end encryption and device identity protocol | Keep as an alternative if mobile Iroh interoperability fails |
| WebRTC data channels | Browser support and mature NAT traversal; requires signaling and usually TURN service, plus another transport stack | Reconsider for a future browser client or video stream |
| Local HTTP or mDNS only | Useful for development on one LAN; does not satisfy cellular or different-network access | Test mode only |
| OAuth device grant | Useful for account sign-in on an input-constrained device; does not transport control or authenticate a direct socket by itself | Optional future account enrollment, not the pairing transport |

Iroh publishes Swift and Kotlin bindings supporting iOS and Android. This is
evidence of feasibility, not proof of compatibility with Sovra's pinned Rust
version: the mobile build must pin a compatible binding release and pass a real
interop test. Sources: [language support](https://www.iroh.computer/blog/iroh-language-support),
[Swift](https://docs.iroh.computer/languages/swift),
[Kotlin](https://docs.iroh.computer/languages/kotlin),
[binding source](https://github.com/n0-computer/iroh-ffi).

Iroh's relay forwards encrypted traffic and helps establish direct paths. Public
relays are rate limited; production needs managed or Sovra-operated relay capacity
with availability monitoring. A relay is not a mailbox and cannot wake a sleeping
phone. Do not assume a published direct-connection percentage applies to Sovra.
Source: [Iroh relay operation](https://www.iroh.computer/blog/shared-relays).
WebRTC also requires relay infrastructure for many network combinations:
[TURN guidance](https://webrtc.org/getting-started/turn-server).
OAuth's device grant separates device and user codes and specifies expiry and
polling behavior, but is an authorization flow rather than remote transport:
[RFC 8628](https://www.rfc-editor.org/rfc/rfc8628.html).

## Repository integration

The existing mesh adapter uses Iroh 1.4.1, configured in the root manifest.
Its locally installed source confirms `Connection::remote_id()` is the identity
from the completed TLS handshake, and `alpn()` identifies the negotiated protocol.
Only that transport identity may identify a phone; a JSON `device_id` is not proof.

Use a separate endpoint and device secret so remote control has an independent
lifetime from Groups, with ALPN `sovra/remote-control/1`. This remains in
`sovra-mesh`, the sole owner of Iroh, with no new crate or dependency-layer change.
Group membership grants no phone access. Do not extend the group request enum
or repurpose a group invitation as a remote-control credential.

| Owner | Responsibility |
| --- | --- |
| `sovra-core::remote_control` | Pure pairing, allowlist, activation and request validation; typed public wire contract |
| `sovra-mesh` | Dedicated Iroh endpoint, completed-handshake identity, bounded streams and encrypted transport |
| `sovra-runtime` | Owned listener tasks, timeouts, persistence transactions, request sequencing and UI requests/replies |
| `sovra-ui` | Skills action, pairing card, connected status, stop/forget controls, native inspection and input |
| `sovra-app` | Paths, independent credential-store key and relay configuration, adapter wiring |
| Future mobile repository | Secure phone identity, QR scanner, remembered desktops, reconnect logic, remote view and input |

Development MCP stays loopback-only with its own credential and enabled state.
Remote control may share validated native input primitives, but must have its own
authorization gate. It must work in ordinary builds with Development mode off.
Never expose the development descriptor or bridge over the network.

Register an action skill named `remote-control`, titled **Use Sovra from your
phone**, with the example **Let me use Sovra from my phone**. Resolve this action
before any model call: activation must work without an AI provider. The action
opens the remote card and sends a typed `Command`; runtime status arrives as
`Event`. The model's ability to load skill instructions does not grant access.

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

Initial limits to enforce and advertise: 32 KiB request frames, 8 MiB response
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

## Mobile reconnection and notification integration

Store the paired desktop key and address hints with a friendly name. Keep phone
secrets in iOS Keychain or Android protected storage; do not promise hardware-bound
key generation if the selected Iroh binding requires exporting a seed. On opening
the app, connect to the last selected desktop and issue Hello. Use bounded
exponential backoff with jitter (one to thirty seconds), reset on a network change
or explicit retry, and stop polling when the app is suspended. A revoked phone
returns to pairing; an inactive desktop shows **Start remote control on your
computer**. Never interpret unreachable as revoked.

APNs/FCM can later notify an enrolled phone that remote control is ready. This
requires an operated notification service, app identifiers, platform credentials,
token rotation and unlinking. Send only an opaque wake hint, with no invitation,
chat content or private key. The phone must authenticate the desktop after waking.
Do not place APNs/FCM provider credentials in the desktop application. An account
service could make same-account enrollment easier later; signing into the same
account alone must not implicitly authorize control of every desktop.

## Runnable reference phone client

Build with `cargo build -p sovra-app`, then run the resulting `sovra` binary with
`--remote-control-client`. This starts a JSON-lines client, with no second GUI,
database or Development MCP connection. It uses the same encrypted transport as
the future phone. Its separate `remote-control-reference-phone-key` uses the
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
no background polling; an interactive mobile app adds the foreground backoff
policy described above. Mutations never reconnect or replay automatically. If a
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
external public relay. It passed on 12 November 2026 in 3.22 seconds. It proves
relay transport, not mobile operating-system background execution or cellular
handover. The reference client API lives in
[`remote_reference.rs`](../crates/sovra-runtime/src/remote_reference.rs); its stdio
wrapper is in [`stdio.rs`](../crates/sovra-runtime/src/remote_reference/stdio.rs).

## Future mobile implementation checklist

1. Pin Swift/Kotlin Iroh bindings compatible with the desktop's Iroh 1.4.1 wire
   protocol. Prove a completed authenticated handshake using the dedicated ALPN
   on both platforms before implementing the control screen.
2. Create one persistent phone identity in platform-protected storage. Scan and
   strictly validate the invitation URI, pin its desktop public key, send Pair,
   then discard the invitation secret. Save only public desktop routing hints
   and its friendly name. Do not put invitation URLs into analytics or crash logs.
3. On foreground launch, connect to the selected remembered desktop and send
   Hello automatically. Store the returned generation and session in memory,
   reset sequence to 1, then Inspect. Never ask for a fresh QR just because the
   network changed or the desktop restarted.
4. Build a remote view from the safe control snapshot and optional screenshot.
   Preserve focus, labels, disabled state, scrolling and desktop confirmation
   dialogs. Inspect before mutations, serialize calls and require an observed
   revision. Give `busy`, `not_paired`, `stale_session`, `stale_observation` and
   `outcome_unknown` distinct recovery paths; never replay uncertain input.
5. Test reconnect after idle timeout, Wi-Fi/cellular handover, desktop stop/start,
   phone suspend/resume, desktop sleep/wake and pair revocation. Test one active
   controller, two simultaneous claimants and pairing while another phone is
   connected. Verify that screenshots cannot reveal an active pairing invitation.
6. Test real iOS/Android devices on different networks, including relay-only
   connectivity and unavailable relays. Budget image bandwidth and foreground
   polling explicitly. Production relay capacity and optional APNs/FCM enrollment
   require separate operated infrastructure; neither exists in this change.

## Acceptance evidence and mobile follow-up

| Requirement | Evidence required before marking complete | Current state |
| --- | --- | --- |
| Research first | Primary sources, options, constraints and selected repository integration | Recorded above |
| Pairing rules | Offline tests for expiry, replacement, wrong identity/secret, one use, bounds and revocation | Core policy, sequencing and checked JSON fixtures pass; runtime pairing/reconnect tests pass |
| Encrypted transport | Two real endpoints; authenticated identity; wrong ALPN, malformed/oversized frames, slow-peer tests | Four local transport tests and an explicit public relay-only test pass; owned runtime deadlines and global admission limits implemented |
| Skill activation | Catalogue test and runtime events; activation without a model; repeated invocation idempotent | Live skill activation and headless activation without a model pass; repeated Start preserves the current invitation in runtime tests |
| Useful phone control | Reference client pairs, inspects, types in chat and observes completion through the running native app | Live encrypted client typed and submitted `/help`; native Skills navigation completed. Headless native test separately verifies chat composition/submission with Development mode off |
| Automatic reconnect | Same saved phone reconnects without invitation after skill activation; stale generation rejected | Real-network test covers client restart and idle timeout; live client reconnected after process restart and desktop endpoint stop/start without another invitation |
| Persistence | Restart restores pairs but starts disabled; failed save/revoke and corrupt file tests | Runtime tests pass, including exclusive ownership and failed revoke disabling access; real client reloads public desktop hints and persistent phone identity |
| Native usability | Development MCP verification; stop/forget; minimum/wide windows and 200% text size | Live verification covers skill, QR presence, indicator, stop/start, invitation privacy and revocation; headless layout checks include the actual QR grid at minimum/wide windows and 200% text. Latest manual launch verified QR bounds and stop cleanup. Screenshot capture is unavailable on this Linux renderer |
| Cellular access | Two networks, direct and relay-only paths, network handover and sleeping desktop | Direct network and real public relay-only roundtrips pass. Real cellular/mobile, handover and sleep/wake testing awaits the mobile app |
| Future app handoff | Exact JSON fixtures, error codes, runnable reference client, mobile build checklist | Documented above and implemented in the reference client |
| Repository gate | `cargo xtask ci` result with any unrelated failures identified | Final desktop gate: `NEXTEST_TEST_THREADS=8 cargo xtask ci` passed all gates, 1377 tests passed and 14 skipped (12 November 2026). The opt-in public relay test also passed separately |

The transport tests use real local QUIC endpoints, including a response larger
than the stream receive window, repeated streams, a changed desktop key, a wrong
ALPN, invalid JSON and an oversized frame. They do not establish Internet/mobile
reachability, native input completion over a network. Separate runtime and headless UI tests
exercise authorization and chat composition. The adapter
accepts only one incoming bidirectional stream at a time and no unidirectional
streams. Its pending-handshake port lets the runtime own handshake deadlines and
concurrency rather than spawning unowned tasks in the adapter.

The mobile application itself is a later deliverable, as requested. Real
iOS/Android tests remain necessary before claiming that a shipping phone app
connects automatically. Live test setup was restored: the temporary phone was
forgotten, access was switched off and its isolated development keys removed.
