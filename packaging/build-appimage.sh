#!/usr/bin/env bash
# 构建 AppImage 发行包: cargo 构建 -> 组装 AppDir -> appimagetool 出包。
#
# 产物: dist/Kichi-<版本>-x86_64.AppImage
# 版本号取 KICHI_VERSION(CI 用 tag), 未设置时读 Cargo.toml 的 [workspace.package]。
#
# 注意: AppImage 把宿主 glibc 当作最低门槛, 在 Fedora 44(glibc 2.43)构建的产物
# 只能在同级别或更新的发行版运行; 发布的包由 CI 在 ubuntu-22.04(glibc 2.35)上构建。
set -euo pipefail

if [[ $# -gt 0 ]]; then
    echo "用法: $(basename "$0")   # 无参数; 版本号可用 KICHI_VERSION 覆盖" >&2
    exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
APPDIR="$DIST/Kichi.AppDir"
TOOLS_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/kichi-packaging"

APPIMAGETOOL_VERSION=1.9.1
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VERSION}/appimagetool-x86_64.AppImage"
APPIMAGETOOL_SHA256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"

version() {
    if [[ -n "${KICHI_VERSION:-}" ]]; then
        printf '%s\n' "${KICHI_VERSION#v}"
        return
    fi
    sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1
}

rasterize() {
    local size="$1" out="$2"
    rm -f "$out"
    if command -v rsvg-convert >/dev/null 2>&1; then
        rsvg-convert -w "$size" -h "$size" "$ICON_SVG" -o "$out" >/dev/null 2>&1 || true
    fi
    if [[ ! -s "$out" ]] && command -v ksvgtopng >/dev/null 2>&1; then
        ksvgtopng "$size" "$size" "$ICON_SVG" "$out" >/dev/null 2>&1 || true
    fi
    if [[ ! -s "$out" ]] && command -v magick >/dev/null 2>&1; then
        magick -background none "$ICON_SVG" -resize "${size}x${size}" "$out" >/dev/null 2>&1 || true
    fi
    if [[ ! -s "$out" ]] && command -v inkscape >/dev/null 2>&1; then
        inkscape "$ICON_SVG" -w "$size" -h "$size" -o "$out" >/dev/null 2>&1 || true
    fi
    if [[ ! -s "$out" ]]; then
        echo "error: 需要一款 SVG 光栅化工具(rsvg-convert / ksvgtopng / magick / inkscape)" >&2
        echo "       Fedora: sudo dnf install librsvg2-tools" >&2
        echo "       Debian/Ubuntu: sudo apt-get install librsvg2-bin" >&2
        exit 1
    fi
}

appimagetool() {
    local tool="${APPIMAGETOOL:-$TOOLS_DIR/appimagetool-$APPIMAGETOOL_VERSION.AppImage}"
    if [[ ! -x "$tool" ]]; then
        echo "==> 下载 appimagetool $APPIMAGETOOL_VERSION"
        mkdir -p "$TOOLS_DIR"
        curl -fsSL --retry 3 -o "$tool.part" "$APPIMAGETOOL_URL"
        mv "$tool.part" "$tool"
        chmod +x "$tool"
    fi
    local got
    got="$(sha256sum "$tool" | cut -d' ' -f1)"
    if [[ "$got" != "$APPIMAGETOOL_SHA256" ]]; then
        echo "error: appimagetool 校验和不匹配" >&2
        echo "      期望 $APPIMAGETOOL_SHA256" >&2
        echo "      实际 $got" >&2
        exit 1
    fi
    # 容器 / 无 FUSE 环境下同样可用。
    "$tool" --appimage-extract-and-run "$@"
}

VERSION="$(version)"
[[ -n "$VERSION" ]] || { echo "error: 无法确定版本号" >&2; exit 1; }
OUT="$DIST/Kichi-$VERSION-x86_64.AppImage"
ICON_SVG="$ROOT/assets/kichi.svg"

echo "==> 构建 release 二进制"
cargo build --release --locked -p kichi-gui --manifest-path "$ROOT/Cargo.toml"

echo "==> 组装 AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
    "$APPDIR/usr/share/icons/hicolor/scalable/apps"
install -Dm755 "$ROOT/target/release/kichi-gui" "$APPDIR/usr/bin/kichi-gui"
install -Dm644 "$ROOT/packaging/kichi.desktop" "$APPDIR/usr/share/applications/kichi.desktop"
# appimagetool 1.9 只在 AppDir 根目录(且必须是普通文件)找 *.desktop。
install -Dm644 "$ROOT/packaging/kichi.desktop" "$APPDIR/kichi.desktop"
install -Dm755 "$ROOT/packaging/appimage/AppRun" "$APPDIR/AppRun"
install -Dm644 "$ICON_SVG" "$APPDIR/usr/share/icons/hicolor/scalable/apps/kichi.svg"
for size in 256 512; do
    mkdir -p "$APPDIR/usr/share/icons/hicolor/${size}x${size}/apps"
    rasterize "$size" "$APPDIR/usr/share/icons/hicolor/${size}x${size}/apps/kichi.png"
done
# AppImage 约定: 根目录要有一份与 desktop 的 Icon= 同名的图标。
install -Dm644 "$APPDIR/usr/share/icons/hicolor/256x256/apps/kichi.png" "$APPDIR/kichi.png"
install -Dm644 "$APPDIR/usr/share/icons/hicolor/256x256/apps/kichi.png" "$APPDIR/.DirIcon"

echo "==> 生成 $OUT"
mkdir -p "$DIST"
ARCH=x86_64 appimagetool "$APPDIR" "$OUT"
echo "完成: $OUT"
