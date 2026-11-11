import QtQuick

// 文件页内部拖拽移动的协调器（纯逻辑，不画界面）。
//
// 拖动源是文件行 / 网格卡片上的 Drag 附加属性，落点是文件夹行 / 卡片和面包屑。
// 这里集中保管「这次拖的是哪几条」「当前悬停的落点」，并把落点转成一次
// moveEntries 调用；行和卡片只负责把事件转进来、渲染高亮。
//
// 数据只用自定义 MIME，不带 text/uri-list：拖到外部应用时没有接收方，
// 不会变成一次意外的文件复制 / 打开。
QtObject {
    id: controller

    /// 拖拽在界面里是否可用：文件页可见、已登录、非搜索 / 退出搜索过渡、非加载中。
    property bool enabled: false
    property var backend: null
    property var selection: null
    /// 当前显示的条目：用来识别还没拿到真实 id 的临时行。
    property var entries: []
    property string currentDirId: ""
    readonly property var breadcrumb: controller.backend
                                       && controller.backend.breadcrumb
                                       ? controller.backend.breadcrumb : []
    readonly property var breadcrumbIds: controller.backend
                                         && controller.backend.breadcrumbIds
                                         ? controller.backend.breadcrumbIds : []

    /// 拖拽数据的唯一 MIME 类型；落点据此判断是不是本应用发起的拖动。
    readonly property string mimeType: "application/x-kichi-file-ids"

    /// 跟随指针的拖影卡片（FilesPage 传入；没有它也能拖，只是光标下没有图）。
    property var dragPreview: null
    /// 由拖影卡片抓出来的图片 URL，直接喂给 Drag.imageSource。
    property string dragImageUrl: ""
    /// 抓住 grab 结果对象，拖拽期间不让它被回收（图片缓存随它一起失效）。
    property var dragImageResult: null
    /// 每次都换一个令牌：上一轮迟到的 grab 回调不能污染这一次拖动。
    property int imageToken: 0

    /// 按下的条目已经准备好拖动数据（还没超过拖动阈值）。
    property bool prepared: false
    /// 平台拖拽真的开始了（超过阈值）；松手 / 取消 / 拖拽结束都回到 false。
    property bool dragging: false
    property string sourceDir: ""
    property var dragIds: []
    /// 按下那一条的名字：只拖一条时提示里直接显示它，让用户知道手里拿的是什么。
    property string dragName: ""
    /// `{ sourceDir, ids }` 的 JSON；Drag.mimeData 直接取这里。
    property string payloadJson: ""
    readonly property var mimeData: controller.payloadJson.length > 0
                                    ? ({ "application/x-kichi-file-ids": controller.payloadJson })
                                    : ({})

    /// 当前悬停的落点："folder" / "crumb" / ""（没有高亮）。
    property string hoverKind: ""
    property string hoverId: ""
    property int hoverIndex: -1
    /// 悬停落点的显示名：拖动提示里说明「会移到哪里」。
    property string hoverName: ""

    function entryForId(id) {
        const key = String(id || "")
        for (let i = 0; i < controller.entries.length; ++i) {
            const entry = controller.entries[i]
            if (entry && String(entry.id) === key)
                return entry
        }
        return null
    }

    /// 临时行（新建文件夹还没换到真实云端 id）不能移动。
    function isPending(id) {
        const entry = controller.entryForId(id)
        return !!entry && entry.pending === true
    }

    /// 与文件页写操作共用同一把按条目锁（backend.fileActionIds）。
    function isLocked(id) {
        const key = String(id || "")
        if (!controller.backend || key.length === 0)
            return false
        const ids = controller.backend.fileActionIds
        if (!ids)
            return false
        for (let i = 0; i < ids.length; ++i) {
            if (String(ids[i]) === key)
                return true
        }
        return false
    }

    /// 按下的条目能否作为拖动源。
    ///
    /// 已选中的条目拖整个选择，未选中的只拖这一条；选择里只要有一条还没拿到
    /// 真实 id 或正被写操作占用，就整次不拖，避免发出注定被拒绝的请求。
    function begin(entry) {
        controller.imageToken += 1
        controller.prepared = false
        controller.dragging = false
        controller.payloadJson = ""
        controller.dragIds = []
        controller.dragName = ""
        controller.dragImageUrl = ""
        controller.dragImageResult = null
        controller.clearHover()
        if (!controller.enabled || !controller.backend || !entry || !entry.id)
            return false
        if (entry.inTrash === true || entry.pending === true)
            return false
        const ids = controller.selection
                  && controller.selection.isSelected(entry.id)
                ? controller.selection.idList() : [String(entry.id)]
        if (ids.length === 0)
            return false
        for (let i = 0; i < ids.length; ++i) {
            if (controller.isPending(ids[i]) || controller.isLocked(ids[i]))
                return false
        }
        controller.dragIds = ids
        controller.dragName = String(entry.name || "")
        controller.sourceDir = String(controller.currentDirId || "")
        controller.payloadJson = JSON.stringify({
            sourceDir: controller.sourceDir,
            ids: ids
        })
        controller.prepared = true
        controller.prepareDragImage(entry, ids.length)
        return true
    }

    /// 让拖影卡片按这次拖动的内容成像。
    ///
    /// 在按下时就抓：grabToImage 要到下一帧才回调，而平台拖拽要等指针越过
    /// 8px 阈值才启动，正常操作下图片已经就绪。抓不到就让这次拖动没有拖影，
    /// 不影响移动本身。
    function prepareDragImage(entry, count) {
        const preview = controller.dragPreview
        if (!preview)
            return
        preview.entry = entry
        preview.itemCount = count
        const token = controller.imageToken
        const grabbed = preview.grabToImage(function (result) {
            if (token !== controller.imageToken || !controller.prepared)
                return
            controller.dragImageResult = result
            controller.dragImageUrl = result ? String(result.url || "") : ""
        })
        if (!grabbed)
            controller.dragImageUrl = ""
    }

    /// 平台拖拽越过阈值：Drag.onDragStarted 调这里。
    function dragStarted() {
        controller.dragging = controller.prepared
    }

    /// 松手 / 取消 / 平台拖拽结束：清掉本次拖动与悬停态。
    function finishDrag() {
        controller.imageToken += 1
        controller.prepared = false
        controller.dragging = false
        controller.dragImageUrl = ""
        controller.dragImageResult = null
        controller.clearHover()
    }

    /// 这个条目能不能作为落点（文件夹本体、非回收站、非临时行、未被写操作占用）。
    function folderTarget(entry) {
        if (!controller.enabled || !controller.backend || !entry)
            return ""
        if (entry.kind !== "folder" || entry.inTrash === true
                || entry.pending === true)
            return ""
        const id = String(entry.id || "")
        if (id.length === 0 || controller.isLocked(id))
            return ""
        return id
    }

    function hoverFolder(entry) {
        if (controller.folderTarget(entry).length === 0) {
            controller.clearHover()
            return false
        }
        controller.hoverKind = "folder"
        controller.hoverId = String(entry.id)
        controller.hoverIndex = -1
        controller.hoverName = String(entry.name || "")
        return true
    }

    function hoverCrumb(index) {
        if (!controller.enabled || index < 0
                || index >= controller.breadcrumbIds.length) {
            controller.clearHover()
            return false
        }
        controller.hoverKind = "crumb"
        controller.hoverId = String(controller.breadcrumbIds[index])
        controller.hoverIndex = index
        controller.hoverName = String(controller.breadcrumb[index] || "")
        return true
    }

    function clearHover() {
        controller.hoverKind = ""
        controller.hoverId = ""
        controller.hoverIndex = -1
        controller.hoverName = ""
    }

    /// 这一条是不是本次拖动带走的条目：行 / 卡片据此显示「已拿起」。
    function isDragged(id) {
        const key = String(id || "")
        if (!controller.dragging || key.length === 0)
            return false
        for (let i = 0; i < controller.dragIds.length; ++i) {
            if (String(controller.dragIds[i]) === key)
                return true
        }
        return false
    }

    function isHoveredFolder(id) {
        return controller.hoverKind === "folder"
                && controller.hoverId === String(id || "")
    }

    function isHoveredCrumb(index) {
        return controller.hoverKind === "crumb" && controller.hoverIndex === index
    }

    /// 落点是文件夹行 / 卡片：交给后端走与右键「移动到…」同一条链路。
    /// 同目录 / 自己 / 自己的子目录不在这里判断，由 moveEntries 统一拒绝并提示。
    function dropOnFolder(entry, raw) {
        const payload = controller.parsePayload(raw)
        const targetId = controller.folderTarget(entry)
        controller.finishDrag()
        if (!payload || targetId.length === 0)
            return false
        return controller.backend.moveEntries(payload.ids, targetId,
                                              String(entry.name || ""))
    }

    /// 落点是面包屑某一级：目标 id 来自与名字并列的 breadcrumbIds。
    function dropOnCrumb(index, raw) {
        const payload = controller.parsePayload(raw)
        const crumbIds = controller.breadcrumbIds
        const crumbNames = controller.breadcrumb
        controller.finishDrag()
        if (!payload || !controller.enabled || index < 0
                || index >= crumbIds.length)
            return false
        return controller.backend.moveEntries(
                    payload.ids, String(crumbIds[index]),
                    String(crumbNames[index] || ""))
    }

    /// 校验落点收到的 JSON；结构不对就当没有这次拖动。
    function parsePayload(raw) {
        const text = raw === undefined || raw === null ? "" : String(raw)
        if (text.length === 0)
            return null
        let data = null
        try {
            data = JSON.parse(text)
        } catch (error) {
            return null
        }
        if (!data || typeof data !== "object"
                || typeof data.sourceDir !== "string")
            return null
        // QVariantList 不是 JS Array：只按 length / 下标遍历（见 pitfalls 58）。
        const rawIds = data.ids
        if (!rawIds || typeof rawIds.length !== "number")
            return null
        const ids = []
        for (let i = 0; i < rawIds.length; ++i) {
            const id = String(rawIds[i] || "")
            if (id.length === 0)
                return null
            ids.push(id)
        }
        if (ids.length === 0)
            return null
        return { sourceDir: String(data.sourceDir), ids: ids }
    }
}
