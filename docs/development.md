# Native development

Build and launch Sovra from a source checkout. Install Git and Rust through
rustup first; [rust-toolchain.toml](../rust-toolchain.toml) selects the toolchain.
Commands below run from the repository root. Environment-variable examples use
POSIX shell syntax; in PowerShell, set variables with `$env:NAME = "value"` before
running the command.

## One-time setup

| Platform | Build prerequisites |
| --- | --- |
| Linux | Run `./scripts/install-linux-deps.sh` to install the native system packages |
| macOS | Install Xcode command-line tools |
| Windows | Install Visual Studio C++ build tools and Windows SDK; use the MSVC Rust target |

```sh
cargo xtask setup
cargo xtask dev
```

Setup installs the pinned check/test tools. `just` is optional and delegates to
xtask. The app needs no Node.js or Python runtime. Generated frontend builds and
optional evaluation scripts have separate prerequisites in their guides.
Packaging needs additional tools; see [Releasing](releasing.md).

## Launch and rebuild

`cargo xtask dev` builds and launches **once**, with `SOVRA_DEV=1`. There is no
file watcher, save-triggered rebuild, or automatic relaunch. Close the running
session and rerun the command after source changes. Do not start `cargo watch`
or a competing reload loop.

An OS-held lock at `target/dev/watch.lock` prevents concurrent dev sessions.
The operating system releases it when the owner exits; do not delete the file
to bypass a running session. Inspect build and runtime output in the launching
terminal. Daily application logs also live under the [app data root](configuration.md).

`cargo xtask run` starts a debug build without setting the development flag.
Development mode's automatic startup requires a debug build and `SOVRA_DEV=1`.
Ordinary launches start control off unless the flag was explicitly inherited;
release builds ignore it. Use [Development MCP](development-mcp.md) to attach a
client to the existing native window.

## Development data and credentials

A debug development launch stores connection keys in separate plaintext files
under `<app data>/dev-credentials/`, with owner-only permissions on Unix. Release
builds use the OS keychain. Keys are not copied between stores.

Other workspace data normally shares the same app-data root. A development launch
is not automatically an isolated test workspace. To use one:

```sh
SOVRA_DATA_DIR=/absolute/path/to/sovra-test-data cargo xtask dev
```

The override is honored only in debug builds with `SOVRA_DEV=1`; the dev launcher
sets that flag. Keep test data and credentials outside the checkout. See
[Configuration](configuration.md) for storage paths and backup limitations.

## Checks

| Command | Purpose |
| --- | --- |
| `cargo xtask ci` | All required local gates |
| `cargo xtask check-docs` | Documentation link and diagram checks |
| `cargo xtask check-deps` | Allowed crate dependencies and ownership |
| `cargo xtask check-styles` | Native styling boundaries |
| `cargo xtask fix` | Apply formatting and Clippy fixes; review the diff afterward |
| `cargo nextest run -p <crate>` | Test one crate |

For UI behavior changes, inspect and exercise the native app using Development
MCP as required by [AGENTS.md](../AGENTS.md). Headless tests complement that check.
A local pass does not prove macOS/Windows/Linux installer behavior; CI and release
verification have their own platform coverage.

## Monitoring the build cache

The dev launcher reports the effective Cargo target size, including
`CARGO_TARGET_DIR`, against an advisory 30 GiB budget. Set
`SOVRA_TARGET_BUDGET_GIB` to change the warning threshold. It does not clean
while building or automatically delete output.

Stop dev sessions and other Cargo commands before reclaiming cache:

```sh
cargo xtask clean-cache
cargo xtask clean-cache reclaim --dry-run
cargo xtask clean-cache reclaim
```

`just clear-cache` and `just free-cache` are aliases for reclaim. Inspect the
command's output before deleting caches. Optional `--cargo-home` also reclaims
recreatable registry/cache/source directories; it does not remove credentials or
installed Cargo binaries. Cache cleanup does not reset the app's data.

Build profiles live in [Cargo.toml](../Cargo.toml), and the job limit in
[.cargo/config.toml](../.cargo/config.toml). Different feature combinations and
profiles can produce additional dependency artifacts; do not clean after every edit.

## Sign-in integration status

Local account creation accepts a name, optionally paired email/password, and
Google sign-in with a configured desktop OAuth client. The runtime persists
account/session records through `MemoryStore`; passwords use Argon2id hashes.
Signing out clears the session without deleting the account or workspace data.
Local accounts are not separate encrypted data stores or company SSO.

Google client precedence is saved setup, startup environment, then build-time
values. See [Google](google.md). Sign-in persistence errors are logged rather
than propagated as failed sign-in events, so a successful visible sign-in does
not by itself guarantee the session survives restart.

Implementation: `sovra-core::auth`, `sovra-providers::auth`,
`sovra-runtime::auth`, and the store's auth-record adapter.

## Two Sovra computers on one machine

Groups can be exercised with two isolated debug instances. The dev lock permits
one launcher, so use its built binary for the second instance:

1. Run `SOVRA_GROUP_REACH=local cargo xtask dev` for the first instance.
2. Launch a second instance from the same build with a separate data directory:

   ```sh
   SOVRA_DEV=1 SOVRA_GROUP_REACH=local \
   SOVRA_DATA_DIR=/absolute/path/to/sovra-second-data target/debug/sovra
   ```

   Adjust the binary path if using `CARGO_TARGET_DIR`; Windows uses `sovra.exe`.
3. Start a group in one instance and join it from the other.

Each instance has its own identity, data, and Development MCP endpoint. This
explicit multi-computer test is separate from the normal single-window workflow.
It does not establish two-device Internet connectivity or production relay availability.
