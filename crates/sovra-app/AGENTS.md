# sovra-app

The `sovra` binary — composition root. Owns process setup and wiring only.

- Order in `main`: Velopack hook (must be first) → paths → logging → runtime → UI.
- Reads configuration, env vars and paths once here and passes values down. No domain logic;
  if a function here needs a test, it belongs in a library crate.
- The only crate that may use `velopack`.
