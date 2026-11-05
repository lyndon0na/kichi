import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 由 build.rs 的 QmlModule("io.kichi") 生成
import io.kichi 1.0

ApplicationWindow {
    id: root

    title: "kichi"
    // 上次关窗时的尺寸在 Component.onCompleted 里赋一次（见下），这里只放默认值。
    // ⚠ 绝不能写成绑定读 backend.windowWidth：cxx-qt 所有属性共用一个 NOTIFY 信号，
    //   任何一次 notify()（包括最大化后 0.5 s 的防抖存盘自己）都会让绑定重新求值，
    //   把已经最大化的窗口按旧尺寸 resize 回去——现象是点最大化后放大一下又缩回
    //   原尺寸、落到屏幕左上角（2026-10-28 用户实机 + kdotool 0.4 s 内复现）。
    width: 1000
    height: 640
    visible: true
    // 窗口底色 = 内容「白纸」色：页头、列表共用一块面，靠侧栏的浅灰区分出外壳
    color: palette.base

    // 会话对象：构造时自动恢复 KWallet 里的登录态
    Kichi {
        id: backend
    }

    // 文件直链是带签名的临时地址：只在 Rust 侧取一次，信号一到就写剪贴板，
    // 不存属性、不绑定到界面文字。
    ClipboardHelper {
        id: fileLinkClipboard
    }

    Connections {
        target: backend

        function onFileLinkReady(id, url) {
            fileLinkClipboard.copyText(url)
        }
    }

    property bool signedIn: backend.authState === "signedIn"
    /// 当前是否最大化（Wayland 下窗口状态由混成器管，这里跟踪自己的意图）
    property bool isMaximized: false
    /// 缩到托盘后窗口隐藏，这里记住「是托盘收起来的」，不是用户主动关
    property bool hiddenToTray: false
    /// 真的要退出（托盘菜单「退出」/注销托盘后），不缩回托盘
    property bool reallyQuitting: false

    Component.onCompleted: {
        // 记忆的尺寸只在这里赋一次：赋值会替换声明处的默认值，之后 backend 再怎么
        // notify 也不会带着窗口尺寸跑（原因见 title 上方注释）
        if (backend.windowWidth > 0)
            root.width = backend.windowWidth
        if (backend.windowHeight > 0)
            root.height = backend.windowHeight
        backend.warmUp()
        if (backend.windowMaximized)
            showMaximized()
        else
            isMaximized = false
    }

    onWidthChanged: saveGeometryTimer.restart()
    onHeightChanged: saveGeometryTimer.restart()
    onVisibilityChanged: function (visibility) {
        if (visibility === Window.Maximized || visibility === Window.FullScreen)
            isMaximized = true
        else if (visibility === Window.Windowed)
            isMaximized = false
        saveGeometryTimer.restart()
    }
    // 重新获得焦点：让本地索引与网盘对一次账（Rust 侧有 60 秒冷却与单飞）
    onActiveChanged: {
        if (root.active && root.signedIn)
            backend.onWindowActivated()
    }

    // 拖窗口边缘会连续触发尺寸变化：防抖后再落盘
    Timer {
        id: saveGeometryTimer
        interval: 500
        onTriggered: {
            var w = root.isMaximized ? backend.windowWidth : root.width
            var h = root.isMaximized ? backend.windowHeight : root.height
            backend.saveWindowGeometry(w, h, root.isMaximized)
        }
    }

    onClosing: function (close) {
        // 托盘点「退出」= 真退出，不能又被下面的缩托盘逻辑拦下来
        // （Qt.quit() 同样会触发 onClosing，不特判就会出现
        //  「点了退出图标还在、窗口也点不回来」——2026-10-26 用户实测）
        if (root.reallyQuitting) {
            backend.shutdownTray()
            backend.shutdownPlayers()
            return
        }
        if (backend.closeToTray && backend.trayAvailable) {
            close.accepted = false
            saveGeometryTimer.stop()
            // 最大化时保留上一次普通尺寸，别把最大化尺寸当成还原尺寸
            var w = root.isMaximized ? backend.windowWidth : root.width
            var h = root.isMaximized ? backend.windowHeight : root.height
            backend.saveWindowGeometry(w, h, root.isMaximized)
            root.hiddenToTray = true
            root.hide()
        } else {
            // 真退出：注销托盘，否则 KDE 面板留下点不动的僵尸图标；
            // 连同正在播放的 mpv / VLC 一起关（缩托盘时不关，见 ADR 0007）
            backend.shutdownTray()
            backend.shutdownPlayers()
        }
    }

    // —— 托盘 ——
    // ⚠ 2026-10-26 实测：Qt.labs.platform 的 SystemTrayIcon 在 KDE 平台主题下
    // 会走 KStatusNotifierItem，它要构造 QMenu（QtWidgets），而本应用只有
    // QGuiApplication → qFatal → 启动即 abort。容器一变就崩，连「关掉设置」
    // 的机会都没有。已改用 Rust 侧 ksni（纯 D-Bus），见 cxxqt_object/tray.rs。

    // 托盘事件：用 Connections 而不是 onXxxChanged（后者对 cxx-qt 生成的
    // 信号名推导不可靠，实测会报 "Cannot assign to non-existent property"）
    Connections {
        target: backend

        function onChanged() {
            // 显示：托盘点了图标或「显示 kichi」
            if (backend.trayShowRequested > 0 && root.lastTrayShow !== backend.trayShowRequested) {
                root.lastTrayShow = backend.trayShowRequested
                root.restoreFromTray()
            }
            // 退出：托盘「退出」，真退出而不是缩回托盘。
            // 必须先注销托盘，否则 KDE 面板会留下点不动的僵尸图标
            if (backend.quitRequested) {
                root.reallyQuitting = true
                Qt.quit()
            }
        }

        // 已占用的条目再点一次：切到传输任务页并定位那一行。
        // 第一次下载不发这个信号——不打断用户正在浏览的目录。
        // 窗口最小化 / 在托盘时照常切页，但不主动把窗口弹出来（现状）。
        function onLocateTransfer(sourceId) {
            backend.clearStatusMessage()
            root.currentPage = pageIds.transfers
            taskCenterPage.focusTransfer(sourceId)
        }

        // 离线任务文件夹的路径与内容已在后台准备好，这时才切页。
        function onTaskFolderReady() {
            root.currentPage = pageIds.files
        }

        // 离线任务产物的父目录已准备好，切到文件页后按产物 id 选中目标。
        function onTaskLocationReady(targetId) {
            root.currentPage = pageIds.files
            filesPage.revealEntryById(targetId)
        }

        function onLibraryFolderReady() {
            root.currentPage = pageIds.files
        }

        // 资料库右键定位：父目录已就绪后切到文件页并选中目标。
        function onLibraryLocationReady(targetId) {
            root.currentPage = pageIds.files
            filesPage.revealEntryById(targetId)
        }
    }

    property int lastTrayShow: 0

    /// 从托盘唤回窗口。窗口本来就在前台时也执行 show/raise，当作「置顶」处理。
    function restoreFromTray() {
        root.hiddenToTray = false
        if (!root.visible)
            root.show()
        root.raise()
        root.requestActivate()
    }

    // 后台线程的结果由主线程抽水投递（cxx-qt 0.10 下不能用 qsignal）
    Timer {
        interval: 100
        running: true
        repeat: true
        onTriggered: backend.pump()
    }

    StackLayout {
        anchors.fill: parent
        currentIndex: root.signedIn ? 1 : 0

        LoginPage {
            backend: backend
        }

        RowLayout {
            spacing: 0

            SideBar {
                Layout.preferredWidth: root.width < 760 ? 168 : 208
                Layout.fillHeight: true
                backend: backend
                currentIndex: root.currentPage
                onPageRequested: function (index) { root.navigateTo(index) }
                onSettingsRequested: root.navigateTo(pageIds.settings)
            }

            // 侧栏与内容之间的细线：两边的底色只差一点点，没有线就看不出边界
            Rectangle {
                Layout.preferredWidth: 1
                Layout.fillHeight: true
                color: theme.hairline
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                // 页头两行，和列表同一块底色（页头是内容的一部分，不单独成栏）：
                //   上行 页名 + 搜索框 + 目录操作
                //   下行 面包屑（只有文件页有）
                Item {
                    id: titleRow

                    Layout.fillWidth: true
                    // 左边对齐列表行的图标列（行内边距 12），右边对齐「修改时间」的右边缘
                    // （行内边距 12；滚动条悬浮在这段留白里，不占布局）——页头和列表
                    // 共用两条竖轴，换页时内容不会横向错开
                    Layout.leftMargin: metrics.pageMargin
                    Layout.rightMargin: metrics.pageMargin
                    Layout.topMargin: 10
                    Layout.preferredHeight: 38

                    Label {
                        id: pageTitleLabel

                        anchors.left: parent.left
                        anchors.verticalCenter: parent.verticalCenter
                        text: root.pageTitle()
                        opacity: 1
                        transform: Translate {
                            id: pageTitleShift
                            y: 0
                        }
                        // 一级标题：18px Bold，和正文（13）拉开层级（docs/ui.md）
                        font.pixelSize: theme.fontTitle
                        font.weight: Font.Bold
                        color: theme.textPrimary
                    }

                    ShareHeaderControls {
                        id: shareHeaderControls

                        visible: root.currentPage === pageIds.shares
                        anchors.left: pageTitleLabel.right
                        anchors.leftMargin: theme.gapSm
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        target: sharesPage
                    }

                    TransferHeaderControls {
                        visible: root.currentPage === pageIds.transfers
                                 && taskCenterPage.currentTab === "transfers"
                        anchors.left: pageTitleLabel.right
                        anchors.leftMargin: theme.gapSm
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        target: taskCenterPage.transfersView
                    }

                    TaskHeaderControls {
                        visible: root.currentPage === pageIds.transfers
                                 && taskCenterPage.currentTab === "offline"
                        anchors.left: pageTitleLabel.right
                        anchors.leftMargin: theme.gapSm
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        target: taskCenterPage.tasksView
                    }

                    TrashHeaderControls {
                        visible: root.currentPage === pageIds.trash
                        anchors.left: pageTitleLabel.right
                        anchors.leftMargin: theme.gapSm
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        target: trashPage
                    }

                    LibraryHeaderControls {
                        id: libraryHeaderControls

                        visible: pageIds.libraryPages.indexOf(root.currentPage) >= 0
                        target: root.currentPage === pageIds.recentSaved
                                ? recentSavedPage
                                : root.currentPage === pageIds.playbackHistory
                                  ? playbackHistoryPage : starredPage
                        anchors.left: pageTitleLabel.right
                        anchors.leftMargin: theme.gapSm
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                    }

                    RowLayout {
                        visible: root.currentPage === pageIds.settings
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: theme.gapXs

                        NavIcon {
                            implicitWidth: 14
                            implicitHeight: 14
                            name: "check"
                            color: theme.textMuted
                            opacity: 0.8
                        }

                        Label {
                            text: qsTr("更改会自动保存")
                            font.pixelSize: theme.fontMeta
                            color: theme.textMuted
                        }
                    }

                    FilesPageSearchBar {
                        id: searchBar

                        visible: root.currentPage === pageIds.files
                        anchors.horizontalCenter: parent.horizontalCenter
                        anchors.verticalCenter: parent.verticalCenter
                        // 真正居中，同时按左右标题 / 操作区的剩余空间收窄，避免重叠。
                        width: {
                            const center = titleRow.width / 2
                            const leftEdge = pageTitleLabel.x + pageTitleLabel.width
                                            + theme.gapMd
                            const rightEdge = filesHeaderActions.x - theme.gapMd
                            const available = 2 * Math.min(center - leftEdge,
                                                           rightEdge - center)
                            return Math.max(160, Math.min(420, available))
                        }
                        backend: backend
                    }

                    // 目录操作带文字；刷新固定排在最后。
                    FilesPageHeaderActions {
                        id: filesHeaderActions

                        visible: root.currentPage === pageIds.files
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        backend: backend
                        target: filesPage
                        newTaskTarget: taskCenterPage
                    }
                }

                // —— 下行：面包屑 ——
                RowLayout {
                    Layout.fillWidth: true
                    Layout.leftMargin: metrics.pageMargin
                    Layout.rightMargin: metrics.pageMargin
                    Layout.topMargin: metrics.pageMargin
                    // 下边距交给内容自己的 topMargin（同样是 pageMargin）：
                    // 面包屑到表头的距离 = 页名到面包屑的距离
                    Layout.bottomMargin: 0
                    Layout.preferredHeight: 30
                    visible: root.currentPage === pageIds.files
                    spacing: 4

                    // 面包屑：上一级 + 各级目录，可点着往回跳。
                    // 独占一行，宽度全归自己；放不下时由 plan 决定显示哪几级，
                    // clip 兜底，只会截断末尾，不会盖住旁边。
                    Item {
                        id: crumbBar

                        Layout.fillWidth: true
                        Layout.minimumWidth: 0
                        // 不写 Layout.fillHeight：布局里的 fillHeight 子项会把整行撑到
                        // 占满剩余高度（实测页头涨成 900px、列表被挤到底部）。
                        // 内容本来就靠 verticalCenter 居中，不需要它。
                        Layout.preferredHeight: 30
                        clip: true

                        // 字号比页名（18）小两级，和正文同档：面包屑是路径，
                        // 不该和页标题一样重
                        readonly property int crumbPixelSize: 13
                        // 一级目录名最多占多宽；超过就中间截断（名字末尾常是版本号，
                        // 掐中间留得住「漫威合集...PTer」）
                        readonly property int nameMaxWidth: 220
                        // 「›」+ 间距，估算每一级额外的固定开销
                        readonly property int separatorWidth: 13
                        // 「...」标记占的宽度（含前面的「›」）
                        readonly property int ellipsisWidth: 23
                        // 中间某一级读不全会截断时，最少留给它这么宽
                        // （约五个汉字：够认出是哪一层）
                        readonly property int minCrumbWidth: 76
                        // 量文字宽度：当前级是 DemiBold，宽度和普通级不一样，得分开量。
                        // 用 FontMetrics 的纯测宽函数，不在 plan 绑定里改 TextMetrics.text；
                        // 后者会让 plan 读到自己在计算过程中改写的依赖，触发绑定循环。
                        FontMetrics {
                            id: normalMetrics
                            font.pixelSize: crumbBar.crumbPixelSize
                        }

                        FontMetrics {
                            id: boldMetrics
                            font.pixelSize: crumbBar.crumbPixelSize
                            font.weight: Font.DemiBold
                        }

                        function crumbWidth(text, bold) {
                            var m = bold ? boldMetrics : normalMetrics
                            return Math.min(crumbBar.nameMaxWidth, m.advanceWidth(text))
                        }

                        // 要显示哪几级 + 每一级给多宽：{ i: 目录下标（-1 = 省略号）, w: 宽度 }。
                        //
                        // 为什么要自己算，不交给布局压缩：QtQuick 布局在宽度不够时，
                        // 是让「每一级」按各自的收缩余地等比变窄——四级目录平摊下来
                        // 「全部文件」会被截成「全...件」、「Movie」只剩「M...e」，
                        // 屏幕上全是碎片，一个名字也读不出来。
                        //
                        // 现在的规则：
                        // 1. 当前目录优先占满（最多 nameMaxWidth）；
                        // 2. 第一级永远显示（「全部文件」）；
                        // 3. 中间几级从最靠近当前的那级往回带：整级放得下就整级显示，
                        //    只差一点就截断到剩余宽度（≥ minCrumbWidth），再早的折掉；
                        // 4. 有级别被折掉时，第一级后面补一个「...」。
                        // 名字因此只有「完整」和「整段截断」两种，不会碎成几个字。
                        readonly property var plan: {
                            var names = root.crumbs
                            var n = names.length
                            if (n === 0)
                                return []
                            if (n === 1)
                                return [{ i: 0, w: crumbBar.crumbWidth(names[0], true) }]

                            var sep = crumbBar.separatorWidth
                            var widths = []
                            for (var i = 0; i < n; ++i)
                                widths.push(crumbBar.crumbWidth(names[i], i === n - 1))

                            // 当前级优先占位；容器窄到连它都放不下时也让它让一点，
                            // 免得名字被截在容器外面
                            var currentWidth = widths[n - 1]
                            var avail = crumbBar.width - upButton.width - sep
                                    - widths[0] - sep
                            if (currentWidth > avail)
                                currentWidth = Math.max(120, avail)
                            avail -= currentWidth

                            var items = [{ i: 0, w: widths[0] }]
                            var middle = []
                            for (var j = n - 2; j >= 1; --j) {
                                // 更早的层级还没显示时，中间要留「...」的位置
                                var reserve = (j > 1 ? crumbBar.ellipsisWidth : 0) + sep
                                var room = avail - reserve
                                if (room >= widths[j]) {
                                    middle.unshift({ i: j, w: widths[j] })
                                    avail -= widths[j] + sep
                                } else if (room >= crumbBar.minCrumbWidth) {
                                    middle.unshift({ i: j, w: room })
                                    break
                                } else {
                                    break
                                }
                            }

                            // 中间有级别被折掉：第一级后面标「...」
                            var firstShownMiddle = middle.length > 0 ? middle[0].i : n - 1
                            if (firstShownMiddle > 1)
                                items.push({ i: -1, w: crumbBar.ellipsisWidth })
                            items = items.concat(middle)
                            items.push({ i: n - 1, w: currentWidth })
                            return items
                        }

                        RowLayout {
                            anchors.left: parent.left
                            anchors.verticalCenter: parent.verticalCenter
                            // 「›」两侧各留 4px：贴住名字会读成一个字
                            spacing: 4

                            KIconButton {
                                id: upButton

                                iconName: "arrow-up"
                                iconSize: 14
                                iconOpacity: 0.65
                                enabled: root.crumbs.length > 1
                                ToolTip.visible: hovered
                                ToolTip.text: qsTr("上一级（回到上级目录）")
                                onClicked: filesPage.goUp()
                            }

                            Repeater {
                                model: crumbBar.plan

                                delegate: RowLayout {
                                    id: crumb

                                    // 目录下标；-1 是省略号
                                    readonly property int crumbIndex: modelData.i
                                    readonly property bool ellipsis: crumbIndex < 0
                                    readonly property bool current: crumbIndex === root.crumbs.length - 1

                                    spacing: 4

                                    Label {
                                        text: "›"
                                        visible: index > 0
                                        font.pixelSize: crumbBar.crumbPixelSize
                                        color: palette.placeholderText
                                    }

                                    // 省略号：表示中间还有目录，不响应点击
                                    Label {
                                        visible: crumb.ellipsis
                                        text: "..."
                                        font.pixelSize: crumbBar.crumbPixelSize
                                        color: palette.placeholderText
                                    }

                                    ElidedLabel {
                                        id: crumbName
                                        objectName: "crumbNameText"

                                        visible: !crumb.ellipsis
                                        text: crumb.ellipsis ? "" : root.crumbs[crumb.crumbIndex]
                                        font.pixelSize: crumbBar.crumbPixelSize
                                        // 当前目录半粗 + 正文色，前几级普通字重 + 弱化色：
                                        // 整行不跟页名抢，又看得出哪一级是「现在这里」
                                        font.weight: crumb.current
                                                     ? Font.DemiBold : Font.Normal
                                        color: crumb.current
                                               ? palette.text
                                               : Qt.rgba(palette.text.r, palette.text.g,
                                                         palette.text.b, 0.7)
                                        elide: Text.ElideMiddle
                                        // 宽度由 plan 算好，不再交给布局压缩：
                                        // 布局压缩会按比例切短名字（见 plan 处的说明）
                                        Layout.preferredWidth: crumb.ellipsis ? 0 : modelData.w
                                        externallyHovered: crumbMouse.containsMouse

                                        MouseArea {
                                            id: crumbMouse
                                            anchors.fill: parent
                                            enabled: !crumb.current
                                            hoverEnabled: true
                                            cursorShape: enabled ? Qt.PointingHandCursor
                                                                 : Qt.ArrowCursor
                                            onClicked: filesPage.goCrumb(crumb.crumbIndex)
                                        }
                                    }

                                    // 加载中转圈：只跟在当前目录名后面
                                    BusyIndicator {
                                        visible: crumb.current
                                                 && (backend.filesLoading
                                                     || backend.filesRefreshing)
                                        running: visible
                                        Layout.preferredWidth: 18
                                        Layout.preferredHeight: 18
                                        Layout.alignment: Qt.AlignVCenter
                                    }
                                }
                            }

                        }
                    }

                }

                // 内容区：与页头同一块底色（窗口色就是 palette.base），中间不画分隔线
                StackLayout {
                    id: contentStack

                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    currentIndex: root.currentPage
                    opacity: 1

                    FilesPage {
                        id: filesPage
                        backend: backend
                        windowVisible: root.visible
                        playMenuEnabled: root.currentPage === pageIds.files
                                         || root.currentPage === pageIds.transfers
                    }

                    SharesPage {
                        id: sharesPage
                        backend: backend
                        active: root.currentPage === pageIds.shares
                    }

                    TaskCenterPage {
                        id: taskCenterPage
                        backend: backend
                        active: root.currentPage === pageIds.transfers
                        viewportWidth: root.width
                        onOpenFolderRequested: function (fileId) {
                            backend.openTaskFolder(fileId)
                        }
                        onRevealLocationRequested: function (fileId) {
                            backend.revealTaskEntry(fileId)
                        }
                        onPlayRequested: function (fileId, position) {
                            filesPage.preparePlayMenu(position)
                            backend.requestTaskPlay(fileId)
                        }
                    }

                    TrashPage {
                        id: trashPage
                        backend: backend
                    }

                    SettingsPage {
                        backend: backend
                        onSignOutRequested: signOutDialog.open()
                    }

                    RecentSavedPage {
                        id: recentSavedPage
                        backend: backend
                        active: root.currentPage === pageIds.recentSaved
                    }

                    PlaybackHistoryPage {
                        id: playbackHistoryPage
                        backend: backend
                        active: root.currentPage === pageIds.playbackHistory
                    }

                    StarredPage {
                        id: starredPage
                        backend: backend
                        active: root.currentPage === pageIds.starred
                    }
                }
            }
        }
    }

    // 播放错误 / 回退提示条：加载中的条目在当前列表里由缩略图内联显示，
    // 条目不再可见（切目录 / 退出搜索 / 换标签页）或离开对应页面时，这里接管取消入口。
    // 底部居中浮动，只占自身尺寸（踩坑第 6 / 18 条）。
    // ApplicationWindow 的 contentItem 已经把 footer 的高度扣掉，
    // 所以贴 parent.bottom 就已经在底栏之上。
    PlayToast {
        id: playToast
        backend: backend
        inlineLoading: {
            if (root.currentPage === pageIds.files)
                return filesPage.showsInlinePlayLoading
            if (root.currentPage === pageIds.transfers)
                return taskCenterPage.showsInlinePlayLoading
            if (root.currentPage === pageIds.recentSaved)
                return recentSavedPage.showsInlinePlayLoading
            if (root.currentPage === pageIds.playbackHistory)
                return playbackHistoryPage.showsInlinePlayLoading
            if (root.currentPage === pageIds.starred)
                return starredPage.showsInlinePlayLoading
            return false
        }
    }

    // 底栏：左侧单槽消息 + 右侧固定传输入口。消息默认显示当前页面 / 目录上下文，
    // 进度、错误和需要留意的结果临时覆盖它，结束后自动回到上下文。
    // 登录页只在本就有错误时出现，此时右侧传输入口隐藏。
    footer: Item {
        id: statusBar

        readonly property int barHeight: 32
        readonly property string message: backend.statusMessage
        readonly property string shownText: message.length > 0
                                             ? message : root.pageContextText()
        readonly property bool taskNavigationLoading: backend !== null
                && String(backend.taskFolderLoadingId || "").length > 0

        implicitHeight: visible ? barHeight : 0
        visible: root.signedIn || message.length > 0

        Rectangle {
            anchors.fill: parent
            color: theme.surfaceRaised
        }

        // 顶部细线：与内容自然过渡，不用硬边框
        Rectangle {
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            implicitHeight: 1
            color: theme.hairline
        }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: metrics.pageMargin
            anchors.rightMargin: metrics.pageMargin
            spacing: theme.gapSm

            BusyIndicator {
                objectName: "statusBarLoadingSpinner"
                Layout.preferredWidth: 14
                Layout.preferredHeight: 14
                Layout.alignment: Qt.AlignVCenter
                visible: statusBar.taskNavigationLoading
                running: visible
            }

            ElidedLabel {
                objectName: "statusBarLabel"
                Layout.fillWidth: true
                Layout.fillHeight: true
                text: statusBar.shownText
                elide: Text.ElideRight
                verticalAlignment: Text.AlignVCenter
                color: statusBar.message.length > 0 ? theme.textSecondary : theme.textMuted
                font.pixelSize: theme.fontMeta
                showToolTipWhenUntruncated: true
            }

            Rectangle {
                Layout.preferredWidth: 1
                Layout.preferredHeight: 16
                Layout.alignment: Qt.AlignVCenter
                visible: root.signedIn
                color: theme.divider
            }

            TransferStatusButton {
                objectName: "transferStatusButton"

                visible: root.signedIn
                activeCount: backend.downloadActive + backend.uploadActive
                offlineCount: taskCenterPage.offlineActiveCount
                pageActive: root.currentPage === pageIds.transfers
                onRequested: {
                    if (root.currentPage !== pageIds.transfers)
                        taskCenterPage.showTaskEntry()
                    root.navigateTo(pageIds.transfers)
                }
            }
        }
    }

    KDialog {
        id: signOutDialog
        title: qsTr("退出登录")
        modal: true
        anchors.centerIn: parent
        width: 360 + leftPadding + rightPadding
        footer: StandardDialogFooter {
            dialog: signOutDialog
        }

        Label {
            text: qsTr("将清除本机保存的登录令牌，下次需要重新输入账号密码。确定退出？")
            wrapMode: Text.Wrap
            width: 360
        }

        onAccepted: backend.signOut()
    }

    property int currentPage: pageIds.files

    onCurrentPageChanged: pageTransition.restart()

    ParallelAnimation {
        id: pageTransition

        NumberAnimation {
            target: contentStack
            property: "opacity"
            from: 0
            to: 1
            duration: theme.motionNormal
            easing.type: Easing.OutCubic
        }

        NumberAnimation {
            target: pageTitleLabel
            property: "opacity"
            from: 0
            to: 1
            duration: theme.motionNormal
            easing.type: Easing.OutCubic
        }

        NumberAnimation {
            target: pageTitleShift
            property: "y"
            from: theme.motionPageLift
            to: 0
            duration: theme.motionNormal
            easing.type: Easing.OutCubic
        }
    }

    Shortcut {
        sequence: "Ctrl+,"
        enabled: root.signedIn
        onActivated: root.navigateTo(pageIds.settings)
    }

    /// 面包屑的各级目录名。模板里的 model / 长度 / 折叠判断都从这里取，
    // 保证三者永远是同一份数据（分开读后端时，改一处容易忘另一处）
    readonly property var crumbs: backend ? backend.breadcrumb : []

    // 页面级尺寸常量（滚动条悬浮宽度 / 页边距），与各页共用同一套数字
    UiMetrics {
        id: metrics
    }

    PageIndex {
        id: pageIds
    }

    // 全局视觉令牌（圆角 / 间距 / 字号 / 表面色）
    KTheme {
        id: theme
    }

    function pageTitle() {
        if (root.currentPage === pageIds.shares)
            return qsTr("分享")
        if (root.currentPage === pageIds.transfers)
            return qsTr("任务")
        if (root.currentPage === pageIds.trash)
            return qsTr("回收站")
        if (root.currentPage === pageIds.settings)
            return qsTr("设置")
        if (root.currentPage === pageIds.recentSaved)
            return qsTr("最近保存")
        if (root.currentPage === pageIds.playbackHistory)
            return qsTr("播放历史")
        if (root.currentPage === pageIds.starred)
            return qsTr("已加星标")
        return qsTr("我的文件")
    }

    function navigateTo(pageIndex) {
        if (root.currentPage !== pageIndex)
            backend.clearStatusMessage()
        root.currentPage = pageIndex
    }

    /// 底栏无提醒时显示的上下文。读取的是各页已经维护的加载态与计数，
    /// 不另起网络请求，也不把常规刷新写进 statusMessage。
    function pageContextText() {
        if (!backend)
            return ""
        if (root.currentPage === pageIds.files) {
            if (backend.searchActive) {
                if (!backend.searchSettled) {
                    // 本地索引没有「已扫多少条」的进度，别借用网络扫描的文案
                    if (backend.searchSource === "index")
                        return qsTr("正在本地索引里搜索…")
                    return qsTr("全盘搜索 · 已扫 %1 条，命中 %2 条")
                            .arg(backend.searchScanned)
                            .arg(backend.searchHits)
                }
                return qsTr("搜索「%1」· 命中 %2 条")
                        .arg(backend.searchQuery)
                        .arg(backend.searchHits)
            }
            const folder = root.crumbs.length > 0
                    ? root.crumbs[root.crumbs.length - 1] : qsTr("全部文件")
            if (backend.filesLoading)
                return qsTr("正在读取「%1」…").arg(folder)
            if (backend.filesRefreshing)
                return qsTr("%1 · 共 %2 项 · 正在刷新…")
                        .arg(folder).arg(backend.files.length)
            return qsTr("%1 · 共 %2 项").arg(folder).arg(backend.files.length)
        }
        if (root.currentPage === pageIds.shares) {
            if (backend.sharesLoading)
                return qsTr("分享 · 正在读取…")
            return qsTr("分享 · 共 %1 条").arg(backend.shares.length)
        }
        if (root.currentPage === pageIds.transfers) {
            if (taskCenterPage.currentTab === "offline") {
                if (taskCenterPage.tasksView.tasksLoading)
                    return qsTr("离线任务 · 正在读取…")
                const count = backend.tasks.length
                const active = taskCenterPage.tasksView.activeCount
                if (active > 0) {
                    return qsTr("离线任务 · 共 %1 条，%2 条进行中")
                            .arg(count)
                            .arg(active)
                }
                return qsTr("离线任务 · 共 %1 条").arg(count)
            }
            const active = taskCenterPage.transfersView.activeCount
            const queued = taskCenterPage.transfersView.queuedCount
            const paused = taskCenterPage.transfersView.pausedCount
            if (active + queued + paused === 0)
                return qsTr("本机传输 · 暂无进行中的任务")
            return qsTr("本机传输 · %1").arg(taskCenterPage.transfersView.statusText)
        }
        if (root.currentPage === pageIds.trash) {
            if (backend.trashLoading
                    || (!backend.trashLoaded && backend.trashError.length === 0))
                return qsTr("回收站 · 正在读取…")
            if (!backend.trashLoaded && backend.trashError.length > 0)
                return qsTr("回收站 · 读取失败")
            return qsTr("回收站 · 共 %1 项").arg(backend.trashFiles.length)
        }
        if (pageIds.libraryPages.indexOf(root.currentPage) >= 0) {
            const title = root.pageTitle()
            if (backend.libraryLoading)
                return qsTr("%1 · 正在读取…").arg(title)
            if (backend.libraryError.length > 0)
                return qsTr("%1 · 读取失败").arg(title)
            if (backend.libraryKind === "recent-saved" && root.currentPage === pageIds.recentSaved
                    || backend.libraryKind === "playback-history"
                       && root.currentPage === pageIds.playbackHistory
                    || backend.libraryKind === "starred" && root.currentPage === pageIds.starred) {
                return qsTr("%1 · 共 %2 项").arg(title).arg(backend.libraryRows.length)
            }
            return title
        }
        return qsTr("设置")
    }
}
