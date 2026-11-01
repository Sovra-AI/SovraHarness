# Google in Sovra

Sovra connects to a person's own Google account from **Settings → Connections**: one
**Connect Google** button, Google's own sign-in page in their browser, and a
switch per service afterwards. The agent then gets tools for the services that
are connected and switched on. Everything is in the single binary: no server,
sidecar or browser extension.

Sources were checked on 2026-10-27; they are listed at the end.

## What the person sees

| State | Card shows | One action |
| --- | --- | --- |
| Not connected | What Sovra can do, the five services | **Connect Google** |
| Connecting | "Your browser has opened Google's page" | Open the page again · Copy link · Cancel |
| Connected | Avatar initial, name, email; one row + switch per service; "Try asking…" examples | Switch a service; **Disconnect Google** (asks once more) |
| Needs sign-in | The account and why | **Sign in again** |
| Needs a client id | Desktop client id and optional masked secret | **Save**, then **Connect Google** |

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

**Prompt injection is the main risk.** Email, documents and invites are written
by other people. Combined with private data and a way to send something out,
that is the "lethal trifecta". The mitigations are layered:

- **No send path.** Email is draft-only and events have no guests. A hidden
  instruction cannot send anything out without the person.
- **Labelled content.** Text from mail, files and event notes is clipped and sits
  under an `untrusted` notice in every tool result.
- **Ask-first wording.** Every tool that adds something says it may be used only
  when the person asked in this conversation, never because content said so.
- **Checked ids.** Ids from the model are restricted to Google id characters
  before they are put into a URL path.
- **Clean headers.** Draft headers are made single-line, so a model cannot add a
  `Bcc:` header, and recipients must be email addresses.
- **Bounded reads.** Responses are size-capped: 4 MB for JSON, 10 MB for files.

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
  (which folds the Google state). It is shown as the Connections section of
  Settings (`settings_view.rs`). The shell opens `OpenSignIn` URLs with
  `cx.open_url`.
  Connections sits inside Settings rather than in its own sidebar entry: a tenth
  destination no longer fits the collapsed rail at the minimum window.

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

**Saved sign-ins expire.** Google refresh tokens die with `invalid_grant` in these
cases, and all of them show as **Sign in again**:

- revoked by the person;
- unused for 6 months;
- a Gmail-scope password change;
- seven days in *Testing* publishing status;
- more than 100 tokens per account per client.

## Scopes and Google verification (release blocker)

| Service | Scope | Google class |
| --- | --- | --- |
| Identity | `openid email profile` | non-sensitive |
| Email | `gmail.readonly`, `gmail.compose` | **restricted** |
| Calendar | `calendar.events` | sensitive |
| Files | `drive.readonly` | **restricted** |
| Contacts | `contacts.readonly` | sensitive |
| To-dos | `tasks` | sensitive (confirm in Cloud Console) |

Email and Files need **restricted** scopes. Reading mail is impossible with
anything less, and `gmail.compose` (drafts) is itself restricted. Shipping them
to the public needs:

- the OAuth consent screen set to *In production*;
- **sensitive-scope verification** (about 3–5 business days), for Calendar,
  Contacts and Tasks;
- **restricted-scope verification plus a yearly CASA security assessment**
  (several weeks), for Gmail and Drive;
- a privacy policy stating the Google API **Limited Use** terms. In particular,
  Workspace data is never used to train models.

**Until it is verified:**

- People see Google's "unverified app" warning.
- The project has a **100-user lifetime cap**.

**Two ways to ship before CASA:**

- Narrow v1 to sensitive scopes. Calendar, Contacts and Tasks work as they are.
  Email could become `gmail.send` with an in-app confirmation step, at the cost
  of reading mail. Files could become `drive.file` with Google's desktop Picker
  as a separate flow.
- Let advanced users supply their own Google Cloud client (the environment
  variables below), which uses Google's personal-use exemption.

The scopes live in one table (`SERVICES` in `sovra-core/src/google.rs`), so
either choice is a data change plus tool tweaks.

## Setting up the Google client (once per release pipeline)

1. In Google Cloud Console, create a project and enable these APIs: Gmail,
   Google Calendar, Google Drive, People, Tasks.
2. Configure the OAuth consent screen: app name **Sovra**, support email,
   privacy policy, the scopes above, and test users while in *Testing*.
3. Create an **OAuth client ID** of type **Desktop app**. Google documents its
   secret as not confidential for installed apps, so it may ship in the binary.
4. Build with the client baked in, so the single binary needs no setup:

   ```sh
   SOVRA_GOOGLE_CLIENT_ID=… SOVRA_GOOGLE_CLIENT_SECRET=… cargo xtask dist
   ```

   The same variables at **run time** override the baked-in values. A client id
   saved in Settings wins over both. That covers development, and people who
   use their own client. Without a client id anywhere, the card asks for one.
   **Connect Google** and **Continue with Google** stay off until a desktop
   client id is saved, set in the environment, or baked in.
5. Use a separate client for development. *Testing* status expires refresh
   tokens after seven days.

## Adding a service or a connection

- **A Google service:** add a `GoogleService` variant and a `SERVICES` row in
  core, the `GoogleRequest` variants and their adapter calls, the tools in
  `sovra-adk/src/google.rs`, and an icon and example in `google_card.rs`.
- **Another connection** (WhatsApp is in progress in parallel): add its own
  `Command`/`Event` pair and state, and add its card as one more child of the
  `settings-connections` section (`render_connections_section`).

## Sources

- Google: [OAuth for desktop apps](https://developers.google.com/identity/protocols/oauth2/native-app),
  [OAuth overview](https://developers.google.com/identity/protocols/oauth2),
  [discovery document](https://accounts.google.com/.well-known/openid-configuration),
  [device flow limits](https://developers.google.com/identity/protocols/oauth2/limited-input-device),
  [webview ban](https://developers.googleblog.com/en/upcoming-security-changes-to-googles-oauth-20-authorization-endpoint-in-embedded-webviews/)
- IETF: [RFC 8252](https://datatracker.ietf.org/doc/html/rfc8252) (native apps), RFC 7636 (PKCE; the test vector is used in `loopback.rs`)
- Scopes: [Gmail](https://developers.google.com/workspace/gmail/api/auth/scopes),
  [Drive](https://developers.google.com/workspace/drive/api/guides/api-specific-auth),
  [Calendar](https://developers.google.com/workspace/calendar/api/auth),
  [Tasks](https://developers.google.com/tasks/auth),
  [all scopes](https://developers.google.com/identity/protocols/oauth2/scopes)
- Verification: [restricted scopes](https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification),
  [production readiness](https://developers.google.com/identity/protocols/oauth2/production-readiness/overview),
  [unverified apps](https://support.google.com/cloud/answer/13464321)
- APIs: [Gmail sending](https://developers.google.com/workspace/gmail/api/guides/sending),
  [Gmail quota](https://developers.google.com/workspace/gmail/api/reference/quota),
  [Drive export](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/export),
  [People searchContacts](https://developers.google.com/people/api/rest/v1/people/searchContacts),
  [Drive desktop Picker](https://developers.google.com/workspace/drive/picker/guides/desktop-mobile-picker)
- UX and safety: [Thunderbird OAuth](https://source-docs.thunderbird.net/en/latest/backend/oauth.html),
  [preconnect sockets (bug 2060544)](https://bugzilla.mozilla.org/show_bug.cgi?id=2060544),
  OWASP LLM01 prompt injection, Simon Willison's "lethal trifecta"
