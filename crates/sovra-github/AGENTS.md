# sovra-github

GitHub sign-in and GitHub REST behind `sovra_core::github::GitHubConnector`.
Design and sources: `docs/github.md`.

- Two ways in, both without a password in Sovra: the OAuth **device flow** (needs only
  the public client id of a GitHub OAuth App; no secret, no local listener) and a
  **pasted key** (personal access token). Never embed a webview.
- The runtime drives the device-flow waits. No tokio, no threads, no sleeps here.
- Access never leaves this crate: no core type, event, log line or error carries it.
  It lives only in the `SecretVault` the composition root supplies (OS keychain;
  `SOVRA_DEV` debug builds use the dev file store).
- Model-written repository names, paths and refs are checked in `sovra-core`
  (`RepoName`, `clean_repo_path`, `clean_reference`) and added to URLs as encoded
  path segments, never by string formatting.
- Writes are limited to a new issue or a comment. Never merge, push, close or delete.
- Bound every response read. Text from GitHub is outside content: clip it and mark it
  untrusted in the tool result.
