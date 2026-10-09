#!/usr/bin/env bash
# Builds a self-contained Windows bundle of SoundTune (exe + GTK runtime).
#
# Run from the repository root inside an MSYS2 "UCRT64" shell:
#   bash packaging/windows/package.sh
# Output: dist/SoundTune/ and dist/SoundTune-windows-x64.zip
set -euo pipefail

PREFIX=/ucrt64
DIST=dist/SoundTune

if [[ "${SKIP_DEPS:-0}" != "1" ]]; then
    pacman -S --needed --noconfirm \
        mingw-w64-ucrt-x86_64-gtk4 \
        mingw-w64-ucrt-x86_64-rust \
        mingw-w64-ucrt-x86_64-pkgconf \
        mingw-w64-ucrt-x86_64-gcc \
        mingw-w64-ucrt-x86_64-librsvg \
        zip
fi

cargo build --release -p soundtune

rm -rf "$DIST"
mkdir -p "$DIST/bin" "$DIST/lib" "$DIST/share/glib-2.0" "$DIST/share/icons"

cp target/release/soundtune.exe "$DIST/bin/"

# Every UCRT64 DLL the exe (and the pixbuf loaders) depend on.
copy_deps() {
    ldd "$1" | awk '{print $3}' | grep -i "^$PREFIX/" | while read -r dll; do
        cp -n "$dll" "$DIST/bin/" 2>/dev/null || true
    done
}
copy_deps "$DIST/bin/soundtune.exe"

# Image loaders (needed for the SVG symbolic icons).
cp -r "$PREFIX/lib/gdk-pixbuf-2.0" "$DIST/lib/"
for loader in "$DIST"/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.dll; do
    copy_deps "$loader"
done
# The cache holds absolute MSYS2 paths; make them relative to bin/.
sed -i "s#\"[^\"]*lib/gdk-pixbuf-2.0#\"../lib/gdk-pixbuf-2.0#" \
    "$DIST/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"

# GSettings schemas and icon themes.
cp -r "$PREFIX/share/glib-2.0/schemas" "$DIST/share/glib-2.0/"
glib-compile-schemas "$DIST/share/glib-2.0/schemas"
cp -r "$PREFIX/share/icons/Adwaita" "$DIST/share/icons/"
cp -r "$PREFIX/share/icons/hicolor" "$DIST/share/icons/"

# Convenience launcher at the top of the bundle.
cat > "$DIST/SoundTune.bat" <<'BAT'
@echo off
start "" "%~dp0bin\soundtune.exe"
BAT

(cd dist && rm -f SoundTune-windows-x64.zip && zip -qr SoundTune-windows-x64.zip SoundTune)
echo "Bundle ready: dist/SoundTune-windows-x64.zip"
