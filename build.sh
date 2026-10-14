#!/usr/bin/env bash
# Compiles robocad in release mode and packages it.
#
#   macOS:   dist/robocad.app and dist/robocad-<version>-macos.dmg (needs macOS)
#   Windows: dist/robocad-<version>-windows-x86_64/robocad.exe (+ robocad-cli.exe) and a .zip,
#            cross-compiled with mingw-w64 (works from macOS and Linux)
#
# Usage: ./build.sh [all|mac|windows] [--universal]
#   all         both packages (default on macOS; on other systems the default is windows)
#   --universal macOS app for Apple Silicon and Intel
#
# Windows prerequisites: CMake >= 3.21 and mingw-w64
#   (macOS: brew install mingw-w64, Debian/Ubuntu: sudo apt install mingw-w64).
set -euo pipefail
cd "$(dirname "$0")"

usage() { awk 'NR > 1 && !/^#/ { exit } NR > 1 { sub(/^# ?/, ""); print }' "$0"; }
die() { echo "error: $*" >&2; exit 1; }

WHAT=""
UNIVERSAL=""
for arg in "$@"; do
  case "$arg" in
    all | mac | windows) WHAT=$arg ;;
    --universal) UNIVERSAL=--universal ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; die "unknown argument: $arg" ;;
  esac
done
if [[ -z "$WHAT" ]]; then
  if [[ "$(uname)" == Darwin ]]; then WHAT=all; else WHAT=windows; fi
fi

VERSION=$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)"/\1/')
WIN_TARGET=x86_64-pc-windows-gnu

build_mac() {
  [[ "$(uname)" == Darwin ]] || die "the macOS app can only be built on macOS"
  (
    # A broken Command Line Tools install ships a C++ header directory without <cstdint>,
    # which breaks the Manifold build; point the compiler at the SDK's headers instead.
    local clt
    clt=$(xcode-select -p 2>/dev/null || true)
    if [[ -z "${CXXFLAGS:-}" && -d "$clt/usr/include/c++/v1" && ! -e "$clt/usr/include/c++/v1/cstdint" ]]; then
      echo "note: working around stale C++ headers in $clt"
      export CXXFLAGS="-isystem $(xcrun --show-sdk-path)/usr/include/c++/v1"
    fi
    scripts/package-macos.sh $UNIVERSAL
  )
}

build_windows() {
  command -v x86_64-w64-mingw32-g++ >/dev/null ||
    die "mingw-w64 not found (macOS: brew install mingw-w64, Debian/Ubuntu: sudo apt install mingw-w64)"
  command -v cmake >/dev/null || die "cmake not found"
  # CMake reads CMAKE_TOOLCHAIN_FILE from the environment since 3.21; that is how the Manifold
  # C++ build (driven by manifold-csg-sys) is pointed at the mingw-w64 compilers.
  local cmake_version
  cmake_version=$(cmake --version | head -1 | sed -E 's/[^0-9]*([0-9]+\.[0-9]+).*/\1/')
  [[ "$(printf '%s\n' 3.21 "$cmake_version" | sort -V | head -1)" == 3.21 ]] ||
    die "CMake $cmake_version is too old for cross-compiling, 3.21 or newer is needed"
  rustup target list --installed | grep -qx "$WIN_TARGET" || rustup target add "$WIN_TARGET"

  # Link the C++ runtime statically so the .exe needs no MinGW DLLs. The Manifold build asks
  # for -lstdc++ explicitly, so a directory holding only the static archive goes first on the
  # library search path.
  local static_dir="$PWD/target/mingw-static"
  mkdir -p "$static_dir"
  ln -sf "$(x86_64-w64-mingw32-g++ -print-file-name=libstdc++.a)" "$static_dir/libstdc++.a"
  (
    # Host compiler flags (e.g. the macOS header workaround) must not leak into the cross build.
    unset CFLAGS CXXFLAGS
    export CMAKE_TOOLCHAIN_FILE="$PWD/packaging/mingw-w64-x86_64.cmake"
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C target-feature=+crt-static -L native=$static_dir"
    cargo build --release -p rc-app -p rc-cli --target "$WIN_TARGET"
  )

  local name="robocad-$VERSION-windows-x86_64"
  local stage="dist/$name"
  rm -rf "$stage" "dist/$name.zip"
  mkdir -p "$stage"
  cp "target/$WIN_TARGET/release/robocad.exe" "target/$WIN_TARGET/release/robocad-cli.exe" \
    README.md LICENSE-MIT LICENSE-APACHE "$stage/"
  if command -v zip >/dev/null; then
    (cd dist && zip -qr "$name.zip" "$name")
    echo "built $stage/robocad.exe and dist/$name.zip"
  else
    echo "built $stage/robocad.exe (zip not found, no archive created)"
  fi
}

case "$WHAT" in
  mac) build_mac ;;
  windows) build_windows ;;
  all) build_mac && build_windows ;;
esac
