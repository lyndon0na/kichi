import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 全盘搜索结束且没有命中时的排查建议。结果视图本身由文件页复用。
//
// 搜索源是 fallback 时，这次只筛了当前目录，建议要针对本地索引给出原因和
// 开启入口；只有索引真的在工作（index / network）时才套用通用建议，
// 否则「索引可能还没收录」会让人误以为索引已经开启。
ColumnLayout {
    id: root

    property var backend: null

    // 搜索源：index / fallback / network；空串 = 没有搜索。
    // 测试替身可能没有这个属性（undefined），按「没有搜索」处理。
    readonly property string source: backend !== null && backend.searchSource !== undefined
                                     ? backend.searchSource : ""
    readonly property bool fallback: source === "fallback"
    readonly property bool indexOff: fallback && backend.localIndexEnabled === false
    readonly property bool indexBuilding: fallback
                                          && backend.localIndexEnabled === true
                                          && backend.indexBuilding === true
    readonly property bool indexUnavailable: fallback
                                             && backend.localIndexEnabled === true
                                             && !indexBuilding

    visible: backend !== null
             && backend.searchActive === true
             && backend.searchSettled === true
             && backend.searchResults.length === 0
    spacing: 8

    Label {
        Layout.fillWidth: true
        horizontalAlignment: Text.AlignHCenter
        text: qsTr("没有找到名称包含「%1」的条目")
            .arg(root.backend ? root.backend.searchQuery : "")
        wrapMode: Text.Wrap
    }

    Label {
        objectName: "searchEmptyStateAdvice"
        Layout.fillWidth: true
        text: {
            if (root.indexOff)
                return qsTr("这次只搜了当前文件夹，没有搜索整个网盘。开启本地索引后会先在后台建立一次目录索引；建好后重新输入关键词，就能在全盘范围搜索（不发出搜索请求）。")
            if (root.indexBuilding)
                return qsTr("本地索引正在后台建立，建好前只搜当前文件夹；建好后重新输入关键词即可全盘搜索。")
            if (root.indexUnavailable)
                return qsTr("本地索引尚未就绪或读取失败，这次只搜了当前文件夹；可在设置中关闭再开启「本地目录索引」重试，或查看索引状态。")
            return qsTr("可以试试：\n· 换更短的关键词（按名字任意位置匹配，不支持 * ? 通配符）\n· 本地索引可能还没收录最新上传的文件\n· 只搜文件名，不搜文件内容；回收站里的条目也会列出")
        }
        color: palette.placeholderText
        wrapMode: Text.Wrap
        font.pixelSize: 12
    }

    // 只在「索引被用户关掉」时给一键开启；未就绪 / 建库中不给按钮，
    // 开关已经是开着的，再点一次没有意义。
    KButton {
        objectName: "enableLocalIndexButton"
        Layout.alignment: Qt.AlignHCenter
        visible: root.indexOff
        text: qsTr("开启本地索引")
        variant: "primary"
        onClicked: {
            if (root.backend)
                root.backend.setLocalIndexEnabled(true)
        }
    }
}
