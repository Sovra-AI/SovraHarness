# Building and releasing installers

Sovra uses Velopack 1.2.161 for native installers and update feeds. The app handles
Velopack install hooks at startup, but does not yet check for or apply updates
through an in-app update flow.

## Build a local installer

Install the .NET SDK required by the packaging workflow (.NET 8), then:

```sh
dotnet tool install -g vpk --version 1.2.161
cargo xtask dist
```

Ensure `vpk` is on `PATH`, or set `SOVRA_VPK` to its executable. The command checks
the exact CLI version before building. It builds the host platform with
`--release --locked`, stages only the app binary, and requires exactly one
nonempty native installer. Local packaging uses `--skip-updates`.

Outputs are in `target/dist/<runtime>-<process-id>/`; staging is in
`target/pack/<runtime>-<process-id>/`. Unlike ordinary builds, this command
explicitly uses the checkout's `target/`, even if `CARGO_TARGET_DIR` is set.

| Host | Installer | Local architecture mappings |
| --- | --- | --- |
| macOS | `.pkg` | ARM64, x86-64 |
| Windows | `.exe` | ARM64, x86-64 |
| Linux | `.AppImage` | ARM64, x86-64 |

These mappings are not proof that every architecture has been release-tested.
A push to `main`, and a matching `vX.Y.Z` tag, packages macOS `sovra` on
`macos-15`, Windows `sovra.exe` on `windows-latest`, and Linux `sovra` on
`ubuntu-24.04`, with `cargo build --release --locked --package sovra-app`.
That workflow does not provide the full local architecture matrix. Build each
installer on its target OS.

## Prepare a release

1. Update the workspace version in `Cargo.toml` and keep `Cargo.lock` consistent.
2. Move the relevant **Unreleased** entries in `CHANGELOG.md` under the release
   version and date; update user guides for changed behavior.
3. Run `cargo xtask ci`. Resolve failures before releasing.
4. Build and install-test the packages on their target systems. Verify normal
   startup, credential storage and Development mode starting off.
5. Verify signing configuration and the resulting signatures before distribution.
6. A push to `main` that updates the downloadable executables publishes them.
   The workflow reads the package version from `Cargo.toml` (never the branch
   name `main`), keeps that version and the changelog heading together, builds
   the three release executables, and publishes that version on the existing
   GitHub Release and Velopack feed. A matching `vX.Y.Z` tag runs the same
   [release.yml](../.github/workflows/release.yml) when the tag equals `v` plus
   that package version. A release that already has the macOS, Windows, and
   Linux executables is not published again. A release that exists but is
   missing any of those three is not finished: publishing uploads all three
   and fails unless they are all on that version afterward. Do not bump the
   version on a push that does not publish executables.

The workflow builds with the lockfile (`cargo build --release --locked --package sovra-app`)
on `macos-15` (`sovra`), `windows-latest` (`sovra.exe`), and `ubuntu-24.04`
(`sovra`), packages each platform, then publishes all three channels to one
GitHub Release with Velopack update feeds. Signing still skips when its secrets
are absent. In-app update checking is not implemented. It does not run the
entire CI suite itself, so release from a reviewed revision with green CI.

## Signing

Signing is conditional: jobs skip it when their configured signing secret is
absent. A successful packaging job therefore does not establish that the
installer is signed or notarized.

- macOS uses the build and installer certificates, P12/keychain passwords,
  Apple ID/app-specific password/team, and signing identity named in the workflow.
  Entitlements are in `packaging/macos/sovra.entitlements`.
- Windows signing expects Azure OIDC credentials and a provisioned
  `packaging/windows/artifact-signing.json`. That JSON file is not included in
  this checkout. The workflow does not generate it; supply the configuration
  required by `--azureTrustedSignFile` before enabling Windows signing.

Keep credentials in the release secret store, never in the checkout, package
staging folder or documentation. The exact secret names and signing arguments
live in the workflow header and packaging step.

## Release limitations

Google OAuth scopes and verification have their own [release requirements](google.md#scopes-and-google-verification-release-blocker).
Group Internet reach currently uses development relay infrastructure; see
[Groups](groups.md). Neither installer
creation nor a generated update feed proves those rollout requirements are met.

See [SECURITY.md](../SECURITY.md) for reporting and secret handling, and
[configuration](configuration.md) for backup limitations before upgrades.
