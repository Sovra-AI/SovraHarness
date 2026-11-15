# Connect your own Microsoft 365 account

Open **Connections → Connect Microsoft 365**. Sovra guides you through a one-time
setup for your own Microsoft app registration. The same steps are available in
installed Windows, macOS and Linux binaries. There is no shared Sovra Microsoft
registration, build variable or configuration file to obtain.

You do **not** need a new Windows login, Office installation, browser extension
or Microsoft 365 Copilot subscription. You do need a Microsoft account with the
services you want to use, plus permission to register an app in Microsoft Entra.
Your organization's policies and service licenses still apply.

## Before you start

- **Work or school:** use the account your organization gave you. Register in
  that organization's Entra directory. If app registration is blocked, use
  **Copy help request** in Sovra and send it to your IT administrator. The message
  lists the exact settings and public IDs to return. Sovra does not send it.
- **Personal:** use this choice for Outlook.com, Hotmail or Live. App registration
  still needs access to an Entra workforce directory. A personal email account
  alone may not provide that access. An administrator or helper with appropriate
  directory access can create the registration for you.

Microsoft's [registration requirements](https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app)
include an Azure account with an active subscription and appropriate app
registration rights. If you have no directory, read the
[tenant prerequisites](https://learn.microsoft.com/en-us/entra/fundamentals/create-new-tenant)
before starting: creating additional workforce tenants is restricted for
free/trial customers. Do not choose an External ID customer-signup directory
for this Outlook/Graph connection. Sovra does not create subscriptions or
promise that every personal account can complete registration for free.

## Four guided steps

### 1. Choose your account

Choose **Work or school** or **Personal**. Select **Start setup**. If an
administrator has already sent the correct IDs, select **I already have the
IDs**; you must still confirm the desktop return address before sign-in.
**Microsoft's requirements** opens Microsoft's current registration guide.

### 2. Create your connection

1. Select **Open Microsoft setup**. In Microsoft Entra, use the directory where
   you have permission to register the app. Work/school users should use their
   own organization's directory.
2. Open **App registrations → New registration**.
3. Name it **Sovra on my computer**.
4. Choose the account type shown in Sovra:

   | Your choice in Sovra | Supported account types in Microsoft |
   | --- | --- |
   | Work or school | **Single tenant only — your directory**, also labelled **Accounts in this organizational directory only** |
   | Personal | **Personal accounts only**, also labelled **Personal Microsoft accounts** |

5. Leave **Redirect URI** empty on this first page, then select **Register**.
6. Keep the Microsoft page open. In Sovra, select **I registered the app**.

An existing registration can be used if its account type and native callback
match these choices. You do not need to delete other registrations or callbacks.

### 3. Set the return address

1. On the registered app's Microsoft page, select **Authentication**.
2. Select **Add a platform** (some portal versions say **Add redirect URI**),
   then **Mobile and desktop applications**.
3. Use **Copy return address** in Sovra. Paste exactly `http://localhost` as the
   **Redirect URI**, then select **Configure** or **Save** in Microsoft.
4. Back in Sovra, tick **I saved this return address**, then **Continue to IDs**.

The platform must be **Mobile and desktop applications**, not Web or SPA. Do
not create a client secret. Keep other callbacks if another app uses them, but
do not add Web/native localhost callbacks distinguished only by their port.
Microsoft ignores the localhost port during redirect matching. The native
system-browser choice is documented in Microsoft's
[desktop guide](https://learn.microsoft.com/en-us/entra/identity-platform/scenario-desktop-app-configuration)
and [redirect rules](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url).
Sovra uses authorization code with PKCE; the advanced **Allow public client
flows** fallback is for other flows without redirects, such as device code.

### 4. Check and sign in

1. In the Microsoft app registration, open **Overview**.
2. Copy **Application (client) ID** into the field with the same label in Sovra.
3. For work/school only, also copy **Directory (tenant) ID** from that same page.
4. Check the account type and return address shown in Sovra. Confirm that the
   address was saved under **Mobile and desktop applications**.
5. Select **Save and sign in**. Sign in on Microsoft's browser page, review the
   requested permissions, and approve them if you agree. Return to Sovra.
6. Wait for **Connected** and your account name. Opening the browser alone does
   not mean the connection succeeded.

Use the exact ID labels, not **Object ID**. IDs contain 36 characters including
dashes. Sovra trims pasted whitespace and catches missing, malformed, all-zero
or duplicate IDs. It cannot determine from their shape whether an ID belongs
to the right registration. Microsoft checks that during sign-in. Never paste a
password or client secret into Sovra; those are not requested.

Sovra requests delegated Graph scopes when signing in, so most users do not
need to select permissions individually in the portal. If your organization
requires administrator consent, ask your administrator to review/add the
scopes in the permission table below under **API permissions → Add a permission
→ Microsoft Graph → Delegated permissions**, following organization policy.
Default scopes also include `User.Read` and `offline_access`. Optional work
services stay off and ask for their additional scopes only when you enable them.
Do not select application permissions or give the app `Mail.Send`.

## After setup

The IDs and account connection are saved on this computer. Next time, Sovra
restores the saved connection or offers **Sign in again**. **Choose what Sovra
can use** reveals service switches. Switching a service off restricts Sovra's
tools; it does not revoke Microsoft's consent.

**Cancel** stops a pending sign-in. **Open Microsoft again** reopens its page.
**Disconnect Microsoft 365** asks for confirmation and removes local access,
keeping the public IDs for easy reconnection. To change registrations,
disconnect first, then use **Edit setup**. **Forget setup** asks for confirmation
and removes the local IDs; it does not delete anything in Entra. Revoking
Microsoft's server-side consent is a separate Microsoft account/admin action.

## If something goes wrong

| What you see | What to do |
| --- | --- |
| No directory, or no permission to register | Go Back to step 1 and use **Copy help request**. Ask the directory administrator; for personal accounts, check the prerequisites above. |
| Application not found / `AADSTS700016` | Cancel, select **Edit setup**, and recopy **Application (client) ID**. For work/school, confirm the Directory ID comes from the same registration. |
| Redirect mismatch / `AADSTS50011` | Cancel and revisit step 3. Save `http://localhost` under **Mobile and desktop applications** for that exact Application ID. |
| A client secret is requested / `AADSTS7000218` | Check the native platform and callback. Do not fix this by creating or sharing a secret. |
| Personal/work account isn't supported | Check the account choice in Sovra and **Supported account types** in Microsoft. Sign in with an account matching that registration. |
| Administrator approval required | Use the copied help request. Your administrator must decide whether to approve access; Sovra cannot bypass the policy. |
| A service has no data or access is denied | Check that service's switch, permissions, license and mailbox/site availability. Other connected services can still work. |
| Browser does not return, or sign-in times out | Cancel, verify the saved return address and local browser/network policy, then retry. A sign-in attempt has a five-minute limit. |
| Saved connection cannot be updated | Check the computer's credential store. Sovra does not report Connected unless saving access succeeds. |

## Why the setup is arranged this way

The interface presents one task at a time, with visible progress, Back buttons,
copy actions and a final review. Values survive Back navigation. The form shows
which field needs correction and prevents submission until required values and
the return-address confirmation are present. These native adaptations follow
GOV.UK's [question-page guidance](https://design-system.service.gov.uk/patterns/question-pages/)
and [check-answers guidance](https://design-system.service.gov.uk/patterns/check-answers/),
using Sovra's own accessible controls and responsive design tokens.

## Service and permission map

All permissions below are **delegated**: calls act as the person who signed in,
within the access that person already has. Application permissions and
tenant-wide background administration are outside this desktop design.

| Service | Intended actions | Delegated scopes | Account limits |
| --- | --- | --- | --- |
| Email in Outlook | Find and read messages; create drafts for review | `Mail.ReadWrite` | Personal and work; requires a mailbox |
| Outlook Calendar | Read a date range; add appointments without guests | `Calendars.ReadWrite` | Personal and work; requires a mailbox |
| OneDrive files | Find and read Word, Excel, PowerPoint, PDF and text files | `Files.Read` | Personal and work |
| Outlook contacts | Look up contacts | `Contacts.Read` | Personal and work |
| Microsoft To Do | List task lists and tasks; add a task | `Tasks.ReadWrite` | Personal and work |
| OneNote | List and read pages | `Notes.Read` | Personal and work |
| SharePoint | Find sites and document libraries; find and read files | `Sites.Read.All` | Work or school |
| Teams chats | List joined chats and read messages | `Chat.Read` | Work or school |
| Teams channels | List joined teams, channels, messages and replies | `Team.ReadBasic.All`, `Channel.ReadBasic.All`, `ChannelMessage.Read.All` | Work or school; channel messages require administrator consent |
| Planner | Read tasks assigned to the signed-in person | `Tasks.Read` | Work or school; service/license must be available |

The scope map follows Microsoft's endpoint permission tables for
[draft messages](https://learn.microsoft.com/en-us/graph/api/user-post-messages?view=graph-rest-1.0),
[calendar ranges](https://learn.microsoft.com/en-us/graph/api/user-list-calendarview?view=graph-rest-1.0),
[calendar writes](https://learn.microsoft.com/en-us/graph/api/user-post-events?view=graph-rest-1.0),
[file search](https://learn.microsoft.com/en-us/graph/api/driveitem-search?view=graph-rest-1.0),
[To Do](https://learn.microsoft.com/en-us/graph/api/todotasklist-list-tasks?view=graph-rest-1.0),
[OneNote](https://learn.microsoft.com/en-us/graph/api/onenote-list-pages?view=graph-rest-1.0),
[SharePoint search](https://learn.microsoft.com/en-us/graph/api/site-search?view=graph-rest-1.0),
[Teams chats](https://learn.microsoft.com/en-us/graph/api/chat-list?view=graph-rest-1.0),
[channel messages](https://learn.microsoft.com/en-us/graph/api/channel-list-messages?view=graph-rest-1.0),
and [Planner assignments](https://learn.microsoft.com/en-us/graph/api/planneruser-list-tasks?view=graph-rest-1.0).
The [permissions reference](https://learn.microsoft.com/en-us/graph/permissions-reference)
defines which grants need an administrator; tenant policy can also restrict
permissions that normally permit user consent.

Creating drafts requires the broader `Mail.ReadWrite` permission; Microsoft
does not offer a draft-only permission. Sovra's request types must expose no
send, delete or mailbox-edit operation, and must never request `Mail.Send`.
Calendar writes omit attendees because adding them sends invitations.

Word and PowerPoint are documents in OneDrive or SharePoint, not separate
OAuth connections. Read the downloaded bytes with Sovra's existing bounded
document readers. This also reads Excel without granting file writes: even
[listing worksheets with the Excel API](https://learn.microsoft.com/en-us/graph/api/worksheet-list?view=graph-rest-1.0)
requires `Files.ReadWrite`. Live spreadsheet editing, Office automation, Power
BI, Dynamics, Azure administration, and Purview are separate future capabilities;
the connection must not claim to implement them.

## Desktop sign-in

Use the Microsoft identity platform's authorization-code flow with **PKCE
S256** and a fresh random `state`. Sovra is a **public client**; do not create,
request, store, or embed a client secret. Guided work/school setup uses the
person's directory GUID; personal setup uses `consumers`. Read identity from
Graph `/me`; do not decode an unverified ID token to establish account identity.
Request `User.Read` and `offline_access` with the selected service scopes.
[Microsoft's protocol reference](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow)
documents the endpoints and grant exchange.

Register **`http://localhost`** as a **Mobile and desktop applications** redirect.
At runtime, bind only `127.0.0.1` on an OS-assigned port and send
`http://localhost:<port>` as the redirect. Microsoft's documented port-ignoring
exception applies to localhost; do not assume a random-port `127.0.0.1` redirect
has the same matching behavior. The callback accepts only one matching state,
rejects duplicate OAuth parameters, and bounds reads. A five-minute runtime
deadline and cancellation wake the listener. The browser response says to
return to Sovra; only completed token exchange and credential persistence may
show **Connected**.
[Redirect restrictions](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url)
explain the platform and localhost requirements.

Microsoft recommends MSAL for supported platforms. Its
[published MSAL platform list](https://learn.microsoft.com/en-us/entra/identity-platform/msal-overview)
does not provide a Rust desktop implementation. Sovra therefore shares its
native callback and PKCE implementation between account adapters and uses the
existing reqwest/rustls stack for Microsoft protocol calls. Device code flow
adds a code-entry step and can be blocked by organization policy; it is not the
primary desktop experience. Embedded webviews, password collection, and a
confidential-client secret are inappropriate for this distribution model.

## Tokens and service failures

Public registration IDs and refresh tokens go into separate entries in the
existing OS credential vault. Each sign-in captures an immutable configuration.
Changing setup is allowed only while disconnected and removes any prior saved
access. No old shared registration is inferred from legacy token entries or
build-time/environment settings. Refresh tokens go into the same vault. The approved debug
development store remains isolated outside the checkout. Access tokens stay in
memory. Refresh before expiry, serialize concurrent refreshes, and persist the
replacement refresh token before considering renewal successful. Microsoft
returns replacement refresh tokens and does not invalidate the old one merely
because it was used. A revoked grant leads to **Sign in again**, without a
retry loop. See [refresh tokens](https://learn.microsoft.com/en-us/entra/identity-platform/refresh-tokens).

Keep identity stable when adding a service; silently switching the account must
not attach new permissions to the old account. Late completion after Cancel or
Disconnect must never restore credentials. Runtime owns all tasks and closes
the callback on cancellation, timeout, replacement, and shutdown.

Use Graph `v1.0`. Bound response sizes, item counts, and text returned to the
model. This version returns up to 20 items from the first response and 40,000
text characters, with `more_available` and truncation flags. It cannot page
through a whole account; narrow searches or open Microsoft for older items.
Files are limited to 20 MB and use the existing document reader; image-only or
unsupported content is not a complete text reading. File content redirects are preauthorized URLs:
never attach the Graph bearer token to a download host and never return these
temporary URLs to the model. See
[file downloads](https://learn.microsoft.com/en-us/graph/api/driveitem-get-content?view=graph-rest-1.0).

Respect `Retry-After` for throttling: the session rejects further calls until
the cooldown expires and tells the helper how long to wait. No write is replayed
automatically. A 403 can mean permission, license, mailbox,
or organization policy trouble and should not disconnect every working service.
An expired or rejected sign-in needs a new browser flow. Device-compliance
policies may require platform broker integration beyond this browser flow;
do not promise universal Conditional Access compatibility. Do not advertise
the `cp1` capability without implementing claims challenges. See
[throttling](https://learn.microsoft.com/en-us/graph/throttling) and
[claims challenges](https://learn.microsoft.com/en-us/entra/identity-platform/claims-challenge).

Disconnect removes Sovra's local tokens. It must not claim to revoke Microsoft's
server-side consent or sign the person out of all Microsoft applications. Offer
Microsoft account application-management instructions separately. Read content
is untrusted data and can enter the user's chosen model context, as with the
Google connector.

## Architecture and verification

The shared `sovra-oauth` boundary owns native callback IO, randomness, and PKCE.
The Microsoft adapter owns Microsoft HTTP and tokens behind core ports. The
runtime owns sign-in and call tasks, service guards, blocking credential IO,
and document decoding. GPUI only sends commands and folds events. This adds no
runtime outside the existing executable and no new external TLS stack.

Automated coverage includes scope and request validation, bounded HTTP replies,
sanitized OAuth failures, the shared loopback callback, runtime cancellation
and persistence, per-user setup/replacement/forgetting, tools registered on
Skills, and the guided flow with responsive native controls at
640×420, 2560×1440 and 200% text size in both themes. Run `cargo xtask ci` and
check the running Development-mode app after changes. Actual Microsoft sign-in,
refresh rotation against Entra, service responses, and organization policy
remain external verification steps until a real registration exists.
