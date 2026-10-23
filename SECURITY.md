# Security Policy

SovraHarness is closed-source, proprietary software owned by Sovra. This
policy applies to everyone with access to the repository.

## Reporting a vulnerability

Report suspected vulnerabilities **privately and directly to the Sovra
engineering maintainers** — not in issue titles, PR descriptions, or any
channel visible beyond the people who need to act on it. Include
reproduction steps, impact, and affected crate(s). You will get an
acknowledgment and a fix or mitigation plan; do not disclose details
outside Sovra at any point.

## Handling secrets

- API keys and credentials live in the **OS keychain** (via `keyring`),
  never in files, environment files, logs, or commit history.
- Signing and notarization credentials exist only as GitHub Actions
  secrets consumed by `.github/workflows/release.yml`.
- If a secret is ever committed, treat it as compromised: rotate it
  immediately, then remove it from history.

## Dependency and build posture

- TLS is rustls only; OpenSSL is banned (`deny.toml`).
- No git dependencies or wildcard versions; every dependency is pinned
  through the root `[workspace.dependencies]` and vetted by `cargo-deny`
  (licenses, advisories) in CI.
- `unsafe` is forbidden across the workspace.
- The Rust toolchain is pinned (`rust-toolchain.toml`); releases are built
  from CI, signed, and distributed through Velopack auto-update.
