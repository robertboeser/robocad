#!/usr/bin/env bash
# Builds a tarball with the binaries, a .desktop file and the icon in dist/.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)"/\1/')
cargo build --release -p rc-app -p rc-cli
NAME="robocad-$VERSION-linux-$(uname -m)"
STAGE="dist/$NAME"
rm -rf "$STAGE" && mkdir -p "$STAGE"
cp target/release/robocad target/release/robocad-cli "$STAGE/"
cp assets/icon/robocad.png "$STAGE/robocad.png"
cat > "$STAGE/robocad.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=robocad
Comment=Simple construction tool for 3D-printable designs
Exec=robocad %F
Icon=robocad
Categories=Graphics;3DGraphics;Engineering;
MimeType=model/stl;application/x-robocad;
DESKTOP
cp README.md LICENSE-MIT LICENSE-APACHE "$STAGE/"
tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
echo "built dist/$NAME.tar.gz"
