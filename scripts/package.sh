#!/usr/bin/env bash
# Assembles dist/com.mroneleg.kdeconnect.sdPlugin from assets/ plus whatever
# release binaries are present, and zips it (folder at the zip root) as
# dist/com.mroneleg.kdeconnect.streamDeckPlugin for OpenDeck's "Install from
# file". Binaries are taken from $BIN_DIR/kdeconnect-opendeck-<triple> (the
# release workflow downloads them there), target/<triple>/release/, or a plain
# host `cargo build --release`.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

ID=com.mroneleg.kdeconnect
NAME=kdeconnect-opendeck
TRIPLES=(x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu)
OUT="dist/$ID.sdPlugin"

rm -rf dist
mkdir -p "$OUT"
cp assets/manifest.json LICENSE "$OUT/"
cp -r assets/icons assets/pi "$OUT/"

found=0
for triple in "${TRIPLES[@]}"; do
	for candidate in "${BIN_DIR:-.}/$NAME-$triple" "target/$triple/release/$NAME"; do
		if [[ -f "$candidate" ]]; then
			install -m 755 "$candidate" "$OUT/$NAME-$triple"
			found=$((found + 1))
			echo "packaged $triple"
			break
		fi
	done
done
if [[ $found -eq 0 && -f "target/release/$NAME" ]]; then
	host=$(rustc -vV | sed -n 's/^host: //p')
	install -m 755 "target/release/$NAME" "$OUT/$NAME-$host"
	found=1
	echo "packaged host build ($host)"
fi
if [[ $found -eq 0 ]]; then
	echo "no binaries found; run cargo build --release first" >&2
	exit 1
fi

(cd dist && zip -qr "$ID.streamDeckPlugin" "$ID.sdPlugin")
echo "dist/$ID.streamDeckPlugin"
