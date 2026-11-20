---
name: finish-a-sovra-change
description: Use when a change to SovraHarness is written and needs to pass every check, update the changelog and be committed, or when asking whether a Sovra change is done.
metadata:
  sovra-title: Finish a change to Sovra
  sovra-example: I've finished my change to Sovra, help me get it ready to commit
---

# Finish a change to Sovra

A change is done only when `cargo xtask ci` passes. Never call it finished
without running it, and if it fails, show the failure.

1. **Tidy.** Run `cargo xtask fix` for formatting and clippy suggestions. Fix
   anything left by hand.
2. **Run every gate:** `cargo xtask ci`. It runs formatting, clippy, tests, the
   layering check (`check-deps`), styling (`check-styles`), docs links and
   diagrams (`check-docs`) and unused dependencies. Fix the code, never the rules.
3. **Check it in the app** when people will see the change: launch once with
   `cargo xtask dev` and test it through Development mode
   (`docs/development-mcp.md`). Wait for real completion, not just accepted
   input. If that was not possible, say what was not checked.
4. **Update `CHANGELOG.md`** under `## [Unreleased]`: a short bold summary and
   one or two plain sentences on what changed for the person using Sovra. Update
   `README.md` and the guide in `docs/` when a feature or workflow changes, and
   list open work only in `docs/roadmap.md`.
5. **Skills page.** A new helper, tool or capability needs its entry in
   `crates/sovra-core/src/skills/builtin.rs` (see `add-a-sovra-feature`).
6. **Commit with a gitmoji first,** for example `✨ feat(plugins): …`,
   `🐛 fix(ui): …` or `📝 docs: …`. Never commit secrets, `.env` files or
   machine-specific settings.
7. **Releases are separate.** Only a push to `main` that publishes new
   executables bumps the version in the root `Cargo.toml` and moves the changelog
   notes under that version (`docs/releasing.md`). Other changes leave the
   version alone.
