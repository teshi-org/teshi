#!/usr/bin/env bash
# Run the same quality gates CI enforces, before pushing.
#
# The terminal runner is a separate Cargo workspace, so it has its own
# formatting and lint commands. teshi-web is wasm32-only and must stay out of
# native workspace commands.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

# Git Bash and WSL do not always inherit the Windows cargo location.
[[ -d "$HOME/.cargo/bin" ]] && PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo >/dev/null 2>&1 && [[ -n "${USERPROFILE:-}" ]]; then
    cargo_bin="$USERPROFILE/.cargo/bin"
    command -v cygpath >/dev/null 2>&1 && cargo_bin="$(cygpath -u "$USERPROFILE")/.cargo/bin"
    [[ -d "$cargo_bin" ]] && PATH="$cargo_bin:$PATH"
fi
command -v cargo >/dev/null 2>&1 || {
    echo "preflight: cargo not found on PATH" >&2
    exit 1
}

run() {
    echo ":: $*"
    "$@"
}

run cargo fmt --all --check
run cargo fmt --manifest-path crates/teshi-tui-runner/Cargo.toml --check
run cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings
run cargo clippy --manifest-path crates/teshi-tui-runner/Cargo.toml --locked --all-targets -- -D warnings

if [[ "${1:-}" != "--no-tests" ]]; then
    run cargo test --workspace --exclude teshi-web --locked
    run cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked
fi

echo "preflight: OK"
