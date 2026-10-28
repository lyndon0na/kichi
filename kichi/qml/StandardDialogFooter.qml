import QtQuick
import QtQuick.Controls

// 标准按钮页脚：确定 / 取消，用 Qt 标准按钮（保留系统给的按钮顺序、图标，
// 点确定由 Dialog 自己关窗）。
//
// 与 `DialogFooter.qml` 的分工：那个给**异步**操作用（点确定不能关窗，要等结果
// 回来再决定）；这个给同步确认用。两者共用同一条留白规则：
// Dialog 的 footer 是整宽、贴底摆放的，不吃 Dialog.padding（见 docs/pitfalls.md
// 第 28 条），所以要自己补——调用方把所在 Dialog 传进来，padding 取它的值，
// 按钮就和正文的左右留白对齐。
DialogButtonBox {
    id: box

    /// 所在弹窗（用来取 padding；没传时退回 12）
    property var dialog: null

    standardButtons: Dialog.Ok | Dialog.Cancel
    padding: box.dialog ? box.dialog.padding : 12
}
