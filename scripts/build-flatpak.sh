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
# 写进 bundle 的 runtime-repo：安装 .flatpak 时会用它在缺失时补运行时及其扩展，
# 例如 codecs-extra（H.264 / HEVC / E-AC-3）。
RUNTIME_REPO="${KICHI_FLATPAK_RUNTIME_REPO:-https://dl.flathub.org/repo/flathub.flatpakrepo}"

# 额外参数（例如 CI 里 /dev/fuse 不可用时传 --disable-rofiles-fuse）。
BUILDER_ARGS=()
if [[ -n "${KICHI_FLATPAK_BUILDER_ARGS:-}" ]]; then
    read -r -a BUILDER_ARGS <<< "$KICHI_FLATPAK_BUILDER_ARGS"
fi

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

flatpak-builder "${BUILDER_ARGS[@]}" --force-clean --repo="$REPO_DIR" "$BUILD_DIR" "$MANIFEST"
flatpak build-bundle --runtime-repo="$RUNTIME_REPO" "$REPO_DIR" "$BUNDLE" "$APP_ID" master
sha256sum "$BUNDLE" > "$BUNDLE.sha256"

echo
echo "产物：$BUNDLE"
echo "安装：flatpak install --user \"$BUNDLE\""
echo
# bundle 已带 runtime-repo，安装时会自动拉取运行时的 codecs-extra 扩展。
# 离线安装、或安装机没有 flathub 远程时，仍可手动装一次。
if ! flatpak info org.freedesktop.Platform.codecs-extra//25.08-extra >/dev/null 2>&1; then
    echo "提示：当前机器还没装编解码扩展（H.264 / HEVC / E-AC-3）："
    echo "  flatpak install flathub org.freedesktop.Platform.codecs-extra//25.08-extra"
fi
