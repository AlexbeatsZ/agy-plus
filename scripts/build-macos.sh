#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
security find-identity -v -p codesigning | rg -q '"Local Development Code Signing"'
npm run tauri -- build --bundles app
app="$PWD/target/release/bundle/macos/AGY Plus.app"
codesign --verify --deep --strict "$app"
codesign -d -r- "$app" 2>&1 | tee "$PWD/target/signing-requirement.txt"
rg -q 'certificate leaf' "$PWD/target/signing-requirement.txt"
