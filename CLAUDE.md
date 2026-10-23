@AGENTS.md

## Claude Code specifics

- A `PostToolUse` hook (`.claude/settings.json`) runs `cargo xtask hook` after every edit: it
  rustfmts the file and runs clippy on its crate (or the layering check for `Cargo.toml`).
  When it reports errors, fix the code — never loosen the rules.
- Relevant skills: `rust-pro` for idiomatic Rust; `systematic-debugging` for failures.
