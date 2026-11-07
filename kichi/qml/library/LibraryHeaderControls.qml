import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"
import "../files"

// 资料库三页共用的页头操作，不显示条目数；只有「已加星标」在标题旁放数据来源。
RowLayout {
    id: root

    property var target: null

    readonly property bool loading: root.target ? root.target.loading : false

    // 数据来源提示（目前只有「已加星标」会填）：只在索引已开启且就绪时出现。
    readonly property string sourceText: root.target
                                          && root.target.backend
                                          && root.target.backend.localIndexEnabled === true
                                          && root.target.kind === "starred"
                                        ? String(root.target.backend.libraryStatus || "")
                                        : ""

    readonly property KTheme theme: KTheme { parent: root }

    spacing: theme.gapSm

    Label {
        objectName: "librarySourceHint"
        Layout.alignment: Qt.AlignVCenter
        Layout.maximumWidth: 300
        visible: text.length > 0
        text: root.sourceText
        color: theme.textMuted
        font.pixelSize: theme.fontMeta
        elide: Text.ElideRight
    }

    Item {
        Layout.fillWidth: true
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
        iconSize: 14
        busy: root.loading
        variant: "ghost"
        enabled: root.target !== null && root.target.backend !== null && !root.loading
        onClicked: {
            if (root.target)
                root.target.refresh()
        }
    }
}
