# Contributing to SovraHarness

This repository is **closed source and proprietary to Sovra** (see
[LICENSE](LICENSE)). Contributions come from Sovra personnel and authorized
collaborators only. Do not share code, discussions, or artifacts outside
Sovra.

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
- **Secrets never land in the repo.** API keys go in the OS keychain
  (keyring), never in files, logs, or commit history. See
  [SECURITY.md](SECURITY.md).

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
| Lint        | Ubuntu        | fmt, layering, styles, deny, shear, clippy          |
| Test        | macOS         | `cargo nextest` including headless GPUI UI tests    |
| Compile     | Windows       | `cargo check --workspace --locked --all-targets`    |
| Changelog   | Ubuntu        | `CHANGELOG.md` changed (pull requests only)         |
| **CI**      | Ubuntu        | Aggregator — require this check once GitHub Team is on |

A new push on a pull request cancels the previous run for that PR. Pushes to
`main` are never cancelled, so a green result on `main` is trustworthy.

`.github/workflows/release.yml` still runs on a `vX.Y.Z` tag.

Dependabot opens weekly grouped PRs for GitHub Actions and Rust patch
updates. `gpui-kit` is ignored; bump it in a dedicated PR after reading that
version's source.

GitHub Free includes 2,000 Actions minutes per month on private repositories
(Linux 1×, Windows 2×, macOS 10×). If every job fails in a few seconds with
no runner, the month's minutes are used up or the spending limit is $0.
Raise the limit at
[Sovra-AI billing](https://github.com/organizations/Sovra-AI/settings/billing)
or wait for the monthly reset. The split jobs above exist so a green `main`
costs fewer minutes than a full three-OS `cargo xtask ci`.

## GitHub Team (branch protection)

This organisation is on **GitHub Free**. Private repositories cannot require
pull requests, status checks, or CODEOWNERS. Until the org is on
[GitHub Team](https://github.com/pricing) (~$4/user/month), those rules are
social: follow this file, and treat a red **CI** check as a blocked merge.

Once Team is enabled, turn on a ruleset (or classic branch protection) on
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

Bump `version` in the root `Cargo.toml`, move the **Unreleased** changelog
entries under the new version, and push a `vX.Y.Z` tag.
`.github/workflows/release.yml` builds, signs, packages (Velopack), and
publishes the release that installed apps auto-update from.
