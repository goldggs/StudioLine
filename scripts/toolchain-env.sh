#!/usr/bin/env bash
# Source this script to use the temporary developer toolchain.
_STUDIOLINE_TOOLS="${STUDIOLINE_TOOLS_DIR:-/var/tmp/studioline-toolchain}"
export CARGO_HOME="$_STUDIOLINE_TOOLS/cargo"
export RUSTUP_HOME="$_STUDIOLINE_TOOLS/rustup"
export CARGO_TARGET_DIR="$_STUDIOLINE_TOOLS/target"
export npm_config_cache="$_STUDIOLINE_TOOLS/npm-cache"
export PATH="$CARGO_HOME/bin:$PATH"
unset _STUDIOLINE_TOOLS
