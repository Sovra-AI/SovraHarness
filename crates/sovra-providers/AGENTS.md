# sovra-providers

IO adapters behind `sovra_core::models::ModelProvider`: HTTP catalog discovery,
connection validation, catalog/config persistence and OS credential storage.

- No UI, environment reads or direct tokio dependency. Runtime owns async tasks;
  sync storage/keychain operations run through runtime's `spawn_blocking`.
- Credentials never enter serializable metadata, Debug output, URLs or logs.
- Bound requests and response sizes, retain cached catalogs on refresh errors,
  and preserve unknown metadata as unknown rather than inferring capabilities.
- Test parsers with representative fixtures and HTTP behavior with local servers.
