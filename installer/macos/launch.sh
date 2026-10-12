#!/bin/sh
# Modeloid.app's entry point. The server runs in a Terminal window, like the
# console window on Windows: the address is printed there and closing the
# window stops Modeloid.
DIR="$(cd "$(dirname "$0")" && pwd)"
exec open -a Terminal "$DIR/modeloid"
