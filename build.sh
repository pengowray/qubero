#!/usr/bin/env bash
# Build the site into web/dist, the same steps the deploy workflow runs.
# The Windows twin is build.bat.
#
# Unlike run.sh this rebuilds the wasm every time: a release build should not
# ship whatever the last dev session happened to leave behind.

set -e
cd "$(dirname "$0")/web"

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "wasm-pack is not installed. Run: cargo install wasm-pack" >&2
  exit 1
fi
if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed | grep -q wasm32-unknown-unknown; then
  echo "The wasm target is missing. Run: rustup target add wasm32-unknown-unknown" >&2
  exit 1
fi

echo "Installing dependencies..."
npm install

echo "Building wasm..."
npm run wasm

# Pinned to one commit, so an existing download is the right one.
if [ ! -f public/diesig/pe.sig ]; then
  echo "Downloading signatures..."
  node ../tools/die.mjs
fi

echo "Building site..."
npm run build

echo
echo "Built web/dist. To serve it: cd web && npm run preview"
