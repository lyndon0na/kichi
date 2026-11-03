import QtQuick
import QtQuick.Controls

// 剪贴板桥：Qt Quick 没有公开的 Qt.application.clipboard。
// 用一个离屏 TextEdit 的 selectAll() / copy() 完成复制，避免新增平台相关依赖。
Item {
    id: root

    // 分享统一复制成一段可直接转发的文本：名称、链接、访问密码各占一行；
    // 没有名称或密码时省略对应行。
    function shareText(url, passCode, title) {
        const link = String(url || "").trim()
        if (link.length === 0)
            return ""
        const name = String(title || "").trim()
        const code = String(passCode || "").trim()
        const lines = []
        if (name.length > 0)
            lines.push(name)
        lines.push(link)
        if (code.length > 0)
            lines.push(qsTr("访问密码：%1").arg(code))
        return lines.join("\n")
    }

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
        objectName: "clipboardEditor"
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
