#!/bin/sh
# Adds Modeloid to the application menu for this user (no root needed).
# Run from the unpacked folder: ./install.sh   Remove with: ./install.sh --remove
# The app runs in a terminal window; closing it stops Modeloid.
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICONS="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"

if [ "$1" = "--remove" ]; then
  rm -f "$APPS/modeloid.desktop" "$ICONS/modeloid.png"
  echo "Removed Modeloid from the application menu."
  exit 0
fi

mkdir -p "$APPS" "$ICONS"
cp "$DIR/modeloid.png" "$ICONS/modeloid.png"
sed "s|^Exec=.*|Exec=\"$DIR/modeloid\"|" "$DIR/modeloid.desktop" > "$APPS/modeloid.desktop"
echo "Added Modeloid to the application menu (runs $DIR/modeloid)."
