## What

<!-- One or two sentences. -->

## Why

<!-- The problem this solves. -->

## Validation

<!-- Commands run and results; include failures or unavailable checks. For UI
changes, describe Development MCP verification. Mark inapplicable items N/A. -->

## Checklist

- [ ] `cargo xtask ci` passed locally
- [ ] `CHANGELOG.md` updated under **Unreleased**, or a docs/CI-only exception requested
- [ ] `README.md` updated if a user-visible feature, tab, or command changed
- [ ] New specialist, tool, or capability is registered on the Skills page (`crates/sovra-core/src/skills/builtin.rs`)
- [ ] Tests cover the change (bug fix ⇒ regression test first)
- [ ] No secrets, keys, or machine-specific paths
