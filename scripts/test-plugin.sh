#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS="${STUDIOLINE_TOOLS_DIR:-/var/tmp/studioline-toolchain}"
LUAU="$TOOLS/luau"
if [[ ! -x "$LUAU/luau" || ! -x "$LUAU/luau-compile" ]]; then
  echo 'Luau CLI is needed for this optional test. Install it under the temporary tools directory.' >&2
  exit 1
fi
mkdir -p "$TOOLS/plugin-tests"
"$LUAU/luau-compile" "$ROOT/plugin/src/StudioLine.server.luau" > /dev/null
cat "$ROOT/plugin/tests/mock_environment.luau" "$ROOT/plugin/src/StudioLine.server.luau" "$ROOT/plugin/tests/mock_assertions.luau" > "$TOOLS/plugin-tests/combined.luau"
"$LUAU/luau" "$TOOLS/plugin-tests/combined.luau"
