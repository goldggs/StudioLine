#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/toolchain-env.sh"
if [[ ! -x "$CARGO_HOME/bin/cargo" || ! -x "$ROOT/ui/node_modules/.bin/vite" ]]; then
  echo 'Run ./scripts/setup-toolchain.sh first.' >&2; exit 1
fi
(cd "$ROOT/ui" && npm run build)
(cd "$ROOT/app" && cargo build "$@")
