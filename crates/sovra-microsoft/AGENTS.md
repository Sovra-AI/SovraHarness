# sovra-microsoft

Microsoft 365 sign-in and Graph behind the core Microsoft ports.
Design and sources: `docs/microsoft-365.md`.

- Native public-client authorization code with shared `sovra-oauth` PKCE and
  loopback callback. No secret, embedded browser, broker process or sidecar.
- Only the configured Microsoft authority and Graph endpoints receive tokens.
  Download redirects never receive a Graph bearer token.
- Access tokens stay in this crate. Refresh tokens use `AsyncSecretVault`, whose
  runtime implementation owns blocking credential IO. Redact debug and errors.
- An uncommitted sign-in cannot write the vault. Serialize renewal and disconnect;
  a disconnected session cannot write itself back.
- No tokio, tasks, threads or sleeps here. Runtime owns all work and deadlines.
- Bound requests and response bodies. Typed actions include no send/delete operation
  and no calendar attendees. Retrieved content is untrusted.
- Use only Graph v1.0. Keep organization features optional; don't request Mail.Send
  or application permissions. Do not retry ambiguous writes automatically.
