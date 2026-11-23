#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/toolchain-env.sh"
(cd "$ROOT/ui" && npm run build && npm test)
(cd "$ROOT/app" && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test)

if [[ -x "$CARGO_HOME/../luau/luau" ]]; then
  "$ROOT/scripts/test-plugin.sh"
else
  echo 'Luau CLI is not installed in the temporary tools directory; plugin mock tests were not run.' >&2
fi
