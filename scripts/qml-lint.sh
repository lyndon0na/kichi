#!/usr/bin/env bash
# QML 静态检查：挡住「整个界面加载失败、应用启动即崩」的那类写法错误。
#
# 为什么需要：QML 是运行时解析的，`cargo build` 完全看不出错误。
# 一次属性名拼错（如给 TextField 写 onEscapePressed，它其实属于 Keys 附加对象）
# 就能让 main.qml 整体加载失败、应用起不来，而构建依旧打印 "Finished"。
# `qmllint` 对这种写法报 `no matching signal found for handler`，改动前后对比明显。
#
# 判定：先构建（拿到目标里生成的 io.kichi 模块 qmltypes），再 lint。
# 与已知基线（KNOWN）比对，出现基线之外的告警/错误即判失败。
# 基线里的条目都是运行期已被验证可加载的历史写法，逐条写明原因。
#
# 用法：
#   ./scripts/qml-lint.sh

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

# —— 规则：滚动条只有一种做法（docs/pitfalls.md 第 13 条）——
# 裸 ScrollBar 自带默认宽度、浮动在内容之上，各页外观会各不相同。
# 页面里一律 `ScrollBar.vertical: KScrollBar {}`；下面的静态检查当场拦住新写法。
raw="$(grep -rnE '(^|[^A-Za-z])ScrollBar[[:space:]]*\{|ScrollBar\.vertical:[[:space:]]*ScrollBar' \
        kichi/qml --include='*.qml' | grep -v '/KScrollBar.qml:' || true)"
if [[ -n "$raw" ]]; then
    echo "qml-lint: 发现裸 ScrollBar，应改用 KScrollBar（见 docs/pitfalls.md 第 13 条）：" >&2
    printf '%s\n' "$raw" >&2
    exit 1
fi

# qmllint 常在 /usr/lib64/qt6/bin，该系统目录默认不在 PATH
LINT="$(command -v qmllint || true)"
if [[ -z "$LINT" ]]; then
    for cand in /usr/lib64/qt6/bin/qmllint /usr/lib/qt6/bin/qmllint; do
        [[ -x "$cand" ]] && LINT="$cand" && break
    done
fi
if [[ -z "$LINT" ]]; then
    echo "qml-lint: 找不到 qmllint（Fedora：qt6-qtdeclarative-devel）" >&2
    exit 1
fi

# `io.kichi` 的类型信息由构建生成；没有它，Kichi 及其属性全都无法解析
MODULES="target/cxxqt/qml_modules"
if [[ ! -d "$MODULES/io/kichi" ]]; then
    echo "qml-lint: 缺少 $MODULES/io/kichi，先跑 cargo build -p kichi" >&2
    exit 1
fi

# 已知基线：<文件>:<行>: <被豁免的关键词>
# unqualified 是仓库里大量存在的历史写法，不影响加载，统一放行。
# 第三条是 FilesPagePlayMenu.qml 里 Repeater delegate 访问外层 menu id 的 lint 误报
#   （该菜单在真实界面里已验证可弹出）。不写文件名和行号：文件一改就会失效，
#   而这条告警本身只出现在那种写法上。
KNOWN=(
    ': Unqualified access'
    'Member "backend" not found on type "QQuickMenu"'
)

out="$("$LINT" -I "$MODULES" kichi/qml/*.qml 2>&1)"

bad=""
while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    case "$line" in
    Warning:* | Error:*) ;;
    *) continue ;;
    esac
    known=0
    for pat in "${KNOWN[@]}"; do
        if [[ "$line" == *"$pat"* ]]; then
            known=1
            break
        fi
    done
    if [[ $known -eq 0 ]]; then
        bad+="$line"$'\n'
    fi
done <<<"$out"

if [[ -n "$bad" ]]; then
    echo "qml-lint: 发现可能导致加载失败的 QML 问题：" >&2
    printf '%s' "$bad" >&2
    exit 1
fi

echo "qml-lint: 通过（无基线之外的问题）"
