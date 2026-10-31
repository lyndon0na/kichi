import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 资料库三页共用的页头操作，不显示条目数或读取状态。
RowLayout {
    id: root

    property var target: null

    readonly property bool loading: root.target ? root.target.loading : false

    readonly property KTheme theme: KTheme { parent: root }

    spacing: theme.gapSm

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
