# Dependency license evidence

Dependency license and advisory policy lives in [deny.toml](../deny.toml). Run
`cargo deny --all-features check` when changing dependencies. The project license
is separate: see [LICENSE](../LICENSE).

## finl_unicode 1.5.0 clarification

The published dependency has inconsistent license metadata in the
`finl_unicode 1.5.0` archive. Its manifest declares
`(MIT OR Apache-2.0) AND Unicode-DFS-2016`, while `LICENSE-UNICODE` is the complete
**UNICODE LICENSE V3** text. That file matches the [SPDX Unicode-3.0 text](https://spdx.org/licenses/Unicode-3.0.html)
exactly after whitespace normalization. The other two license files retain the
MIT/Apache choice. The registry archive SHA-256 matches Cargo.lock:
`80bb028c8b4148c9ee0cca68fcd9add6044e81d3619f48577ddf13a263d047a2`;
all three inspected files match the corresponding archive members byte for byte.

`deny.toml` now clarifies **only this exact version** as
`(MIT OR Apache-2.0) AND Unicode-3.0`, with hashes of all three license files.
This corrects the metadata mismatch using the [cargo-deny clarification mechanism](https://embarkstudios.github.io/cargo-deny/checks/licenses/cfg.html#clarify);
it does not expand the already permitted licenses, waive the Unicode obligation,
change the confidence threshold or exclude a dependency. An upstream version or
license-text change requires renewed review. A temporary configuration with an
incorrect Unicode-file hash was also checked: it rejects the clarification and
fails the license gate, as intended. SHA-256 fingerprints of raw files:

| File | SHA-256 |
| --- | --- |
| LICENSE-MIT | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| LICENSE-APACHE | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| LICENSE-UNICODE | `361d7912957842f2ce61c774b14afba41ae91df84882982e6f7e749e01c07c91` |

