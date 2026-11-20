# Date helper: Sovra's first code plugin

The source of `plugin.wasm` at the top of this repository. Sovra runs it in its
sealed box as three tools: count the days between two dates, find a date's
weekday, and find the date some days before or after another. It imports nothing,
so it cannot reach files, programs or the internet.

```sh
cargo xtask plugin-wasm   # rebuild ../../plugin.wasm reproducibly
cargo xtask check-plugin  # check the committed module matches this source
```

The tools it answers are listed in [`sovra-plugin.json`](../../sovra-plugin.json).
The contract a module follows is in
[Plugins with code](../../docs/plugins.md#plugins-with-code).
