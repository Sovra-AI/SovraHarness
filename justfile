# SovraHarness command runner (delegates to `cargo xtask`).

# Watch files, rebuild/relaunch the app, and stream logs (ensures watchexec is installed)
default: dev

# Install pinned CI tools and the Watchexec development runner
setup:
    @if ! watchexec -V 2>/dev/null | grep -qx "watchexec 2.7.2"; then cargo xtask setup; fi

# Watch files, rebuild/relaunch the app, and stream logs
dev: setup
    cargo xtask dev

# Run the desktop app (debug build)
run:
    cargo xtask run

# Run all CI gates: fmt, clippy, check-deps, check-styles, deny, shear, tests
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

# Release build + Velopack packaging
dist:
    cargo xtask dist
