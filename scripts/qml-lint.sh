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

# —— 规则：滚动条只有一种做法 ——
# 裸 ScrollBar 自带默认宽度、浮动在内容之上，各页外观会各不相同。
# 页面里一律 `ScrollBar.vertical: KScrollBar {}`；下面的静态检查当场拦住新写法。
raw="$(grep -rnE '(^|[^A-Za-z])ScrollBar[[:space:]]*\{|ScrollBar\.vertical:[[:space:]]*ScrollBar' \
        kichi/qml --include='*.qml' | grep -v '/KScrollBar.qml:' || true)"
if [[ -n "$raw" ]]; then
    echo "qml-lint: 发现裸 ScrollBar，应改用 KScrollBar：" >&2
    printf '%s\n' "$raw" >&2
    exit 1
fi

# —— 规则：滚动视图不能越过边界回弹 ——
# Qt Quick 默认是 DragAndOvershootBounds，会像移动端一样在首尾橡皮筋回弹。
# 生产 QML 里出现可滚视图的文件都必须显式声明 StopAtBounds。
scroll_files="$(grep -rlE '(^|[^A-Za-z])(ListView|GridView|Flickable)[[:space:]]*\{' \
        kichi/qml --include='*.qml' | grep -v '/tests/' || true)"
missing_bounds=""
while IFS= read -r file; do
    [[ -z "$file" ]] && continue
    if ! grep -qE 'boundsBehavior:[[:space:]]*Flickable\.StopAtBounds' "$file"; then
        missing_bounds+="$file"$'\n'
    fi
done <<<"$scroll_files"
if [[ -n "$missing_bounds" ]]; then
    echo "qml-lint: 可滚视图缺少 boundsBehavior: Flickable.StopAtBounds" >&2
    printf '%s' "$missing_bounds" >&2
    exit 1
fi

# —— 规则：进度条只有一种做法 ——
# breeze 风格的 ProgressBar 在宽度变化时会逐帧刷 TypeError，
# 页面里一律 KProgressBar；下面这段静态检查当场拦住裸写法。
rawbar="$(grep -rnE '(^|[^A-Za-z])ProgressBar[[:space:]]*\{' \
        kichi/qml --include='*.qml' | grep -v '/KProgressBar.qml:' || true)"
if [[ -n "$rawbar" ]]; then
    echo "qml-lint: 发现裸 ProgressBar，应改用 KProgressBar：" >&2
    printf '%s\n' "$rawbar" >&2
    exit 1
fi

# —— 规则：QML 暴露名不能冲突 ——
# qproperty 与 qinvokable 共用 QML 命名空间；重名时属性会遮住同名方法，
# 构建和静态检查仍然通过，直到用户点击才报 "not a function"。
duplicate_cxx_names="$(grep -oE 'cxx_name = "[^"]+"' kichi/src/cxxqt_object.rs \
    | sed -E 's/cxx_name = "(.*)"/\1/' | sort | uniq -d)"
if [[ -n "$duplicate_cxx_names" ]]; then
    echo "qml-lint: kichi/src/cxxqt_object.rs 存在重复的 QML 暴露名：" >&2
    while IFS= read -r name; do
        grep -nF "cxx_name = \"$name\"" kichi/src/cxxqt_object.rs >&2
    done <<<"$duplicate_cxx_names"
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

# —— 运行期逻辑测试（QtQuick Test）——
# 静态检查看不出 ListModel 的越界 move、差异算法死循环这类问题。
RUNNER="$(command -v qmltestrunner || true)"
if [[ -z "$RUNNER" ]]; then
    for cand in /usr/lib64/qt6/bin/qmltestrunner /usr/lib/qt6/bin/qmltestrunner; do
        [[ -x "$cand" ]] && RUNNER="$cand" && break
    done
fi
if [[ -z "$RUNNER" ]]; then
    echo "qml-lint: 找不到 qmltestrunner（Fedora：qt6-qtdeclarative-devel）" >&2
    exit 1
fi

# 警告即失败：`QML ListModel: move: out of range` 这类问题只以 QWARN 出现，
# 进程照跑、测试照过，只有盯着输出才看得见（2026-10-27 传输页卡死就是这样漏的）。
# 还要加超时：差异算法一旦死循环，界面线程原地空转，测试进程不会自己退出。
for test_file in kichi/qml/tests/tst_*.qml; do
    [[ -f "$test_file" ]] || continue
    test_out="$(QT_QPA_PLATFORM=offscreen timeout 120 "$RUNNER" -input "$test_file" 2>&1)"
    status=$?
    if [[ $status -ne 0 ]] || grep -qE '^(FAIL!|QWARN|WARNING)' <<<"$test_out"; then
        echo "qml-lint: 运行期测试失败：$test_file（退出码 $status）" >&2
        printf '%s\n' "$test_out" >&2
        exit 1
    fi
    echo "qml-lint: $test_file 通过"
done

# 真实界面跑在 org.kde.desktop 风格下；它的 ScrollBar 会把内边距绑到样式自己的
# 槽位上，默认风格里能用的 padding 在那里可能完全不生效（滑块宽度不变）。
# 滚动条样式必须两种风格下都通过。
kde_out="$(QT_QPA_PLATFORM=offscreen QT_QUICK_CONTROLS_STYLE=org.kde.desktop \
    timeout 120 "$RUNNER" -input kichi/qml/tests/tst_scroll_bar.qml 2>&1)"
status=$?
if [[ $status -ne 0 ]] || grep -qE '^(FAIL!|QWARN|WARNING)' <<<"$kde_out"; then
    echo "qml-lint: 滚动条样式在 KDE 风格下失败（退出码 $status）" >&2
    printf '%s\n' "$kde_out" >&2
    exit 1
fi
echo "qml-lint: tst_scroll_bar.qml（KDE 风格）通过"
