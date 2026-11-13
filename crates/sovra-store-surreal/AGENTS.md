# sovra-store-surreal

SurrealDB adapter for `sovra_core::MemoryStore`. Owns: connection (`engine::any`), schema,
SurrealQL, row types. The only crate that may name `surrealdb`.

- Callers hold `Arc<dyn MemoryStore>`; never leak SurrealDB types, errors or SurrealQL
  through the port. Map every native error into `MemoryError`.
- The desktop uses `surrealkv://dir`; tests use `mem://`. Remote WebSocket access
  is not enabled by the workspace feature set. Deployment choices belong in the
  composition root and `StoreLocation`.
- Schema lives in `src/schema.rs`: every statement uses `DEFINE ... OVERWRITE` so re-applying
  converges. **Never `IF NOT EXISTS`** — it silently keeps a stale definition. Data-shape
  changes bump `SCHEMA_VERSION` and get a migration arm in `open`.
- Facts are bi-temporal and never deleted: supersession sets `invalid_at`/`expired_at` and
  links `supersedes`. Don't add DELETE paths.
- `gpui-kit`-style version discipline applies: SurrealDB is BUSL-1.1 (excepted in
  `deny.toml`); new `surrealdb-*` crates appearing in the tree must be added there
  deliberately, after reviewing their license terms.
- Every port method gets an integration test in `tests/store.rs` against `mem://` — real
  engine, no mocks. Verify SurrealQL against the pinned source
  (`~/.cargo/registry/src/*/surrealdb-3.3.4`) or docs for that exact version, not memory.
