import QtQuick
import QtQuick.Controls

// 剪贴板桥：Qt Quick 没有公开的 Qt.application.clipboard。
// 用一个离屏 TextEdit 的 selectAll() / copy() 完成复制，避免新增平台相关依赖。
Item {
    id: root

    function copyText(text) {
        const value = String(text || "")
        if (value.length === 0)
            return false
        editor.text = value
        editor.selectAll()
        editor.copy()
        return true
    }

    TextEdit {
        id: editor
        x: -10000
        y: -10000
        width: 1
        height: 1
        opacity: 0
        readOnly: true
        selectByMouse: false
        textFormat: TextEdit.PlainText
        wrapMode: TextEdit.NoWrap
    }
}
