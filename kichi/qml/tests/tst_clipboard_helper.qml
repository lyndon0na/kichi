import QtQuick
import QtTest
import ".."

TestCase {
    name: "ClipboardHelper"
    when: windowShown
    width: 300
    height: 200

    ClipboardHelper { id: helper }

    TextEdit {
        id: sink
        width: 120
        height: 60
    }

    function test_copies_plain_text_into_the_application_clipboard() {
        sink.text = ""
        sink.forceActiveFocus()
        verify(helper.copyText("https://mypikpak.com/s/example"))
        sink.paste()
        compare(sink.text, "https://mypikpak.com/s/example")
    }

    function test_empty_text_is_ignored() {
        compare(helper.copyText(""), false)
    }

    function test_share_text_keeps_link_and_password_together() {
        const passCode = ["a", "1", "b", "2"].join("")
        compare(helper.shareText(" https://mypikpak.com/s/example ",
                                 " " + passCode + " "),
                "https://mypikpak.com/s/example\n访问密码：" + passCode)
        compare(helper.shareText("https://mypikpak.com/s/public", ""),
                "https://mypikpak.com/s/public")
    }
}
