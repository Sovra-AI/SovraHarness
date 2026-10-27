# sovra-google

Google sign-in and Google REST behind `sovra_core::google::GoogleConnector`.
Design, scopes and sources: `docs/google.md`.

- Desktop OAuth only: system browser, single-use `127.0.0.1` listener on a random port,
  PKCE S256 and a fresh `state` per attempt. Never embed a webview or use OOB/device flow.
- Tokens never leave this crate: no core type, event, log line or error carries them.
  The refresh token lives only in the `SecretVault` the composition root supplies
  (OS keychain; `SOVRA_DEV` debug builds use the dev file store).
- Runtime owns blocking calls (`restore`, `begin_sign_in`, `wait`, `save`, `forget`).
  No tokio, no threads, no sleeps here; cancellation wakes the listener by connecting to it.
- Scopes come from `sovra_core::google::SERVICES`; never request scopes outside it.
  Email is drafted, never sent; events invite nobody.
- Bound every response read. Text from mail and files is outside content: clip it and
  mark it untrusted in the tool result.
