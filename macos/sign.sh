#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:-$ROOT/target/release/agy-plus}"
IDENTITY="${AGY_PLUS_SIGN_IDENTITY:-Local Development Code Signing}"
IDENTIFIER="com.alexbeatsz.agy-plus"

if [[ ! -x "$BIN" ]]; then
  echo "missing executable: $BIN" >&2
  echo "run bash macos/release.sh or cargo build --release first" >&2
  exit 1
fi

identity_line="$(security find-identity -v -p codesigning | grep -F -- "\"$IDENTITY\"" | head -1 || true)"
if [[ -z "$identity_line" ]]; then
  echo "code signing identity not found or not trusted: $IDENTITY" >&2
  exit 1
fi
identity_hash="$(printf '%s\n' "$identity_line" | awk '{print tolower($2)}')"

requirement="$(codesign -d -r- "$BIN" 2>&1 || true)"
if [[ "$requirement" == *"identifier \"$IDENTIFIER\""* &&
      "$requirement" == *"certificate leaf = H\"$identity_hash\""* ]] &&
   codesign --verify --strict "$BIN" >/dev/null 2>&1; then
  echo "already signed: $BIN"
  exit 0
fi

STAGE="$BIN.signed.$$"
trap 'rm -f "$STAGE"' EXIT

cp "$BIN" "$STAGE"
chmod +x "$STAGE"
codesign --force --timestamp=none --identifier "$IDENTIFIER" --sign "$IDENTITY" "$STAGE"
codesign --verify --strict "$STAGE"
mv -f "$STAGE" "$BIN"
trap - EXIT

echo "signed $BIN"
codesign -d -r- "$BIN" 2>&1 | grep -F 'designated =>' | sed 's/^#* */  /'
