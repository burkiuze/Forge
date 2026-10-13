#!/usr/bin/env sh
# Forge installer for Linux / macOS: copies the binary, icon and a desktop entry.
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="${HERE}/forge"
[ -f "$BIN" ] || BIN="${HERE}/../target/release/forge"
PREFIX="${PREFIX:-$HOME/.local}"
mkdir -p "$PREFIX/bin"
install -m 755 "$BIN" "$PREFIX/bin/forge"
if [ "$(uname)" = "Linux" ]; then
  mkdir -p "$PREFIX/share/applications" "$PREFIX/share/icons/hicolor/256x256/apps"
  [ -f "$HERE/../assets/logo.png" ] && cp "$HERE/../assets/logo.png" "$PREFIX/share/icons/hicolor/256x256/apps/forge.png"
  cat > "$PREFIX/share/applications/forge.desktop" <<DESK
[Desktop Entry]
Name=Forge
Comment=AI-native game engine
Exec=$PREFIX/bin/forge
Icon=forge
Terminal=false
Type=Application
Categories=Development;Game;
DESK
fi
echo "Forge installed to $PREFIX/bin/forge — run 'forge' (the setup wizard opens on first launch)."
