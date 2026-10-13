#!/usr/bin/env bash
# Builds robocad.app and a DMG in dist/. Usage: scripts/package-macos.sh [--universal]
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)"/\1/')
DIST=dist
APP="$DIST/robocad.app"

if [[ "${1:-}" == "--universal" ]]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
  cargo build --release -p rc-app -p rc-cli --target aarch64-apple-darwin
  cargo build --release -p rc-app -p rc-cli --target x86_64-apple-darwin
  BIN_DIR="$DIST/universal"
  mkdir -p "$BIN_DIR"
  for b in robocad robocad-cli; do
    lipo -create -output "$BIN_DIR/$b" \
      "target/aarch64-apple-darwin/release/$b" "target/x86_64-apple-darwin/release/$b"
  done
else
  cargo build --release -p rc-app -p rc-cli
  BIN_DIR=target/release
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN_DIR/robocad" "$APP/Contents/MacOS/robocad"
cp "$BIN_DIR/robocad-cli" "$APP/Contents/MacOS/robocad-cli"
sed "s/@VERSION@/$VERSION/g" packaging/Info.plist.in > "$APP/Contents/Info.plist"

# Icon set from the 1024 px master.
ICONSET="$DIST/robocad.iconset"
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon/robocad.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z $d $d assets/icon/robocad.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/robocad.icns"
rm -rf "$ICONSET"

# Ad-hoc signature so Gatekeeper on Apple Silicon accepts the local build.
# For distribution, sign with a Developer ID and notarize (see docs/release.md).
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || echo "warning: codesign failed"

DMG="$DIST/robocad-$VERSION-macos.dmg"
rm -f "$DMG"
STAGE="$DIST/dmg"
rm -rf "$STAGE" && mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "robocad $VERSION" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"
echo "built $APP and $DMG"
