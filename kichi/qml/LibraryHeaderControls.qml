import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 资料库三页共用的页头状态与刷新入口，与分享 / 离线任务页同构。
RowLayout {
    id: root

    property var target: null

    readonly property int entryCount: root.target ? root.target.entries.length : 0
    readonly property bool loading: root.target ? root.target.loading : false
    readonly property bool hasError: root.target
                                     && String(root.target.errorText || "").length > 0

    readonly property KTheme theme: KTheme { parent: root }

    spacing: theme.gapSm

    ElidedLabel {
        objectName: "libraryHeaderStatus"
        Layout.fillWidth: true
        Layout.alignment: Qt.AlignVCenter
        text: root.loading
              ? (root.entryCount > 0 ? qsTr("正在刷新…") : qsTr("正在读取…"))
              : root.hasError && root.entryCount === 0
                ? qsTr("读取失败")
                : root.entryCount > 0
                  ? qsTr("共 %1 项").arg(root.entryCount) : ""
        color: root.hasError && !root.loading ? theme.danger : theme.textMuted
        font.pixelSize: theme.fontMeta
        elide: Text.ElideRight
    }

    ContentViewToggle {
        Layout.alignment: Qt.AlignVCenter
        backend: root.target ? root.target.backend : null
        onViewRequested: function (view) {
            if (root.target)
                root.target.setContentView(view)
        }
    }

    KButton {
        objectName: "libraryRefreshButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("刷新")
        iconName: "refresh"
        variant: "ghost"
        enabled: root.target !== null && root.target.backend !== null && !root.loading
                 && !root.target.backend.fileActionRunning
        onClicked: {
            if (root.target)
                root.target.refresh()
        }
    }
}
