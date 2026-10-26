#!/usr/bin/env bash
# Assembles the static site in dist/: page, wasm player, macroquad's JS loader
# and the perfect-play table (solved first if missing).
set -euo pipefail
cd "$(dirname "$0")/.."

[ -f tables/duat.table.gz ] || cargo run --release -- solve

cargo build --release --target wasm32-unknown-unknown --bin play --no-default-features --features gui

# The JS loader must match the macroquad version the player was built with.
macroquad_dir=$(cargo metadata --format-version 1 --features gui | python3 -c '
import json, os, sys
pkg = next(p for p in json.load(sys.stdin)["packages"] if p["name"] == "macroquad")
print(os.path.dirname(pkg["manifest_path"]))')

rm -rf dist
mkdir -p dist/tables
cp web/index.html dist/
cp target/wasm32-unknown-unknown/release/play.wasm dist/
cp "$macroquad_dir/js/mq_js_bundle.js" dist/
cp tables/duat.table.gz dist/tables/
ls -l dist dist/tables
