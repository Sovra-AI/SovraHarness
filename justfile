# SovraHarness command runner (delegates to `cargo xtask`).

# One-shot debug build + launch (re-run yourself to reload; no auto-reload)
default: dev

# Install pinned CI tools (deny, nextest, shear)
setup:
    cargo xtask setup

# One-shot debug build + launch with SOVRA_DEV=1
dev:
    cargo xtask dev

# Run the desktop app (debug build, without SOVRA_DEV)
run:
    cargo xtask run

# Run all CI gates: fmt, clippy, check-deps, check-styles, check-docs, deny, shear, tests
ci:
    cargo xtask ci

# Auto-fix fmt + clippy warnings
fix:
    cargo xtask fix

# Enforce crate layering rules
check-deps:
    cargo xtask check-deps

# Enforce UI colour boundary and single SVG brand asset
check-styles:
    cargo xtask check-styles

# Enforce README mermaid graphs and that docs markdown links resolve
check-docs:
    cargo xtask check-docs

# Report Cargo target disk use (passthrough: status / reclaim / flags)
clean-cache *args:
    cargo xtask clean-cache {{args}}

# Clear reclaimable Cargo build caches (refuses while cargo xtask dev holds the lock)
clear-cache *args:
    cargo xtask clean-cache reclaim {{args}}

# Free Rust build-cache space using the same guarded cleanup
alias free-cache := clear-cache

# Release build + Velopack packaging
dist:
    cargo xtask dist
