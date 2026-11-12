# GitHub in Sovra

How a person connects GitHub from **Connections**, and what Sovra can then do
with it.

There is one **Connect GitHub** button. GitHub's own page opens in the browser
with a short code already on the clipboard. The person pastes it and presses
**Authorize**, and the card turns to **Connected** by itself. The **GitHub
helper** (`GitHubAgent`) then appears to the assistant with the tools below.
Everything ships in the single binary: no server, no listener and no secret.

## What the person sees

| State | Card shows | Actions |
| --- | --- | --- |
| Not connected | What Sovra can do with GitHub | **Connect GitHub** · *Use a GitHub key instead* |
| Waiting for you on GitHub | The code in large type ("already copied for you") | Copy code · Open GitHub again · Cancel |
| Make a key (no GitHub app in this copy, or chosen) | Three numbered steps | **Open GitHub** (the key page, already filled in) · paste box · **Connect** |
| Checking your key… | — | — |
| Connected | Avatar initial, name, @username, three "Try asking…" examples | **Disconnect GitHub** (asks once more and links to GitHub's settings) |
| Needs you to connect again | Why | **Connect again** |

- **Two ways in, one button.** A copy built with a GitHub app uses GitHub's
  device sign-in. A copy without one sends **Connect GitHub** straight to the
  key steps, so the button always works.
- **The key page is pre-filled.** **Open GitHub** opens
  `github.com/settings/tokens/new?description=Sovra&scopes=repo,read:org`. The
  person only scrolls down, presses **Generate token** and copies the key.
- **The pasted key is never shown again.** The box is masked and cleared when
  sent. `GitHubKey`'s `Debug` output is redacted.
- **Disconnect** removes the access from this computer. GitHub has no way for a
  public desktop app to revoke its own token, so the confirm row links to
  GitHub's **Applications** settings for a full removal.

## What the agent can do

| Tool | What it does | Changes things |
| --- | --- | --- |
| `github_list_repositories` | The person's repositories (own, collaborator, organisation), most recently changed first; optional name filter | No |
| `github_search` | Issue or pull-request search with GitHub search words; adds `is:issue`/`is:pr` when missing | No |
| `github_read_issue` | One issue or PR conversation with its latest 20 comments | No |
| `github_read_pull_request` | Changed files (first 60), the start of the first 15 patches, branches, and a summary of the checks | No |
| `github_read_file` | A file's text (clipped at 30,000 characters), or a folder listing, at an optional branch, tag or commit | No |
| `github_create_issue` | Opens an issue, signed as the person | **Yes** |
| `github_comment` | Comments on an issue or PR, signed as the person | **Yes** |

Nothing merges, pushes, closes, deletes or edits code.

**Prompt injection** is the main risk: issues, comments and code are written by
other people. The defences are layered, as for Google:

- **Labelled content.** Every result that carries outside text includes
  `github::UNTRUSTED`, and the helper's procedure repeats it.
- **Ask-first wording.** Both write tools say they may be used only when the
  person asked in this conversation, and the helper must show what it posted.
- **Checked names.** `RepoName`, `clean_repo_path` and `clean_reference` in
  `sovra-core` refuse anything that is not a plain `owner/name`, a path without
  `.`/`..`/empty segments, or a branch-like ref. The adapter then adds each one
  as an encoded URL path segment, never by string formatting. The commit id used
  for the checks request must be hex from GitHub's own answer.
- **Bounded reads.** Responses are capped at 8 MB, and every text field is
  clipped.

## How it works

```
sovra-ui  ──Command::GitHub──▶  sovra-runtime (GitHubRuntime: one owner task)
   ▲                                │  owned poll task (aborted on Cancel / Disconnect)
   └──Event::GitHub(Changed/        │  blocking worker: keychain
        OpenSignIn/Notice)          ▼
                       sovra_core::github::GitHubConnector  ◀── sovra-github (adapter)
sovra-adk: AdkHarness::with_github(GitHubTools) → Capability::GitHub → GitHubAgent
```

- **`sovra-core/src/github.rs`** holds the pure parts:
  - the state, commands, events and the `GitHubConnection` reducer;
  - `GitHubRequest` and the name/path/ref/search checks;
  - the device-flow answer mapping and the wait between checks;
  - the `GitHubConnector` port.
  The keychain port `secrets::SecretVault` is shared with Google.
- **`sovra-github`** is the IO side: the device flow, key checks, the vault entry
  `connection.github` (`{token, account}`) and bounded reqwest + rustls calls to
  about ten REST endpoints (API version `2022-11-28`).
- **`sovra-runtime/src/github.rs`** owns the state machine:
  - restore on start;
  - sign-in attempts numbered so a late result from a replaced attempt is
    ignored;
  - the 15-minute limit;
  - the per-call guard;
  - *Needs you to connect again* when a call gets a 401.
- **`sovra-ui/src/github_card.rs`** is the card. On `OpenSignIn` the shell copies
  the code to the clipboard and opens the page.

### Device sign-in (RFC 8628, GitHub "device flow")

1. `POST github.com/login/device/code` with the client id and
   `scope=repo read:org`. GitHub returns a device code, the user code,
   `github.com/login/device`, an interval and an expiry.
2. The runtime opens the page and checks `POST github.com/login/oauth/access_token`
   every *interval* seconds (at least 5). `slow_down` adds 5 seconds each
   time, and `expired_token` or `access_denied` stop with plain words.
3. On success it reads `GET /user`, saves `{token, account}` in the keychain
   (the dev file store under `SOVRA_DEV=1`) and publishes **Connected**.

The device flow needs only the public client id, with no client secret and no
callback listener. That is why it was chosen over the loopback flow Google uses.

## Setting up the GitHub app (once per release pipeline)

1. Create an OAuth App at <https://github.com/settings/applications/new> (or
   under an organisation). Name it **Sovra**. Any homepage and callback URL will
   do; device flow does not use the callback.
2. On the app's page, tick **Enable Device Flow** and save.
3. Copy the **Client ID**. It is public, so it may ship in the binary.
4. Locally, copy `github-client.env.example` to the gitignored
   `github-client.env` and fill it in. `crates/sovra-app/build.rs` bakes it into
   every build. CI sets `SOVRA_GITHUB_CLIENT_ID` as a build variable, which wins
   over the file. The same variable at **run time** overrides the baked value.

Without an id, the key path still works end to end. Organisations that restrict
OAuth Apps must approve **Sovra** before their private repositories are visible.
Until then, GitHub returns 403 and the tool says so in plain words.

## Not yet

- No notifications inbox, review submission, labels or assignees.
- Fine-grained keys work, but the key page link creates a classic key, because
  fine-grained keys cannot be pre-filled by URL.
- GitHub Enterprise Server addresses are not configurable from the screen
  (`sovra_github::Endpoints` supports them in code).

## Sources

Checked on 2026-11-12:

- GitHub Docs, *Authorizing OAuth apps → Device flow*
- GitHub Docs, *Scopes for OAuth apps*
- GitHub REST API docs: users, repositories, search, issues, issue comments,
  pulls, check runs, repository contents
- RFC 8628, *OAuth 2.0 Device Authorization Grant*
