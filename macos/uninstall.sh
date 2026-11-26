#!/bin/bash
set -euo pipefail

LABEL="com.alexbeatsz.agy-plus.launcher"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"

launchctl bootout "gui/$UID" "$PLIST" >/dev/null 2>&1 || true
rm -f "$PLIST"
echo "uninstalled $LABEL"
