# justfile for hyprland-preview-share-picker
# Run `just` to see all available commands. Cargo recipes assume you
# are inside the dev shell (`nix develop`) where gtk4 + layer-shell
# libs are on the linker path; the nix-* recipes work standalone.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Default — list recipes.
default:
    @just --list --unsorted

# ─────────────────────────── Build & Run ───────────────────────────

# Build the release binary into ./target/release/.
build:
    cargo build --release

# Run the picker. Forward extra args after `--`, e.g. `just run -- --help`.
run *ARGS:
    cargo run -- {{ARGS}}

# ─────────────────────────── Quality ───────────────────────────

# Auto-fix formatting drift (uses rustfmt.toml).
fmt:
    cargo fmt --all

# Strict read-only gate — same checks CI runs. Fails on format drift
# or any clippy warning. Run before pushing.
lint:
    cargo fmt --all --check
    cargo clippy --all-targets --all-features -- -D warnings

test:
    cargo test --all

# Full local gate before push: lint + test + nix build.
check: lint test nix-check

clean:
    cargo clean
    rm -rf result

# ─────────────────────────── Nix ───────────────────────────

nix-build:
    nix build .#hyprland-preview-share-picker --print-build-logs

nix-check:
    nix flake check --print-build-logs

# Bump flake.lock inputs (nixpkgs) and re-validate. CI does this
# weekly; run it manually to pull updates early.
update:
    nix flake update
    nix flake check --print-build-logs
