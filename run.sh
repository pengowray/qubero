#!/usr/bin/env bash
# Start the dev server. Sets up whatever is missing first, so a fresh clone
# and an everyday run are the same command. The Windows twin is run.bat.
#
# The environment passes straight through to vite:
#   QUBERO_FILES=/mnt/d ./run.sh   serves that directory under /local/
#   PORT=17273 ./run.sh            puts this server on its own port

set -e
cd "$(dirname "$0")/web"

# wasm-pack and the wasm target, checked before the build that needs them so
# the failure names the one command that fixes it.
check_rust_toolchain() {
  if ! command -v wasm-pack >/dev/null 2>&1; then
    echo "wasm-pack is not installed. Run: cargo install wasm-pack" >&2
    exit 1
  fi
  if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed | grep -q wasm32-unknown-unknown; then
    echo "The wasm target is missing. Run: rustup target add wasm32-unknown-unknown" >&2
    exit 1
  fi
}

if [ ! -d node_modules ]; then
  echo "Installing dependencies..."
  npm install
fi

# The wasm build is half a minute and only core changes need it, so it runs
# when the package is missing rather than every time. After editing anything
# under crates/, run `npm run wasm` in web/ yourself.
if [ ! -f src/pkg/qubero_wasm.js ]; then
  check_rust_toolchain
  echo "Building wasm..."
  npm run wasm
fi

# The signatures that name which tool produced an executable. They are pinned
# to one commit and downloaded, so this runs once per clone.
if [ ! -f public/diesig/pe.sig ]; then
  echo "Downloading signatures..."
  node ../tools/die.mjs
fi

npm run dev
