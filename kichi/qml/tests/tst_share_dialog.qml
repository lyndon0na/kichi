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
    property var urlField: findChild(dialog, "shareCreatedUrl")

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

    function openFor(ids) {
        dialog.openFor({ id: ids[0], name: "a.mkv" }, ids)
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
        compare(urlField.text, "https://mypikpak.com/s/abc")
        compare(createDialog.copyNotice, "", "结果出现时不能假装已经复制")
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
