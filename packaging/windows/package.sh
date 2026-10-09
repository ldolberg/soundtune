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
# MSYS2 ldd prints "name.dll => /ucrt64/bin/name.dll (0x...)"; keep only the
# DLLs from the UCRT64 prefix (system DLLs live under /c/Windows). awk is used
# for the filtering because, unlike grep, it does not exit non-zero when a
# binary has no UCRT64 dependencies, which would abort under pipefail.
copy_deps() {
    local dll
    ldd "$1" | awk -v prefix="$PREFIX/" \
        'index(tolower($3), prefix) == 1 { print $3 }' |
        while read -r dll; do
            [[ -e "$DIST/bin/$(basename "$dll")" ]] || cp "$dll" "$DIST/bin/"
        done
}
copy_deps "$DIST/bin/soundtune.exe"

# Image loaders (needed for the SVG symbolic icons).
PIXBUF_DIR=lib/gdk-pixbuf-2.0/2.10.0
cp -r "$PREFIX/lib/gdk-pixbuf-2.0" "$DIST/lib/"
shopt -s nullglob
for loader in "$DIST/$PIXBUF_DIR"/loaders/*.dll; do
    copy_deps "$loader"
done
shopt -u nullglob
# Keep only what the runtime needs (drops static import libs, if any).
find "$DIST/lib/gdk-pixbuf-2.0" -name '*.a' -delete
# On Windows gdk-pixbuf resolves relative module paths in loaders.cache
# against the installation root (the parent of bin/), so rewrite every entry,
# whether absolute ("C:/msys64/ucrt64/lib/...", "/ucrt64/lib/...") or already
# relative, to "lib/gdk-pixbuf-2.0/...".
sed -i -E 's#"[^"]*lib(/|\\\\)gdk-pixbuf-2\.0#"lib/gdk-pixbuf-2.0#' \
    "$DIST/$PIXBUF_DIR/loaders.cache"

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
