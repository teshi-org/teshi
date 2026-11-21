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

PYTHON_BIN=""
for candidate in python3 python python3.13 python3.12 python3.11; do
    if command -v "$candidate" >/dev/null 2>&1 && "$candidate" -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 11) else 1)" >/dev/null 2>&1; then
        PYTHON_BIN="$candidate"
        break
    fi
done
if [[ -z "$PYTHON_BIN" ]]; then
    for candidate in python3 python; do
        if command -v "$candidate" >/dev/null 2>&1 && "$candidate" -c "import sys" >/dev/null 2>&1; then
            PYTHON_BIN="$candidate"
            break
        fi
    done
fi
if [[ -z "$PYTHON_BIN" ]]; then
    echo "preflight: python3/python not found on PATH" >&2
    exit 1
fi

run() {
    echo ":: $*"
    "$@"
}

run cargo fmt --all --check
run cargo fmt --manifest-path crates/teshi-tui-runner/Cargo.toml --check

# Fast contract and partition gates catch catalog/CI regressions in seconds.
run "$PYTHON_BIN" scripts/check-feature-e2e-catalog.py
run "$PYTHON_BIN" scripts/run-ci-suite.py check
run "$PYTHON_BIN" scripts/test-hosted-ui-transport.py
run "$PYTHON_BIN" scripts/test-release-workflow-contract.py
run "$PYTHON_BIN" scripts/test-nightly-workflow-contract.py
run "$PYTHON_BIN" -m unittest discover -s scripts/tests -p 'test_ci_*.py'
if "$PYTHON_BIN" -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 10) else 1)" >/dev/null 2>&1; then
    run "$PYTHON_BIN" scripts/test-browser-agent-package.py
else
    echo ":: Skipping test-browser-agent-package.py (requires Python >= 3.10)"
fi
if "$PYTHON_BIN" -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 11) else 1)" >/dev/null 2>&1; then
    run "$PYTHON_BIN" scripts/test_update_manifest.py
else
    echo ":: Skipping test_update_manifest.py (requires Python >= 3.11)"
fi

if [[ "${1:-}" != "--no-tests" ]]; then
    run cargo test --workspace --exclude teshi-web --locked
    run cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked
fi

# Run --all-features clippy after default-feature tests (matching CI) to avoid feature-flip rebuilds.
run cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings
run cargo clippy --manifest-path crates/teshi-tui-runner/Cargo.toml --locked --all-targets -- -D warnings

echo "preflight: OK"
