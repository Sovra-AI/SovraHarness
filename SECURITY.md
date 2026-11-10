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

- Installed/release builds keep API keys in the **OS keychain** (via `keyring`).
  The user-approved exception is debug development with `SOVRA_DEV=1`, which uses
  separate plaintext files under the app data directory's `dev-credentials/`.
  Unix saves enforce directory mode 0700 and file mode 0600. Release builds ignore
  the flag. No key is copied automatically between stores or included in packages.
  Secrets must never enter repository files, logs, or commit history.
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
- The Rust toolchain is pinned (`rust-toolchain.toml`); the release workflow
  packages with Velopack and signs when signing credentials are configured.
  Unsigned local installers are for testing; automatic update checks are not yet
  implemented in the app.

## Data and execution boundaries

The OS keychain protects credentials, not the whole workspace. Memory, attached
files, schedules and group records live in app data; WhatsApp pairing keys live
in its local session database. Development and installed launches share the
app-data root, apart from their separate credential stores. See
[Configuration and local data](docs/configuration.md).

Local account sign-in does not establish per-account data isolation. Terminal
commands and approved local MCP programs run with the person's OS rights; they
are not sandboxes. Remote model and tool connections can receive content needed
for the request. Group Internet reach uses discovery and encrypted relays by
default. Review the [network inventory](docs/value-realisation/03-sovereignty-evidence.md)
when assessing an offline deployment.

Development-control access is authenticated and local, with a visible indicator
and revocation in Settings. Debug `cargo xtask dev` starts it on; ordinary and
release launches start it off. The bridge cannot enable its own access. See
[Development MCP](docs/development-mcp.md) for the control boundary.
