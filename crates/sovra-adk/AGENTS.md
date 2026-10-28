# sovra-adk

ADK-Rust adapter crate. Owns all `adk-*` dependencies (`adk-core`, `adk-agent`, `adk-model`,
`adk-runner`, `adk-session`, `adk-tool`) and bridges `sovra-core` domain configuration and
protocol events with the Agent Development Kit execution engine.

- The only crate allowed to depend on `adk-*` crates (enforced by `xtask/src/deps.rs`).
- Never leak `adk-*` types across the crate boundary; expose `sovra-core` types and a
  single `thiserror` `Error` enum.
- Never call `std::env::var` or `GeminiModel::from_env`; accept `sovra_core::AgentConfig`
  constructed once in `sovra-app`.
- Never depend on `tokio` directly (`sovra-runtime` owns the `tokio` runtime).
- Unit-test agent execution offline with fake `adk_core::Llm` implementations.
- **Skills page registration:** Whenever a new specialist agent or tool is wired into
  `sovra-adk`, register it in `sovra_core::skills::builtin_skills()` with plain-English
  grandma-friendly copy and an example prompt so users can discover and run it from the
  Skills page.
