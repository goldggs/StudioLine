#!/usr/bin/env bash
# Developer setup. All toolchains, dependency caches and build outputs live outside the repository.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS="${STUDIOLINE_TOOLS_DIR:-/var/tmp/studioline-toolchain}"
export CARGO_HOME="$TOOLS/cargo"
export RUSTUP_HOME="$TOOLS/rustup"
export CARGO_TARGET_DIR="$TOOLS/target"
export npm_config_cache="$TOOLS/npm-cache"
mkdir -p "$TOOLS" "$CARGO_HOME" "$RUSTUP_HOME"
if [[ ! -x "$CARGO_HOME/bin/cargo" ]]; then
  command -v curl >/dev/null || { echo 'curl is required.' >&2; exit 1; }
  echo 'Installing the minimal Rust toolchain under the temporary tools directory...'
  curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o "$TOOLS/rustup-init.sh"
  sh "$TOOLS/rustup-init.sh" -y --no-modify-path --profile minimal --component rustfmt --component clippy
fi
export PATH="$CARGO_HOME/bin:$PATH"
# Pin the Luau CLI used for plugin syntax and mock tests. It stays outside the repository.
if [[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 && ! -x "$TOOLS/luau/luau-compile" ]]; then
  command -v unzip >/dev/null || { echo 'unzip is required for the Luau test tool.' >&2; exit 1; }
  command -v sha256sum >/dev/null || { echo 'sha256sum is required to verify the Luau test tool.' >&2; exit 1; }
  mkdir -p "$TOOLS/luau"
  curl --proto '=https' --tlsv1.2 -fsSL https://github.com/luau-lang/luau/releases/download/0.748/luau-ubuntu.zip -o "$TOOLS/luau/luau.zip"
  echo "e6905c04dbccaeff54ca4986b0017f125db5aa301bcb34d0446355cbfee5bc83  $TOOLS/luau/luau.zip" | sha256sum --check --status || { echo 'Luau download checksum failed.' >&2; exit 1; }
  unzip -oq "$TOOLS/luau/luau.zip" -d "$TOOLS/luau"
fi
command -v node >/dev/null || { echo 'Node.js is required to build the interface.' >&2; exit 1; }
command -v npm >/dev/null || { echo 'npm is required to build the interface.' >&2; exit 1; }
mkdir -p "$TOOLS/ui"
cp "$ROOT/ui/package.json" "$TOOLS/ui/package.json"
if [[ -f "$ROOT/ui/package-lock.json" ]]; then
  cp "$ROOT/ui/package-lock.json" "$TOOLS/ui/package-lock.json"
  (cd "$TOOLS/ui" && npm ci --no-audit --no-fund)
else
  (cd "$TOOLS/ui" && npm install --no-audit --no-fund)
fi
if [[ -e "$ROOT/ui/node_modules" && ! -L "$ROOT/ui/node_modules" ]]; then
  echo 'ui/node_modules exists and is not a symlink. Remove it or move it before continuing.' >&2
  exit 1
fi
ln -sfn "$TOOLS/ui/node_modules" "$ROOT/ui/node_modules"
printf '\nToolchain ready in %s\nRun: source scripts/toolchain-env.sh\n' "$TOOLS"
