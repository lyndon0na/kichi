#!/usr/bin/env bash
# 构建 Flatpak 发行包: 安装运行时 -> flatpak-builder 沙箱内源码构建 -> 打包成单文件 bundle。
#
# 产物: dist/Kichi-<版本>-x86_64.flatpak（--install 时同时装进用户级 flatpak）
# 版本号取 KICHI_VERSION(CI 用 tag), 未设置时读 Cargo.toml 的 [workspace.package]。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
MANIFEST="$ROOT/packaging/flatpak/io.github.lyndon0na.Kichi.yml"
APP_ID="io.github.lyndon0na.Kichi"
RUNTIME_VERSION="25.08"
RUNTIMES=(
    "org.freedesktop.Platform//$RUNTIME_VERSION"
    "org.freedesktop.Sdk//$RUNTIME_VERSION"
    "org.freedesktop.Sdk.Extension.rust-stable//$RUNTIME_VERSION"
)

INSTALL=0
case "${1:-}" in
    "") ;;
    --install) INSTALL=1 ;;
    *)
        echo "用法: $(basename "$0") [--install]   # --install 额外装入用户级 flatpak" >&2
        exit 2
        ;;
esac

version() {
    if [[ -n "${KICHI_VERSION:-}" ]]; then
        printf '%s\n' "${KICHI_VERSION#v}"
        return
    fi
    sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1
}

VERSION="$(version)"
[[ -n "$VERSION" ]] || { echo "error: 无法确定版本号" >&2; exit 1; }
OUT="$DIST/Kichi-$VERSION-x86_64.flatpak"

echo "==> 准备 flathub 远端与运行时"
flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install --user --noninteractive -y flathub "${RUNTIMES[@]}"

echo "==> flatpak-builder 构建"
mkdir -p "$DIST"
flatpak-builder --user --force-clean --repo="$DIST/flatpak-repo" "$DIST/flatpak-build" "$MANIFEST"

echo "==> 生成 $OUT"
flatpak build-bundle "$DIST/flatpak-repo" "$OUT" "$APP_ID"

if [[ $INSTALL == 1 ]]; then
    echo "==> 安装到用户级 flatpak"
    flatpak install --user --noninteractive -y "$OUT"
fi

echo "完成: $OUT"
