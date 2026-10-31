import QtQuick

// 已加星标页：服务端按 STAR 标签筛选，Rust 再按更新时间倒序并去重。
LibraryPage {
    kind: "starred"
    primaryHeader: qsTr("大小")
    secondaryHeader: qsTr("更新时间")
    emptyText: qsTr("账号里还没有已加星标的项目")
}
