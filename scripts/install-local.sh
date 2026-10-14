#!/usr/bin/env bash
# Installs the teshi CLI from this checkout onto PATH so every shell and agent
# tool resolves the same binary.
#
# `--profile dev` is deliberate. `cargo install` defaults to `release`, which
# enables `lto = true` (Cargo.toml). Every iteration then pays a full LTO link
# (~4 min) and cannot reuse the `target/debug` cache that `cargo build` fills.
# With the dev profile the install stays incremental: ~14 s after a one-line
# edit, ~4 s when nothing changed.
#
# Build the packaged installer (scripts/build-exe-setup.ps1) only when the
# installer or the update path itself is what you are validating.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"

cargo_args=(install --path apps/teshi-cli --profile dev)
if [[ "${1:-}" == "--force" ]]; then
  cargo_args+=(--force)
fi

cargo "${cargo_args[@]}"

installed="${CARGO_HOME:-$HOME/.cargo}/bin/teshi"
resolved="$(command -v teshi || true)"
if [[ -n "$resolved" && "$resolved" != "$installed" ]]; then
  echo "warning: PATH resolves 'teshi' to '$resolved' instead of '$installed'." >&2
  echo "warning: reorder PATH or uninstall the other copy so agents pick up this build." >&2
fi

echo "Installed: $installed"
"$installed" --version
