import QtQuick
import QtTest
import "../files"
import "../components"

TestCase {
    name: "FileIcons"
    when: windowShown
    width: 120
    height: 80

    Component {
        id: fileIconComponent

        FileIcon {
            color: "black"
        }
    }

    Component {
        id: navIconComponent

        NavIcon {
            color: "black"
        }
    }

    property var probe: null

    function init() {
        probe = fileIconComponent.createObject(parent)
        verify(probe !== null)
    }

    function cleanup() {
        if (probe)
            probe.destroy()
        probe = null
    }

    function expectExtensions(extensions, expected) {
        for (var i = 0; i < extensions.length; ++i) {
            probe.kind = "file"
            probe.fileName = "sample." + extensions[i]
            compare(probe.iconName, expected, extensions[i])
        }
    }

    function test_folder_kind_wins_over_extension() {
        probe.kind = "folder"
        probe.fileName = "movie.mp4"
        compare(probe.iconName, "folder")
    }

    function test_media_extensions() {
        expectExtensions([
            "mp4", "mkv", "avi", "mov", "webm", "ts", "m2ts", "m4v",
            "flv", "wmv", "rmvb", "rm", "mpg", "mpeg", "mts", "vob",
            "3gp", "ogv"
        ], "video")
        expectExtensions([
            "mp3", "flac", "wav", "m4a", "ogg", "opus", "aac", "ape",
            "wma", "aiff", "alac", "mka", "m4b"
        ], "music")
        expectExtensions([
            "jpg", "jpeg", "png", "gif", "webp", "bmp", "heic", "heif",
            "avif", "svg", "tif", "tiff", "ico", "jxl", "raw"
        ], "photo")
    }

    function test_document_and_office_extensions() {
        expectExtensions(["pdf"], "pdf")
        expectExtensions(["doc", "docx", "odt", "rtf", "txt", "md"], "document")
        expectExtensions(["xls", "xlsx", "csv", "tsv", "ods"], "spreadsheet")
        expectExtensions(["ppt", "pptx", "odp", "key"], "presentation")
    }

    function test_archive_code_subtitle_and_application_extensions() {
        expectExtensions([
            "zip", "rar", "7z", "tar", "gz", "bz2", "xz", "zst", "tgz",
            "cab"
        ], "archive")
        expectExtensions([
            "rs", "py", "js", "tsx", "jsx", "c", "h", "cpp", "hpp",
            "java", "go", "kt", "sh", "json", "yaml", "yml", "toml", "xml",
            "html", "css", "sql"
        ], "code")
        expectExtensions([
            "srt", "ass", "ssa", "vtt", "sub", "smi", "sup", "pgs", "mks"
        ], "subtitle")
        expectExtensions([
            "exe", "msi", "apk", "deb", "rpm", "appimage", "pkg"
        ], "application")
        expectExtensions(["torrent"], "torrent")
    }

    function test_unknown_and_case_insensitive_extensions() {
        probe.kind = "file"
        probe.fileName = "README"
        compare(probe.iconName, "file")
        probe.fileName = "bundle.unknown"
        compare(probe.iconName, "file")
        probe.fileName = "ARCHIVE.ZIP"
        compare(probe.iconName, "archive")
    }

    function test_every_file_type_has_an_icon_path() {
        var nav = navIconComponent.createObject(parent)
        verify(nav !== null)
        var names = [
            "file", "video", "music", "photo", "pdf", "document",
            "spreadsheet", "presentation", "archive", "code", "subtitle",
            "application", "torrent"
        ]
        for (var i = 0; i < names.length; ++i)
            verify(String(nav.pathData[names[i]] || "").length > 0, names[i])
        nav.destroy()
    }
}
