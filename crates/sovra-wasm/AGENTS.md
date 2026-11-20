# sovra-wasm

The sealed box plugin code runs in: `sovra_core::plugins::PluginCodeEngine` on the
`wasmi` interpreter. The only crate naming `wasmi`.

- Never add WASI or a new host import. Plugin code gets `sovra.log`, and
  `sovra.fetch`/`sovra.take_reply` only when its plugin lists addresses. A new
  import also needs core `check_interface`, `docs/plugins.md` and a test here.
- Network policy is not decided here: `fetch` hands every request to the runtime's
  `CodeHost`, which applies core `check_fetch` and the public-address filter.
- Every call gets a fresh `Store` with fuel and `StoreLimits` (memory, tables,
  table elements, instances). Any new resource a module can declare needs a cap
  and a regression test with a hostile module.
- Bound every read of guest memory. Messages that reach the helper are plain
  sentences; wasmi's own error text stays in `tracing::debug!`.
- Runtime owns threads and timeouts. Do not add async or IO dependencies.
