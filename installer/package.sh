#!/usr/bin/env bash
# Builds Modeloid's release files for one platform into dist/.
#
#   bash installer/package.sh windows-x64   (Git Bash; also needs Inno Setup 6)
#   bash installer/package.sh linux-x64     (needs rsvg-convert for the icon)
#   bash installer/package.sh macos-arm64   (needs rsvg-convert, iconutil, ditto)
#
# Output (VERSION from modeloid-gui/Cargo.toml unless set):
#   windows: Modeloid-<v>-windows-x64.zip, Modeloid-<v>-setup.exe
#   linux:   Modeloid-<v>-linux-x64.tar.gz
#   macos:   Modeloid-<v>-macos-arm64.zip (Modeloid.app, unsigned)
#
# The package is the server binary + public/ (browser app) + pyservice/ (the
# Python project the app sets up on first launch). The server finds both next
# to its own exe, so the folder can live anywhere.
set -euo pipefail

PLATFORM="${1:?usage: package.sh windows-x64|linux-x64|macos-arm64}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="${VERSION:-$(sed -n 's/^version *= *"\(.*\)"/\1/p' modeloid-gui/Cargo.toml | head -n1)}"
NAME="Modeloid-$VERSION-$PLATFORM"
DIST="$ROOT/dist"
PKG="$DIST/$NAME"
BUILD="target/dx/modeloid-gui/release/web"

echo "==> Modeloid $VERSION for $PLATFORM"

# dx bundle never removes old hashed files from public/, so start clean.
rm -rf target/dx/modeloid-gui/release
dx bundle --release --platform web -p modeloid-gui

case "$PLATFORM" in
  windows-*) EXE="server.exe"; OUT_EXE="Modeloid.exe" ;;
  *)         EXE="server";     OUT_EXE="modeloid" ;;
esac
[ -f "$BUILD/$EXE" ] || { echo "error: $BUILD/$EXE not found"; exit 1; }

# The app folder, the same on every system.
rm -rf "$PKG"
mkdir -p "$PKG/pyservice"
cp "$BUILD/$EXE" "$PKG/$OUT_EXE"
cp -R "$BUILD/public" "$PKG/public"
cp modeloid/src/pyservice/{main.py,pyproject.toml,uv.lock} "$PKG/pyservice/"
chmod +x "$PKG/$OUT_EXE" 2>/dev/null || true

ICON_SVG="modeloid-gui/assets/modeloid-ico.svg"

case "$PLATFORM" in
  windows-*)
    rm -f "$DIST/$NAME.zip"
    (cd "$DIST" && powershell.exe -NoProfile -Command "Compress-Archive -Path '$NAME' -DestinationPath '$NAME.zip'")
    ISCC="${ISCC:-}"
    for candidate in "$ISCC" \
        "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" \
        "${LOCALAPPDATA:-}/Programs/Inno Setup 6/ISCC.exe"; do
      if [ -n "$candidate" ] && [ -f "$candidate" ]; then ISCC="$candidate"; break; fi
    done
    [ -f "$ISCC" ] || { echo "error: Inno Setup 6 (ISCC.exe) not found; set ISCC=<path>"; exit 1; }
    # "-D", not "/D": Git Bash would turn "/D…" into a path
    "$ISCC" "-DAppVersion=$VERSION" installer/windows/modeloid.iss
    ;;

  linux-*)
    cp installer/linux/modeloid.desktop "$PKG/"
    cp installer/linux/install.sh "$PKG/"
    chmod +x "$PKG/install.sh"
    rsvg-convert -w 256 -h 256 "$ICON_SVG" -o "$PKG/modeloid.png"
    tar -C "$DIST" -czf "$DIST/$NAME.tar.gz" "$NAME"
    ;;

  macos-*)
    APP="$DIST/Modeloid.app"
    rm -rf "$APP"
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
    # Binary, public/ and pyservice/ stay together in MacOS/ (found next to the exe).
    mv "$PKG/$OUT_EXE" "$PKG/public" "$PKG/pyservice" "$APP/Contents/MacOS/"
    rmdir "$PKG"
    cp installer/macos/launch.sh "$APP/Contents/MacOS/Modeloid"
    chmod +x "$APP/Contents/MacOS/Modeloid"
    sed "s/@VERSION@/$VERSION/g" installer/macos/Info.plist > "$APP/Contents/Info.plist"

    ICONSET="$DIST/Modeloid.iconset"
    rm -rf "$ICONSET" && mkdir -p "$ICONSET"
    for size in 16 32 128 256 512; do
      rsvg-convert -w $size -h $size "$ICON_SVG" -o "$ICONSET/icon_${size}x${size}.png"
      rsvg-convert -w $((size * 2)) -h $((size * 2)) "$ICON_SVG" -o "$ICONSET/icon_${size}x${size}@2x.png"
    done
    iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Modeloid.icns"
    rm -rf "$ICONSET"

    rm -f "$DIST/$NAME.zip"
    ditto -c -k --keepParent "$APP" "$DIST/$NAME.zip"
    ;;

  *) echo "error: unknown platform $PLATFORM"; exit 1 ;;
esac

echo "==> Done:"
ls -1 "$DIST" | grep -F "$VERSION" || true
