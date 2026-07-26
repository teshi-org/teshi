#!/usr/bin/env bash
# Run teshi web UI smoke scenarios (embedded browser replay).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TESHI="${TESHI_BIN:-cargo run --quiet --}"

echo "==> build frontend"
(cd apps/teshi-web-ui && npm ci && npm run build)

echo "==> ensure python venv"
if [[ ! -d .venv ]]; then
  python3 -m venv .venv
fi
# shellcheck disable=SC1091
source .venv/bin/activate
pip install -q -r python/requirements.txt
python -m playwright install chromium

echo "==> build teshi CLI and NDJSON runner"
cargo build --quiet
cargo build --manifest-path runner/Cargo.toml --quiet

TESHI_EXE="$ROOT/target/debug/teshi"
RUNNER_EXE="$ROOT/runner/target/debug/runner"
export TESHI_BIN="$TESHI_EXE"
export TESHI_RUNNER_CMD="$RUNNER_EXE"

echo "==> start teshi web"
"$TESHI_EXE" web --port 1421 --no-open &
WEB_PID=$!
cleanup() {
  kill "$WEB_PID" 2>/dev/null || true
  kill "$SIDECAR_PID" 2>/dev/null || true
}
trap cleanup EXIT

for _ in $(seq 1 30); do
  if curl -sf "http://127.0.0.1:1421/" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

echo "==> start embedded sidecar"
"$TESHI_EXE" browser serve-embedded --navigate "http://127.0.0.1:1421" &
SIDECAR_PID=$!
sleep 3

echo "==> run web-ui smoke"
"$TESHI_EXE" run tests/feature/web-ui/welcome_smoke.feature

echo "==> web-ui smoke passed"
