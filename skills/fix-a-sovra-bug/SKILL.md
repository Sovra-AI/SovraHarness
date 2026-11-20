---
name: fix-a-sovra-bug
description: Use when something in Sovra is broken or behaves wrongly and the fix is being made in a SovraHarness checkout, including failing tests and build errors.
metadata:
  sovra-title: Fix a bug in Sovra
  sovra-example: Sovra forgets my Text size setting after a restart, can you fix it?
---

# Fix a bug in Sovra

Find the cause before changing code, and prove the fix with a test.

1. **Reproduce it.** Write down the exact steps, what happens and what should
   happen. For screen problems, launch once with `cargo xtask dev` and watch the
   log in that terminal. For a test failure, run just that crate:
   `cargo nextest run -p <crate>`.
2. **Find the layer.** Follow the data: the view sends a `Command`, the runtime
   or an adapter does the work, a pure reducer in `sovra-core` folds the `Event`,
   the view renders it. Decide which step first goes wrong before editing.
3. **Write the failing test first,** in the layer that owns the mistake: a unit
   test beside core logic, a `Client` test for the runtime, a headless
   `#[gpui_kit::test]` for the UI. Run it and see it fail for the reason you
   expect.
4. **Fix the cause, not the symptom.** Make the smallest change that makes the
   test pass. Never loosen lints, `clippy.toml`, `deny.toml` or
   `xtask/src/deps.rs` to get past a failure, and never add `#[allow(..)]`
   without a `reason`.
5. **Check you did not break neighbours.** Run the crate's tests again, then
   follow `finish-a-sovra-change`.

If two or three attempts have not found the cause, stop and list what you know,
what you ruled out and what you would check next, instead of guessing further.
