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
}
