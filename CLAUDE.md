@AGENTS.md

## Claude Code specifics

- A `PostToolUse` hook (`.claude/settings.json`) runs `cargo xtask hook` after every edit: it
  rustfmts the file and runs clippy on its crate (or the layering check for `Cargo.toml`).
  When it reports errors, fix the code — never loosen the rules.
- Relevant skills: `rust-pro` for idiomatic Rust; `systematic-debugging` for failures.
- **Skills page rule (Claude, Codex, Grok, Cursor):** Whenever you add a new specialist
  agent (`crates/sovra-core/src/specialists.rs`), tool, or capability, you **must** register
  it in `crates/sovra-core/src/skills/builtin.rs` with a plain-English grandma-friendly
  title, description, and concrete example prompt so it appears on the **Skills** page
  (`crates/sovra-ui/src/skill_library.rs`).
