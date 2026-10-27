# sovra-files

Local document/media decoding and managed attachment IO behind `sovra_core::files::FileStore`.

- Runtime owns calls through blocking tasks; do not add a runtime or GUI dependency.
- Accept paths only for an explicit user-selected import. Persist validated opaque IDs.
- Bound reads, archive expansion, image allocation and extracted output. Never execute
  file contents, write archive members to disk, or fetch linked resources.
- Preserve recovery guidance and extraction limitations; do not claim unread media or
  excerpts represent a complete file. Do not log document content or image bytes.
- Test codecs with small synthetic fixtures and keep private storage permissions.
