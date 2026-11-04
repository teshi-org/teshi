#!/usr/bin/env bash
# Installs the teshi CLI into ${CARGO_HOME:-$HOME/.cargo}/bin, giving a `teshi`
# name that outlives the build directory (survives `cargo clean`, usable by
# tools pinned to an absolute path).
#
# This is NOT the routine local-debugging entry point. Put target/debug on PATH
# instead (AGENTS.md): that directory is a complete set of binaries and needs no
# install step, so it cannot go stale between build and use.
#
# Why an installed copy is a poor debugging target: apps/teshi-cli/Cargo.toml
# declares a single `teshi` bin, so `cargo install --path` produces that one
# executable and no siblings. teshi resolves helper binaries relative to
# current_exe() (crates/teshi-tui/src/cli/terminal.rs for the terminal sidecar,
# crates/teshi-tui/src/runner.rs for the TUI runner), so an installed copy finds
# neither. Putting target/debug on PATH covers the workspace siblings but not the
# TUI runner: Cargo.toml excludes crates/teshi-tui-runner from the workspace, so
# it builds into its own target/debug and is reached only through
# TESHI_TUI_RUNNER or PATH.
# On Windows a symlink to target/debug does not help either, because
# current_exe() reports the link path there, leaving sibling lookup pointed at
# the link's own directory.
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

# Refresh the build identity before every install.
#
# crates/teshi-core/build.rs stamps TESHI_BUILD_LOCAL_TIMESTAMP from the clock,
# and it only declares `rerun-if-env-changed`. Without a `rerun-if-changed` it
# is not rerun when crate sources change, so cargo reuses the previously cached
# identity: an install that picks up new code still reports the older
# timestamp, and `--version` cannot distinguish the two. Touching the build
# script forces it to rerun and reread the clock, so the reported timestamp
# always reflects this install.
#
# Cost: teshi-core plus its dependents recompile, ~10 s on the dev profile.
touch crates/teshi-core/build.rs

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
version="$("$installed" --version)"
echo "$version"

# The build succeeded and a binary exists, which is not proof that the binary
# carries this checkout's identity. Compare the sha it reports against the
# source it was built from so a stale artifact fails loudly here instead of
# surfacing later as "the new build behaves like the old one".
head="$(git rev-parse --short HEAD 2>/dev/null || true)"
if [[ -n "$head" && "$version" != *"$head"* ]]; then
  echo "warning: installed binary reports '$version', which does not contain the current HEAD ($head)." >&2
  echo "warning: the artifact is stale or came from another checkout; rebuild before trusting it." >&2
fi
