#!/usr/bin/env bash
# Install the PikPak Linux icon and desktop entry for the current user.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ICON_SRC="$ROOT/assets/pikpak-linux.svg"
DESKTOP_SRC="$ROOT/packaging/pikpak-linux.desktop"

DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
ICON_NAME="pikpak-linux"
APPS_DIR="$DATA_HOME/applications"
HICOLOR="$DATA_HOME/icons/hicolor"

SIZES=(16 24 32 48 64 128 256 512)

if [[ ! -f "$ICON_SRC" ]]; then
    echo "icon not found: $ICON_SRC" >&2
    exit 1
fi

install -Dm644 "$DESKTOP_SRC" "$APPS_DIR/$ICON_NAME.desktop"
install -Dm644 "$ICON_SRC" "$HICOLOR/scalable/apps/$ICON_NAME.svg"

for size in "${SIZES[@]}"; do
    out="$HICOLOR/${size}x${size}/apps/$ICON_NAME.png"
    mkdir -p "$(dirname "$out")"
    if command -v rsvg-convert >/dev/null 2>&1; then
        rsvg-convert -w "$size" -h "$size" "$ICON_SRC" -o "$out"
    elif command -v ksvgtopng >/dev/null 2>&1; then
        ksvgtopng "$size" "$size" "$ICON_SRC" "$out"
    elif command -v magick >/dev/null 2>&1; then
        magick -background none -resize "${size}x${size}" "$ICON_SRC" "$out"
    else
        echo "warning: no SVG rasterizer found, skipping ${size}px" >&2
        continue
    fi
done

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$HICOLOR" >/dev/null 2>&1 || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
fi
if command -v kbuildsycoca6 >/dev/null 2>&1; then
    kbuildsycoca6 >/dev/null 2>&1 || true
elif command -v kbuildsycoca5 >/dev/null 2>&1; then
    kbuildsycoca5 >/dev/null 2>&1 || true
fi

echo "installed:"
echo "  icon:    $HICOLOR/scalable/apps/$ICON_NAME.svg (+ PNG sizes)"
echo "  desktop: $APPS_DIR/$ICON_NAME.desktop"
echo "note: make sure 'pikpak-gui' is on \$PATH (or edit Exec= in the desktop file)."
