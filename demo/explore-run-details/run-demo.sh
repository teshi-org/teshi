#!/usr/bin/env bash
set -euo pipefail

demo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd "$demo_dir/../.." && pwd)"
binary="$repo_dir/target/debug/teshi"

if [[ ! -x "$binary" ]]; then
  (cd "$repo_dir" && cargo build -p teshi-cli)
fi

cd "$demo_dir"
exec "$binary" .
