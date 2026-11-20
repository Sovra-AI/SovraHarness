---
name: add-a-sovra-feature
description: Use when working in a SovraHarness checkout to add a new feature, command, helper, tool or connection to Sovra, from the core types through the runtime to the screen.
metadata:
  sovra-title: Add a feature to Sovra
  sovra-example: Add a way to pin a chat to the top of the chat list in Sovra
---

# Add a feature to Sovra

Work inside a SovraHarness checkout. `AGENTS.md` at its root (and the closest
`AGENTS.md` in each crate) holds the binding rules. Read them before the first
edit, and follow them where they say more than this skill does.

1. **Find the closest existing feature** and follow its shape. Search for a
   similar `Command` variant in `crates/sovra-core` and read how it travels
   end to end. Do not copy-paste a sibling implementation: extract the shared
   part first.
2. **Decide in core.** Add the `Command` (UI → runtime) and `Event`
   (runtime → UI) variants in `sovra-core`. Put the rules in a pure function or
   reducer there, with unit tests in `#[cfg(test)] mod tests` next to it. Core
   has no IO, no async runtime and no GUI.
3. **Do IO in an adapter.** Handle the command in `sovra-runtime`, or in the
   adapter crate that owns the dependency (see the crate list in `AGENTS.md`).
   Spawn tasks through `sovra-runtime` so each has an owner and is cancelled on
   shutdown. Only `sovra-runtime` uses tokio.
4. **Render in `sovra-ui`.** Send the command and fold the event. Follow
   `change-sovra-screens` for anything visible.
5. **Let the compiler guide you.** The enums are exhaustive, so `cargo check`
   lists every `match` that needs the new variant. Never add `_ =>` arms on
   Sovra's own enums.
6. **Register it on the Skills page.** A new specialist
   (`crates/sovra-core/src/specialists.rs`), tool or user-facing capability must
   get an entry in `crates/sovra-core/src/skills/builtin.rs` with a plain-English
   title, a one-sentence description and a concrete example prompt
   (`Skill::friendly_card_copy`). A test in `sovra-core` fails without it.
7. **Wire configuration once.** Environment, paths and settings are read in
   `sovra-app` and passed down. A new crate is only for a distinct dependency set
   or real boundary, and must be added to `xtask/src/deps.rs`.
8. **Test through the boundary.** Runtime: drive `Client` and assert on `Event`s.
   UI: a headless `#[gpui_kit::test]` with `Client::loopback()`.
9. Finish with `finish-a-sovra-change`.

Rust rules that catch people out: no `unsafe`, no `unwrap()` outside tests,
`tracing` instead of `println!`, one `thiserror` enum per library crate, and
every dependency declared once in the root `[workspace.dependencies]`.
