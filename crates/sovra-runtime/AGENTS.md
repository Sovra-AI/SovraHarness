# sovra-runtime

Async engine. Owns the tokio runtime (dedicated thread), command dispatch, and later the agent
loop, provider calls and tool execution.

- The only crate that may use `tokio`. Its public API stays runtime-agnostic: `Client` with
  `async-channel` endpoints, so the UI and tests never touch tokio.
- Every incoming `Command` is handled in `serve`'s exhaustive `match`; long work runs in its own
  task and reports progress as `Event`s. Never block a runtime worker with sync IO
  (use `tokio::task::spawn_blocking`).
- Tests drive the real runtime through `Client` and assert on events.
- Memory arrives as a `MemoryFactory` from the composition root; the runtime never names a
  backend. Source admission runs in owned background tasks; acknowledge capture only after
  persistence and surface storage failures. Agent replies are captured by `tap_agent_events`
  with explicit authorship — model processing must never stall the command loop.
