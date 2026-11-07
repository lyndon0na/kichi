#!/usr/bin/env bash
# 构建 kichi 的 Flatpak 包。
#
# 前置条件：
# - flatpak-builder、org.kde.Sdk//6.11、org.kde.Platform//6.11
# - org.freedesktop.Sdk.Extension.rust-stable//25.08
#
# 产物：
# - dist/io.github.lyndon0na.kichi-<日期>.flatpak
# - dist/io.github.lyndon0na.kichi-<日期>.flatpak.sha256

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

APP_ID=io.github.lyndon0na.kichi
MANIFEST=packaging/flatpak/$APP_ID.yml
BUILD_DIR="$ROOT/build-dir"
REPO_DIR="$ROOT/dist/flatpak-repo"
DIST_DIR="$ROOT/dist"
VERSION="${KICHI_FLATPAK_VERSION:-$(date +%Y%m%d)}"
VERSION="${VERSION#v}"
VERSION="${VERSION//\//-}"
VERSION="${VERSION// /-}"
BUNDLE="$DIST_DIR/$APP_ID-$VERSION.flatpak"

for tool in flatpak flatpak-builder; do
    if ! command -v "$tool" >/dev/null; then
        echo "build-flatpak: 缺少 $tool" >&2
        exit 1
    fi
done

if ! flatpak info org.kde.Sdk//6.11 >/dev/null 2>&1; then
    echo "build-flatpak: 缺少 org.kde.Sdk//6.11" >&2
    exit 1
fi

mkdir -p "$DIST_DIR"

flatpak-builder --force-clean --repo="$REPO_DIR" "$BUILD_DIR" "$MANIFEST"
flatpak build-bundle "$REPO_DIR" "$BUNDLE" "$APP_ID" master
sha256sum "$BUNDLE" > "$BUNDLE.sha256"

echo
echo "产物：$BUNDLE"
echo "安装：flatpak install --user \"$BUNDLE\""
