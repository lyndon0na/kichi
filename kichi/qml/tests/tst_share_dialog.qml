import QtQuick
import QtTest
import ".."

TestCase {
    name: "ShareDialog"
    when: windowShown
    width: 900
    height: 700

    QtObject {
        id: backend

        property bool shareActionRunning: false
        property string shareCreatedUrl: ""
        property string shareCreatedPassCode: ""
        property string shareCreatedExpiration: ""

        property int createCalls: 0
        property var lastIds: []
        property int lastDays: 0
        property bool lastPassword: false

        signal changed()
        signal shareActionDone(string action, bool ok, string message)

        function createShare(ids, days, needPassword) {
            createCalls += 1
            lastIds = ids.slice()
            lastDays = days
            lastPassword = needPassword
            shareActionRunning = true
        }

        function finishCreate(ok, message) {
            shareActionRunning = false
            shareActionDone("create", ok, message)
        }
    }

    ShareDialog {
        id: dialog
        backend: backend
    }

    property var createDialog: findChild(dialog, "shareCreateDialog")
    property var expiration: findChild(dialog, "shareExpiration")
    property var customDays: findChild(dialog, "shareCustomDays")
    property var passwordBox: findChild(dialog, "sharePasswordRequired")
    property var errorLabel: findChild(dialog, "shareCreateError")
    property var contentType: findChild(dialog, "shareCreatedContent")
    property var copyButton: createDialog.stage === "success"
                             ? findChild(dialog, "shareCopyLink") : null
    property var doneButton: createDialog.stage === "success"
                             ? findChild(dialog, "shareDoneButton") : null
    property var footerStatus: findChild(dialog, "dialogFooterStatus")
    property var clipboardEditor: findChild(dialog, "clipboardEditor")

    function init() {
        backend.shareActionRunning = false
        backend.shareCreatedUrl = ""
        backend.shareCreatedPassCode = ""
        backend.shareCreatedExpiration = ""
        backend.createCalls = 0
        backend.lastIds = []
        backend.lastDays = 0
        backend.lastPassword = false
        if (createDialog.visible)
            createDialog.close()
    }

    function cleanup() {
        if (createDialog.visible)
            createDialog.close()
    }

    function openFor(ids, name) {
        dialog.openFor({ id: ids[0], name: name || "a.mkv" }, ids)
        tryCompare(createDialog, "visible", true)
    }

    function test_defaults_are_seven_days_and_password_required() {
        openFor(["a", "b"])
        compare(expiration.currentIndex, 1)
        compare(expiration.currentText, qsTr("7 天"))
        compare(passwordBox.checked, true)

        dialog.submit()
        compare(backend.createCalls, 1)
        compare(backend.lastIds.length, 2)
        compare(backend.lastDays, 7)
        compare(backend.lastPassword, true)
    }

    function test_custom_days_are_submitted_as_an_integer() {
        openFor(["a"])
        expiration.currentIndex = 3
        customDays.text = "14"
        dialog.submit()
        compare(backend.lastDays, 14)
    }

    function test_success_switches_to_result_without_auto_copying() {
        openFor(["a"])
        dialog.submit()
        backend.shareCreatedUrl = "https://mypikpak.com/s/abc"
        backend.shareCreatedPassCode = "p9k2"
        backend.shareCreatedExpiration = "7 天"
        backend.finishCreate(true, "created")

        tryCompare(createDialog, "stage", "success")
        compare(contentType.text,
                "https://mypikpak.com/s/abc\n访问密码：" + backend.shareCreatedPassCode)
        compare(createDialog.copyNotice, "", "结果出现时不能假装已经复制")
        compare(findChild(dialog, "shareCreatedPassword"), null)
        compare(findChild(dialog, "shareCopyPassword"), null)
        compare(createDialog.spacing, 16, "正文和页脚之间要留出足够间距")
        compare(copyButton.text, "复制链接")
        compare(copyButton.variant, "primary")
        compare(doneButton.text, "完成")
        compare(doneButton.variant, "secondary")
        verify(footerStatus.visible, "复制提示和操作按钮要放在同一行")
    }

    function test_copy_link_copies_url_and_password_together() {
        openFor(["a"])
        dialog.submit()
        backend.shareCreatedUrl = "https://mypikpak.com/s/abc"
        backend.shareCreatedPassCode = "p9k2"
        backend.shareCreatedExpiration = "7 天"
        backend.finishCreate(true, "created")
        tryCompare(createDialog, "stage", "success")

        mouseClick(copyButton)
        compare(clipboardEditor.text,
                "https://mypikpak.com/s/abc\n访问密码：" + backend.shareCreatedPassCode)
        compare(createDialog.copyNotice, "链接和密码已复制")
        compare(footerStatus.text, "链接和密码已复制")
    }

    function test_long_item_name_does_not_overlap_result_and_footer() {
        openFor(["a"], "[DBD-Raws][Poppin'Party×MyGO!!!!! 合同ライブ「Divide／Unite」]"
                       + "[1080P][BDRip][HEVC-10bit][FLAC][MKV]")
        dialog.submit()
        backend.shareCreatedUrl = "https://mypikpak.com/s/abc"
        backend.shareCreatedPassCode = "p9k2"
        backend.shareCreatedExpiration = "7 天"
        backend.finishCreate(true, "created")
        tryCompare(createDialog, "stage", "success")
        wait(40)

        const contentBottom = createDialog.contentItem.mapToItem(
                    createDialog.background, 0, createDialog.contentItem.height).y
        const footerTop = createDialog.footer.mapToItem(createDialog.background, 0, 0).y
        compare(createDialog.contentItem.implicitHeight,
                createDialog.contentItem.childrenRect.height,
                "正文高度必须参与弹窗尺寸计算")
        verify(contentBottom <= footerTop,
               "正文不能压住页脚（正文底 " + contentBottom
               + "，页脚顶 " + footerTop + "）")

        const statusCenter = footerStatus.mapToItem(
                    createDialog.footer, 0, footerStatus.height / 2).y
        const copyCenter = copyButton.mapToItem(
                    createDialog.footer, 0, copyButton.height / 2).y
        const doneCenter = doneButton.mapToItem(
                    createDialog.footer, 0, doneButton.height / 2).y
        verify(Math.abs(statusCenter - copyCenter) <= 2
               && Math.abs(copyCenter - doneCenter) <= 2,
               "提示、复制链接和完成必须在同一行")

        const copyRight = copyButton.mapToItem(
                    createDialog.footer, copyButton.width, 0).x
        const doneLeft = doneButton.mapToItem(createDialog.footer, 0, 0).x
        verify(copyRight <= doneLeft,
               "复制链接和完成不能互相覆盖（复制右 " + copyRight
               + "，完成左 " + doneLeft + "）")
    }

    function test_failure_stays_in_form_and_shows_reason() {
        openFor(["a"])
        dialog.submit()
        backend.finishCreate(false, "没有权限")
        compare(createDialog.stage, "form")
        compare(errorLabel.text, "没有权限")
        compare(errorLabel.visible, true)
    }
}
