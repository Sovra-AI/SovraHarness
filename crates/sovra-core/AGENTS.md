# sovra-core

Pure domain. Owns: the UI↔runtime protocol (`Command`, `Event`), domain types, state reducers,
and the port traits adapters implement.

- No IO, no async runtime, no GUI, no clocks/randomness (pass them in). Allowed external
  crates are listed in `CORE_ALLOWED_EXTERNAL` (`xtask/src/deps.rs`).
- Everything here is deterministic: every public function gets unit tests; use `proptest`
  for reducers/parsers once added.
- Keep `Command`/`Event` exhaustive. Adding a variant is how a feature starts.
- Port traits (e.g. `MemoryStore` in `memory.rs`) live here: async via `async-trait`,
  backend-neutral errors, no backend types in signatures. Pure retrieval logic
  (`fuse_rrf`, validity rules) stays here where it is unit-tested; adapters only do IO.
- Facts are bi-temporal (`created/valid/invalid/expired_at`); "current" means both
  `invalid_at` and `expired_at` are `None`; recall also checks `stopped_at` and
  effective validity dates. Never add a hard-delete to the port.
- **New agents & capabilities → Skills catalogue:** Any new specialist added to
  `specialists.rs` or new capability added to the protocol must be registered in
  `src/skills/builtin.rs` with a plain-English title, description, and example prompt
  for the Skills page (enforced by `skills::tests`).
