# Google in Sovra

How a person connects their Google account from **Connections** and what each
service switch allows.

Sovra connects to a person's own Google account from the **Connections** tab: one
**Connect Google** button, Google's own sign-in page in their browser, and a
switch per service afterwards. The agent then gets tools for the services that
are connected and switched on. Everything is in the single binary: no server,
sidecar or browser extension.

Start with **Connections → Connect Google**. If this copy has no configured
client, use **Use my own Google Cloud client** as described below.

## What the person sees

| State | Card shows | One action |
| --- | --- | --- |
| Not connected | What Sovra can do, the five services | **Connect Google** |
| Connecting | "Your browser has opened Google's page" | Open the page again · Copy link · Cancel |
| Connected | Avatar initial, name, email; one row + switch per service; "Try asking…" examples | Switch a service; **Disconnect Google** (asks once more) |
| Needs sign-in | The account and why | **Sign in again** |
| Not set up yet | "Google isn't included in this copy of Sovra yet" (builds without a built-in client) | **Use my own Google Cloud client** reveals the client id and masked secret; **Save**, then **Connect Google** |

- Plain words throughout: Email, Calendar, Files, Contacts, To-dos, each with the
  product name beside it and one sentence on what Sovra does with it.
- Google's consent screen lets people untick individual permissions. Sovra reads
  what was actually granted. A service whose permissions weren't granted shows
  "Off — turning it on asks Google".
- Switching a granted service off keeps it off without signing out. Its tools
  disappear on the next reply, and calls already in flight are refused.
- The browser lands on a self-contained "You're connected — you can close this
  tab" page (or "Nothing was connected"). If nothing happens for five minutes,
  Sovra stops waiting and says so.
- "Try asking" examples open Chat with the question filled in.
- A saved desktop client takes precedence over environment configuration, then
  build-time configuration. The same choice powers Continue with Google.
  The credential store keeps the client separately from the account, so
  disconnecting preserves setup; saving clears the secret input.

## What the agent can do

| Service | Tools | Writes |
| --- | --- | --- |
| Email (Gmail) | `search_email`, `read_email`, `draft_email` | Drafts only. Sovra **never sends**; the person presses Send in Gmail. |
| Calendar | `list_calendar_events`, `add_calendar_event` | Events on their own calendar; **nobody is invited** (`sendUpdates=none`, no attendees). |
| Files (Drive) | `search_drive_files`, `read_drive_file` | None. Docs/Slides export as text and Sheets as CSV (first sheet only). |
| Contacts | `search_contacts` | None. |
| To-dos (Tasks) | `list_todos`, `add_todo` | To-dos on the default list. |

Treat retrieved mail, files, and event notes as untrusted content. Tool results
are bounded and labelled; draft headers and resource IDs are validated. Gmail
has no send tool and calendar events do not invite guests. These controls do not
make all connected tools safe from prompt injection or prevent content from
being included in model requests. Review drafts before sending them in Gmail.

## How it works

```
sovra-ui  ──Command::Google──▶  sovra-runtime (GoogleRuntime: one owner task)
   ▲                                │  blocking worker: keychain, loopback listener
   └──Event::Google(Changed/        │  async: token exchange, REST
        OpenSignIn/Notice)          ▼
                       sovra_core::google::GoogleConnector  ◀── sovra-google (adapter)
sovra-adk: AdkHarness::with_google(GoogleTools) — tools per switched-on service
```

- **`sovra-core/src/google.rs`** holds the pure parts:
  - `SERVICES` (labels, explanations, least-privilege scopes);
  - `granted_services` (token `scope` → services);
  - `GoogleState` and the `GoogleConnection` reducer;
  - `GoogleRequest` (one variant per tool);
  - the HTML-to-text and clipping helpers;
  - the `GoogleConnector` port.
- **`sovra-google`** is the IO side: the desktop OAuth flow, the vault, and thin
  bounded reqwest + rustls clients for about 15 endpoints. Generated Google
  crates were avoided because they bring a second HTTP/TLS stack (hyper-rustls)
  and heavy compile times.
- **`sovra-runtime/src/google.rs`** owns the state machine: restore on start,
  sign-in with a timeout and cancellation, switches, disconnect, and the
  per-call guard that turns tool calls away when a service is off. When a call
  finds the sign-in no longer works, the state moves to *Needs sign-in*.
- **`sovra-ui`** has `google_card.rs`: a `GoogleCard` entity owned by the shell
  (which folds the Google state). It is the Google row of the **Connections**
  sidebar tab. Settings only points at that tab. The shell opens `OpenSignIn`
  URLs with `cx.open_url`. The collapsed rail scrolls, so Connections stays
  reachable at the minimum window.

### Sign-in (RFC 8252, Google "installed apps")

1. Bind a single-use listener on **`127.0.0.1`, random port**. That is loopback
   only; `localhost` is "not recommended" in RFC 8252.
2. Open `accounts.google.com/o/oauth2/v2/auth` in the **system browser**.
   Embedded webviews are blocked by Google with `disallowed_useragent`. The
   request carries:
   - **PKCE S256**;
   - a fresh random `state`;
   - `access_type=offline`;
   - `prompt=consent`, so the checkboxes always appear;
   - `login_hint` when reconnecting.
3. Accept connections until a `GET /` arrives with the matching `state`. Ignore
   favicon requests, forged states and speculative empty connections (a known
   browser behaviour), and answer with the static page.
4. Exchange the code at `oauth2.googleapis.com/token` and read the profile from
   the OpenID `userinfo` endpoint. Map the returned `scope` to services.
5. Keep the **refresh token in the OS keychain** as one JSON entry with the
   account, through the same credential store as model keys (dev file store
   under `SOVRA_DEV=1`). Access tokens stay in memory, are refreshed a minute
   early, and are refreshed once more after a 401.
6. **Disconnect** revokes the token at `oauth2.googleapis.com/revoke` (best
   effort; offline still signs out locally) and deletes the keychain entry.

**Desktop apps cannot add a permission later.** Google doesn't support
incremental authorization for installed apps. Turning on a service that wasn't
granted therefore re-runs the whole consent for *granted + that one*.
Cancelling is clean: it sets a flag and connects to the listener, which wakes
the blocked `accept` without polling or sleeping.

Saved sign-ins can expire or be revoked. **Sign in again** starts a new consent
flow. Testing/publishing status and account policies can affect token lifetime;
consult Google's current documentation instead of assuming a saved token is permanent.

## Scopes and Google verification (release blocker)

The requested scopes are defined by `SERVICES` in
[core Google rules](../crates/sovra-core/src/google.rs): identity, Gmail read/compose,
Calendar events, Drive read-only, Contacts read-only, and Tasks.
A runtime service switch controls tool availability; it does not narrow an
already issued Google token.

Before distributing a shared OAuth client, check the actual Cloud project's
consent configuration, API enablement, requested scopes, and verification state.
Sensitive/restricted scope verification and security-assessment requirements
depend on usage and data flow, including third-party servers. A successful local
sign-in does not establish approval for public distribution. Follow Google's
[restricted-scope verification guidance](https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification)
and [desktop OAuth documentation](https://developers.google.com/identity/protocols/oauth2/native-app).

## Setting up the Google client (once per release pipeline)

1. In Google Cloud Console, create a project and enable these APIs: Gmail,
   Google Calendar, Google Drive, People, Tasks.
2. Configure the OAuth consent screen: app name **Sovra**, support email,
   privacy policy, the scopes above, and test users while in *Testing*.
3. Create an **OAuth client ID** of type **Desktop app**. Google documents its
   secret as not confidential for installed apps, so it may ship in the binary.
4. Build with the client baked in, so the single binary needs no setup. Locally,
   copy `google-client.env.example` to the gitignored `google-client.env` and
   fill it in; `crates/sovra-app/build.rs` bakes it into every build (`cargo xtask
   dev`, `run`, `dist`). CI and release pipelines set the same names as build
   environment variables, which win over the file:

   ```sh
   SOVRA_GOOGLE_CLIENT_ID=… SOVRA_GOOGLE_CLIENT_SECRET=… cargo xtask dist
   ```

   The same variables at **run time** override the baked-in values. A client id
   saved in Settings wins over both. That covers development, and people who
   use their own client. Without a client id anywhere, the card asks for one.
   **Connect Google** and **Continue with Google** stay off until a desktop
   client id is saved, set in the environment, or baked in.
5. Use a separate client for development and check the configured test-user and
   publishing restrictions before testing.

## Extending the adapter

Add service definitions and request variants in core, bounded API calls in
`sovra-google`, tools in `sovra-adk`, and native controls in `google_card.rs`.
Register any new user-facing capability on the Skills page. The runtime owns
sign-in tasks, cancellation, and per-call permission checks.
