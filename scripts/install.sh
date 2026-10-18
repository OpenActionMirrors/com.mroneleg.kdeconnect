#!/usr/bin/env bash
# Builds for this machine, packages, and copies the plugin into OpenDeck's
# plugins folder. Fully quit OpenDeck (tray icon -> Quit) and start it again
# afterwards. It must be a real copy: OpenDeck's file server returns 403 for
# files under a symlinked plugin, which breaks icons and the settings panel.
# Prints a single line so it can be piped.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo build --release --quiet
scripts/package.sh >/dev/null

DEST="${XDG_CONFIG_HOME:-$HOME/.config}/opendeck/plugins/com.mroneleg.kdeconnect.sdPlugin"
mkdir -p "$(dirname "$DEST")"
rm -rf "$DEST"
cp -r dist/com.mroneleg.kdeconnect.sdPlugin "$DEST"
echo "installed to $DEST; restart OpenDeck to load it"
