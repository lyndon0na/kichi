#!/usr/bin/env bash
# 构建 AppImage（产物面向当前构建机）。
#
# 设计边界：
# - Qt / Kirigami / qqc2-desktop-style 和它们的运行库进包；
# - 内嵌播放保留，但 libmpv / FFmpeg 由宿主提供（需安装 mpv-libs / mpv）；
# - KWallet、XDG Portal、Plasma 主题、Breeze 图标继续用宿主；
# - 工具固定版本并校验 SHA-256，缓存到 target/appimage-tools；
# - 产物只面向当前构建机（Fedora 的 glibc 下限），不作为跨发行版承诺。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ARCH="${ARCH:-x86_64}"
if [[ "$(uname -m)" != "$ARCH" ]]; then
    echo "build-appimage: 当前脚本只准备了 x86_64 工具，当前架构是 $(uname -m)" >&2
    exit 1
fi

VERSION="${KICHI_APPIMAGE_VERSION:-$(date +%Y%m%d)}"
VERSION="${VERSION#v}"
VERSION="${VERSION//\//-}"
VERSION="${VERSION// /-}"

TOOLS_DIR="${KICHI_APPIMAGE_TOOLS_DIR:-$ROOT/target/appimage-tools}"
BUILD_DIR="${KICHI_APPIMAGE_BUILD_DIR:-$ROOT/target/appimage-build}"
DIST_DIR="${KICHI_APPIMAGE_DIST_DIR:-$ROOT/dist}"
APPDIR="$BUILD_DIR/AppDir"
APPIMAGE_NAME="kichi-${VERSION}-${ARCH}.AppImage"

case "$BUILD_DIR" in
    "$ROOT"/target/*) ;;
    *)
        echo "build-appimage: 拒绝删除不在 target/ 下的构建目录：$BUILD_DIR" >&2
        exit 1
        ;;
esac

LINUXDEPLOY="$TOOLS_DIR/linuxdeploy-x86_64.AppImage"
QT_PLUGIN="$TOOLS_DIR/linuxdeploy-plugin-qt-x86_64.AppImage"
APPIMAGETOOL="$TOOLS_DIR/appimagetool-x86_64.AppImage"

# linuxdeploy 内置的 patchelf 0.15 不认识 Fedora 44 默认生成的 DT_RELR
# 重定位，会把打包后的动态库写坏；必须使用 0.18 或更新的版本。
PATCHELF="${KICHI_PATCHELF:-$(command -v patchelf || true)}"
if [[ -z "$PATCHELF" && -x "$TOOLS_DIR/patchelf" ]]; then
    PATCHELF="$TOOLS_DIR/patchelf"
fi
[[ -n "$PATCHELF" && -x "$PATCHELF" ]] || {
    echo "build-appimage: 找不到 patchelf（Fedora：sudo dnf install patchelf）" >&2
    exit 1
}
PATCHELF_VERSION="$("$PATCHELF" --version 2>/dev/null | awk '{print $2}')"
if [[ -z "$PATCHELF_VERSION" ]] || \
    [[ "$(printf '%s\n' 0.18 "$PATCHELF_VERSION" | sort -V | head -1)" != 0.18 ]]; then
    echo "build-appimage: patchelf 需要 >= 0.18，当前为 ${PATCHELF_VERSION:-未知}" >&2
    exit 1
fi
export PATCHELF

LINUXDEPLOY_SHA=c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d
QT_PLUGIN_SHA=15106be885c1c48a021198e7e1e9a48ce9d02a86dd0a1848f00bdbf3c1c92724
APPIMAGETOOL_SHA=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0

LINUXDEPLOY_URL="https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage"
QT_PLUGIN_URL="https://github.com/linuxdeploy/linuxdeploy-plugin-qt/releases/download/1-alpha-20250213-1/linuxdeploy-plugin-qt-x86_64.AppImage"
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage"
RUNTIME_URL="https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64"
RUNTIME="$TOOLS_DIR/runtime-x86_64"

fetch_tool() {
    local path="$1"
    local url="$2"
    local expected="$3"
    local tmp="$path.part"

    if [[ ! -x "$path" ]]; then
        command -v curl >/dev/null 2>&1 || {
            echo "build-appimage: 缺少 curl，且工具不在 $TOOLS_DIR" >&2
            exit 1
        }
        echo "==> 下载 $(basename "$path")"
        curl -fL --retry 8 --retry-all-errors --retry-delay 5 \
            --connect-timeout 15 -o "$tmp" "$url"
        echo "$expected  $tmp" | sha256sum -c -
        mv "$tmp" "$path"
        chmod +x "$path"
    fi

    echo "$expected  $path" | sha256sum -c --status - || {
        echo "build-appimage: 工具校验失败：$path" >&2
        exit 1
    }
}

prune_appdir() {
    local appdir="$1"
    local f p plugin
    local -A keep=()

    echo "==> 裁剪未使用的 Qt 模块"
    for path in \
        usr/qml/QtQuick/Controls/FluentWinUI3 \
        usr/qml/QtQuick/Controls/Imagine \
        usr/qml/QtQuick/Controls/Material \
        usr/qml/QtQuick/Controls/Universal \
        usr/qml/QtQuick/Controls/designer \
        usr/qml/QtQuick/LocalStorage \
        usr/qml/QtQuick/Particles \
        usr/qml/QtQuick/Pdf \
        usr/qml/QtQuick/Scene2D \
        usr/qml/QtQuick/Scene3D \
        usr/qml/QtQuick/VirtualKeyboard \
        usr/qml/QtQuick/tooling; do
        rm -rf "$appdir/$path"
    done

    # 缩略图由 PikPak 服务器下发（JPEG）；本地只保留图标 / 头像常见的格式。
    for plugin in "$appdir"/usr/plugins/imageformats/*.so; do
        [[ -e "$plugin" ]] || continue
        case "$(basename "$plugin")" in
            libqjpeg.so|libqgif.so|libqpng.so|libqsvg.so|libqwebp.so|libqico.so|libqicns.so)
                ;;
            *)
                rm -f "$plugin"
                ;;
        esac
    done
    rm -f "$appdir/usr/plugins/platforminputcontexts/libqtvirtualkeyboardplugin.so"

    if [[ -d "$appdir/usr/translations" ]]; then
        find "$appdir/usr/translations" -type f \
            ! -name 'qtbase_en.qm' ! -name 'qtbase_zh_CN.qm' \
            ! -name 'qtbase_zh_TW.qm' ! -name 'qtdeclarative_en.qm' \
            ! -name 'qtdeclarative_zh_CN.qm' ! -name 'qtdeclarative_zh_TW.qm' \
            -delete
    fi

    # 内嵌播放保留，但 libmpv / FFmpeg 交给宿主：先删掉 libmpv，
    # 下面的 ldd 可达性会把只被它拖进来的 FFmpeg / 编解码库一并裁掉。
    rm -f "$appdir"/usr/lib/libmpv.so "$appdir"/usr/lib/libmpv.so.*

    # 只保留从剩余程序、插件和 QML 模块可达的库；被裁模块拖进来的
    # libQt6Pdf / libQt63D / 冷门图片编解码器会在这一步一起消失。
    appdir="$(readlink -f "$appdir")"
    while IFS= read -r -d '' f; do
        file -b "$f" | grep -q '^ELF' || continue
        while IFS= read -r p; do
            p="$(readlink -f "$p")"
            [[ "$p" == "$appdir"/usr/lib/* ]] && keep["$p"]=1
        done < <(ldd "$f" 2>/dev/null | sed -n 's/.*=> \(.*\) (0x.*/\1/p')
    done < <(find "$appdir/usr/bin" "$appdir/usr/plugins" "$appdir/usr/qml" -type f -print0)

    while IFS= read -r -d '' f; do
        [[ -n "${keep["$f"]+x}" ]] || rm -f "$f"
    done < <(find "$appdir/usr/lib" -maxdepth 1 -type f -name '*.so*' -print0)
    find "$appdir/usr/lib" -maxdepth 1 -type l -print0 | while IFS= read -r -d '' f; do
        [[ -e "$f" ]] || rm -f "$f"
    done
}

mkdir -p "$TOOLS_DIR" "$DIST_DIR"
fetch_tool "$LINUXDEPLOY" "$LINUXDEPLOY_URL" "$LINUXDEPLOY_SHA"
fetch_tool "$QT_PLUGIN" "$QT_PLUGIN_URL" "$QT_PLUGIN_SHA"
fetch_tool "$APPIMAGETOOL" "$APPIMAGETOOL_URL" "$APPIMAGETOOL_SHA"

# appimagetool 默认会在打包时临时下载 runtime；单独缓存它，GitHub 下载
# 偶发 504 时至少第二次构建不用再等。没有固定 SHA（continuous 通道）。
if [[ ! -x "$RUNTIME" ]]; then
    command -v curl >/dev/null 2>&1 || {
        echo "build-appimage: 缺少 curl，且 runtime 不在 $TOOLS_DIR" >&2
        exit 1
    }
    echo "==> 下载 AppImage runtime"
    curl -fL --retry 8 --retry-all-errors --retry-delay 5 \
        --connect-timeout 15 -o "$RUNTIME.part" "$RUNTIME_URL"
    mv "$RUNTIME.part" "$RUNTIME"
    chmod +x "$RUNTIME"
fi
file -b "$RUNTIME" | grep -q '^ELF' || {
    echo "build-appimage: runtime 不是 ELF：$RUNTIME" >&2
    exit 1
}

# 构建环境可能没有 /dev/fuse；让 AppImage 工具走解包运行路径。
export APPIMAGE_EXTRACT_AND_RUN=1
export PATH="$TOOLS_DIR:$PATH"

# linuxdeploy 自带的 strip 来自较旧的 binutils，无法识别 Fedora 44 的
# `.relr.dyn` 段。依赖仍由 linuxdeploy 处理，应用二进制最后用宿主 strip 处理。
export NO_STRIP=1

QMAKE="${QMAKE:-$(command -v qmake6 || true)}"
if [[ -z "$QMAKE" ]]; then
    for candidate in /usr/lib64/qt6/bin/qmake6 /usr/lib/qt6/bin/qmake6; do
        [[ -x "$candidate" ]] && QMAKE="$candidate" && break
    done
fi
[[ -n "$QMAKE" && -x "$QMAKE" ]] || {
    echo "build-appimage: 找不到 qmake6（Fedora：qt6-qtbase-devel）" >&2
    exit 1
}
export QMAKE

QT_QML_DIR="$("$QMAKE" -query QT_INSTALL_QML)"
QT_PLUGINS_DIR="$("$QMAKE" -query QT_INSTALL_PLUGINS)"
QT_BIN_DIR="$("$QMAKE" -query QT_INSTALL_BINS)"
QT_LIBEXEC_DIR="$("$QMAKE" -query QT_INSTALL_LIBEXECS)"
export PATH="$QT_BIN_DIR:$QT_LIBEXEC_DIR:$PATH"

command -v qmlimportscanner >/dev/null 2>&1 || {
    echo "build-appimage: 找不到 qmlimportscanner（Fedora：qt6-qtdeclarative-devel）" >&2
    exit 1
}

echo "==> 构建 release 二进制"
cargo build --release --locked -p kichi

echo "==> 准备 AppDir"
rm -rf "$BUILD_DIR"
mkdir -p \
    "$APPDIR/usr/bin" \
    "$APPDIR/usr/share/applications" \
    "$APPDIR/usr/share/icons/hicolor/scalable/apps" \
    "$APPDIR/usr/share/licenses/kichi" \
    "$BUILD_DIR/qml-src"

# 只扫描运行时顶层 QML，避免把 tests 里的 QtTest 依赖带进包。
# main.qml 的 io.kichi 模块编在二进制里，扫描器无法解析；其余顶层组件
# 已覆盖实际用到的 Qt Quick / Kirigami / 桌面风格模块。
find kichi/qml -maxdepth 1 -type f -name '*.qml' ! -name 'main.qml' \
    -exec cp {} "$BUILD_DIR/qml-src/" \;
cp packaging/appimage/kichi.desktop.in "$BUILD_DIR/kichi.desktop"

if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$BUILD_DIR/kichi.desktop"
fi

# Kirigami 的桌面平台插件不在 QML 模块目录里，必须显式带上并让 linuxdeploy
# 递归部署它依赖的 KF6 库。
KIRIGAMI_PLATFORM_PLUGIN="$QT_PLUGINS_DIR/kf6/kirigami/platform/org.kde.desktop.so"
if [[ -f "$KIRIGAMI_PLATFORM_PLUGIN" ]]; then
    install -Dm644 "$KIRIGAMI_PLATFORM_PLUGIN" \
        "$APPDIR/usr/plugins/kf6/kirigami/platform/org.kde.desktop.so"
else
    echo "build-appimage: 警告：找不到 Kirigami 平台插件 $KIRIGAMI_PLATFORM_PLUGIN" >&2
fi

# KDE 会话使用 org.kde.desktop 风格，配色来自 KDE 平台主题；不带上它，
# 包内只剩 xdgdesktopportal，Plasma 下会退回默认调色板、丢失系统强调色。
KDE_PLATFORM_THEME="$QT_PLUGINS_DIR/platformthemes/KDEPlasmaPlatformTheme6.so"
if [[ -f "$KDE_PLATFORM_THEME" ]]; then
    install -Dm644 "$KDE_PLATFORM_THEME" \
        "$APPDIR/usr/plugins/platformthemes/KDEPlasmaPlatformTheme6.so"
else
    echo "build-appimage: 警告：找不到 KDE 平台主题 $KDE_PLATFORM_THEME；KDE 下可能退回默认浅色" >&2
fi

export QML_SOURCES_PATHS="$BUILD_DIR/qml-src:$ROOT/packaging/appimage"
export QML_MODULES_PATHS="$QT_QML_DIR"
# linuxdeploy-plugin-qt 默认只带 X11 平台插件；显式加入 Wayland，
# 否则 KDE Wayland 会话会静默退回 XWayland。
export EXTRA_PLATFORM_PLUGINS=libqwayland.so

echo "==> 部署 Qt / QML / KF6 依赖"
"$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --executable "$ROOT/target/release/kichi" \
    --desktop-file "$BUILD_DIR/kichi.desktop" \
    --icon-file "$ROOT/kichi/assets/icons/hicolor/scalable/apps/kichi.svg" \
    --icon-filename kichi \
    --plugin qt

# Wayland 平台插件的 shell / 图形集成插件由运行时分目录加载，
# linuxdeploy-plugin-qt 不会自动带上，需显式复制后部署依赖。
for plugin_dir in \
    wayland-decoration-client \
    wayland-graphics-integration-client \
    wayland-shell-integration; do
    if [[ -d "$QT_PLUGINS_DIR/$plugin_dir" ]]; then
        mkdir -p "$APPDIR/usr/plugins/$plugin_dir"
        cp -a "$QT_PLUGINS_DIR/$plugin_dir/." "$APPDIR/usr/plugins/$plugin_dir/"
    fi
done

# linuxdeploy-plugin-qt 负责 QML 模块，QML 模块的 ELF 依赖会由 linuxdeploy
# 继续递归部署；这里再处理显式放进 AppDir 的 Kirigami / Wayland 插件。
for plugin_path in \
    "$APPDIR/usr/plugins/kf6/kirigami/platform" \
    "$APPDIR/usr/plugins/platformthemes" \
    "$APPDIR/usr/plugins/wayland-decoration-client" \
    "$APPDIR/usr/plugins/wayland-graphics-integration-client" \
    "$APPDIR/usr/plugins/wayland-shell-integration"; do
    [[ -d "$plugin_path" ]] || continue
    "$LINUXDEPLOY" \
        --appdir "$APPDIR" \
        --deploy-deps-only "$plugin_path"
done

# linuxdeploy 会塞一个把 GNOME / XFCE 指向 gtk2 平台主题的钩子，但包内没有
# gtk2 插件；非 KDE 的外观改由应用自己读 XDG Desktop Portal。保留文件本身
# （AppRun 会无条件 source 它），只把内容换成空操作。
mkdir -p "$APPDIR/apprun-hooks"
cat > "$APPDIR/apprun-hooks/linuxdeploy-plugin-qt-hook.sh" <<'HOOK'
# Replaced by kichi: non-KDE appearance comes from XDG Desktop Portal.
# Do not set QT_QPA_PLATFORMTHEME here; the gtk2 platform theme is not packaged.
# linuxdeploy 的 AppRun 只会 source 这个文件，libmpv 检查必须从这里带进来。
. "$this_dir"/apprun-hooks/kichi-require-libmpv.sh
HOOK

# libmpv / FFmpeg 不随包提供，缺了在启动阶段给一句能照做的提示，
# 而不是让动态链接器抛 “libmpv.so.2: cannot open shared object file”。
cat > "$APPDIR/apprun-hooks/kichi-require-libmpv.sh" <<'HOOK'
if ! {
    ldconfig -p 2>/dev/null | grep -q 'libmpv\.so\.2' ||
    [ -e /usr/lib64/libmpv.so.2 ] || [ -e /usr/lib/libmpv.so.2 ] ||
    [ -e /lib64/libmpv.so.2 ] || [ -e /lib/libmpv.so.2 ]
}; then
    echo "kichi: 需要宿主安装 libmpv（Fedora: sudo dnf install mpv-libs；Arch: sudo pacman -S mpv；Debian: sudo apt install libmpv2）" >&2
    exit 1
fi
HOOK

prune_appdir "$APPDIR"

if command -v strip >/dev/null 2>&1; then
    strip --strip-unneeded "$APPDIR/usr/bin/kichi"
fi

echo "==> 验证动态库依赖"
missing=0
while IFS= read -r -d '' f; do
    file -b "$f" | grep -q '^ELF' || continue
    out="$(ldd "$f" 2>&1 || true)"
    # libmpv 由宿主提供，AppDir 里本来就不该有；其余未解析依赖仍算失败。
    unresolved="$(printf '%s\n' "$out" | grep 'not found' | grep -v 'libmpv\.so\.2' || true)"
    if [[ -n "$unresolved" ]]; then
        echo "build-appimage: $f 存在未解析依赖：" >&2
        printf '%s\n' "$unresolved" >&2
        missing=1
    fi
done < <(find "$APPDIR/usr" -type f \( -perm -u+x -o -name '*.so' -o -name '*.so.*' \) -print0)
[[ "$missing" == 0 ]] || exit 1

# libmpv 必须留给宿主；意外进包说明裁剪逻辑回退了。
[[ ! -e "$APPDIR/usr/lib/libmpv.so.2" ]] || {
    echo "build-appimage: 不该打进包的 libmpv.so.2" >&2
    exit 1
}

for required in \
    usr/bin/kichi \
    usr/qml/org/kde/desktop/qmldir \
    usr/qml/org/kde/kirigami/qmldir \
    usr/qml/QtQuick/Controls/Fusion/qmldir \
    usr/plugins/kf6/kirigami/platform/org.kde.desktop.so \
    usr/plugins/platforms/libqwayland.so \
    usr/plugins/platforms/libqxcb.so \
    usr/plugins/wayland-shell-integration/libxdg-shell.so \
    usr/lib/libQt6Quick.so.6; do
    [[ -f "$APPDIR/$required" ]] || {
        echo "build-appimage: AppDir 缺少必需文件：$required" >&2
        exit 1
    }
done

# KDE 平台主题在纯 GNOME 构建机上可能不存在；存在时必须成功进包。
if [[ -f "$KDE_PLATFORM_THEME" ]]; then
    [[ -f "$APPDIR/usr/plugins/platformthemes/KDEPlasmaPlatformTheme6.so" ]] || {
        echo "build-appimage: AppDir 缺少 KDE 平台主题" >&2
        exit 1
    }
fi

echo "==> 收集许可证与项目声明"
install -m644 LICENSE "$APPDIR/usr/share/licenses/kichi/LICENSE"
install -m644 THIRD_PARTY_NOTICES.md "$APPDIR/usr/share/licenses/kichi/THIRD_PARTY_NOTICES.md"
for pkg in qt6-qtbase qt6-qtdeclarative qt6-qtwayland kf6-kirigami \
    kf6-qqc2-desktop-style plasma-integration mpv-libs; do
    if [[ -d "/usr/share/licenses/$pkg" ]]; then
        mkdir -p "$APPDIR/usr/share/licenses/kichi/$pkg"
        cp -a "/usr/share/licenses/$pkg/." "$APPDIR/usr/share/licenses/kichi/$pkg/"
    fi
done

echo "==> 生成 AppImage"
"$APPIMAGETOOL" --no-appstream --runtime-file "$RUNTIME" "$APPDIR" "$DIST_DIR/$APPIMAGE_NAME"
chmod +x "$DIST_DIR/$APPIMAGE_NAME"
(
    cd "$DIST_DIR"
    sha256sum "$APPIMAGE_NAME" > "$APPIMAGE_NAME.sha256"
)

echo
echo "完成：$DIST_DIR/$APPIMAGE_NAME"
echo "大小：$(du -h "$DIST_DIR/$APPIMAGE_NAME" | cut -f1)"
echo "校验：$DIST_DIR/$APPIMAGE_NAME.sha256"
