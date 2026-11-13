# Contributing to SovraHarness

How Sovra people open a branch, pass CI, and land a pull request on `main`.

Source on the default branch `main` is under the **Sustainable Use License**
(see [LICENSE](LICENSE)). This is n8n's Sustainable Use License, which n8n's
documentation allows other projects to use. The model is fair-code:
source-available, and free to use and extend for your own internal business
purposes or for non-commercial or personal use, with the limits in that
license. It is not an OSI open-source license. Files named `.ee.` or kept in
a directory whose name contains `.ee` are under
[LICENSE_EE.md](LICENSE_EE.md) instead. n8n does not own SovraHarness.

The canonical engineering rules live in [AGENTS.md](AGENTS.md) (plus a short
`AGENTS.md` per crate) and apply to humans and coding agents alike. This file
covers the workflow around them; it does not duplicate them.

## Getting started

```sh
# Rust via https://rustup.rs — the pinned toolchain (1.99.1) installs on first build
# Linux only: ./scripts/install-linux-deps.sh
cargo xtask setup   # once: install pinned development tools
cargo xtask dev     # one-shot debug build + launch; re-run yourself to reload
```

Read before writing code:

- [docs/architecture.md](docs/architecture.md) — why the layering is what it is
- [docs/development.md](docs/development.md) — the one-shot dev session
- [docs/styling.md](docs/styling.md) — the native design system (required for UI work)
- [docs/README.md](docs/README.md) — map of every top-level guide

## Best practices

- **Definition of done:** `cargo xtask ci` passes locally. CI runs the same
  gates; don't lean on it to find what you could have caught first.
- **Decide in core, IO in adapters, render in UI.** New capabilities flow
  end-to-end: `Command`/`Event` variants in `sovra-core`, handling in
  `sovra-runtime`, a pure reducer in `sovra-core`, rendering in `sovra-ui`.
- **Fix the code, never the rules.** `[workspace.lints]`, `clippy.toml`,
  `deny.toml`, and `xtask/src/deps.rs` are not to be loosened to make a
  check pass. If a rule is genuinely wrong, raise it with reasoning.
- **Bug fix ⇒ regression test first.** Pure logic gets unit tests next to
  the code; runtime behavior is driven through `Client` and asserted on
  `Event`s; UI uses headless GPUI tests with `Client::loopback()`.
- **Dependencies** are declared once in root `[workspace.dependencies]`.
  No OpenSSL (rustls only), no git dependencies, no wildcard versions;
  remove unused deps (cargo-shear fails CI).
- **Secrets never land in the repo or logs.** Installed/release API keys use
  the OS keychain. Debug `SOVRA_DEV=1` uses separate owner-only development
  files outside the checkout; see [SECURITY.md](SECURITY.md).

## Branches, commits, and review

Several people work on this repository. Day-to-day work goes through pull
requests so CI, changelog, and review happen before `main` moves.

1. Branch from the latest `main`. Keep the branch short-lived and focused.
   Name it `feat/<topic>`, `fix/<topic>`, `docs/<topic>`, or `chore/<topic>`.
2. Prefer small, reviewable commits. Prefix every commit message with a
   gitmoji and conventional scope (`✨ feat(ui): …`, `🐛 fix(runtime): …`,
   `📝 docs: …`). No attribution or co-author trailers.
3. **Mandatory Changelog & README rule:** Every change that adds, changes,
   fixes, or removes behaviour updates [CHANGELOG.md](CHANGELOG.md) under
   **Unreleased**, and [README.md](README.md) whenever user-visible features,
   tabs, or commands change. CI fails the pull request if `CHANGELOG.md` is
   untouched; add the `skip-changelog` label only for a docs-only or CI-only
   change that has nothing to record.
4. Open a pull request against `main`. Fill in the template. Request review
   from a code owner if GitHub did not already. CI must be green.
5. Squash-merge the pull request. The squash title is the PR title (gitmoji
   prefix, same style as commits). Delete the branch on merge.

Do not force-push `main`. Do not commit to `main` on your laptop except for
an agreed hotfix; even then, prefer a pull request so CI still runs.

A new specialist, tool, or user-facing capability must be registered on the
**Skills** page (`crates/sovra-core/src/skills/builtin.rs`) with a
plain-English title, description, and example prompt.

## GitHub Actions

`.github/workflows/ci.yml` runs on every pull request and every push to
`main`:

| Job         | Runner        | What it proves                                      |
| ----------- | ------------- | --------------------------------------------------- |
| Lint        | Ubuntu        | fmt, layering, styles, docs, deny, shear, clippy    |
| Test        | macOS         | `cargo nextest` including headless GPUI UI tests    |
| Compile     | Windows       | `cargo check --workspace --locked --all-targets`    |
| Changelog   | Ubuntu        | `CHANGELOG.md` changed (pull requests only)         |
| **CI**      | Ubuntu        | Aggregator — require this check once GitHub Team is on |

A new push on a pull request cancels the previous run for that PR. Pushes to
`main` are never cancelled, so a green result on `main` is trustworthy.

`.github/workflows/release.yml` runs on every push to `main` and on a
`vX.Y.Z` tag that matches the package version. The package version comes from
the root `Cargo.toml`. It is never the branch name `main`.

Dependabot opens weekly grouped PRs for GitHub Actions and Rust patch
updates. `gpui-kit` is ignored; bump it in a dedicated PR after reading that
version's source.

Runner availability, billing limits and private-repository protection depend on
this organisation's current GitHub plan and settings. Check those settings when
jobs cannot acquire a runner; the workflow files do not prove the account's plan.

## Branch protection

Where the organisation's plan supports it, configure a ruleset on `main`.
Until protection is enabled, follow these rules during review and treat a red
**CI** check as a blocked merge.

Turn on a ruleset (or classic branch protection) on
`main`:

- Require a pull request before merging, 1 approving review
- Dismiss stale reviews on new commits
- Require review from Code Owners (`.github/CODEOWNERS`)
- Require conversation resolution
- Require status checks: **CI** (the aggregator job), and do not skip if
  they are pending
- Block force pushes and branch deletions
- Optionally require linear history (squash-only is already the merge
  setting)

## Releasing

A push to `main` that updates the downloadable executables bumps `version` in
the root `Cargo.toml` above the version already published on the GitHub Release
and Velopack feed, and moves the **Unreleased** changelog entries under that
same version in the same change. The package version is a semantic version,
never the branch name `main`. `.github/workflows/release.yml` then
release-builds the three executables — macOS `sovra` on `macos-15`, Windows
`sovra.exe` on `windows-latest`, and Linux `sovra` on `ubuntu-24.04` — with
`cargo build --release --locked --package sovra-app`, and publishes them on
that existing feed. A matching `vX.Y.Z` tag runs the same workflow when the tag
equals that package version. Signing is skipped when its secrets are absent.
In-app update checks are not implemented. Do not bump the version on a push
that does not publish executables, and do not publish the same version twice.
Follow the [release guide](docs/releasing.md) before publishing.
