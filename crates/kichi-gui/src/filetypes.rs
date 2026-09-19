//! 文件类型分类单点: 图标、mpv 播放、系统打开与「只下载」判定共用同一份表。

use eframe::egui::Color32;
use kichi_core::types::File;

use crate::icons::Glyph;

/// 文件类型分类。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FileType {
    Folder,
    Video,
    Audio,
    Image,
    Document,
    Archive,
    Executable,
    Torrent,
    Other,
}

/// 可用 mpv 播放的视频扩展名(小写)。
const VIDEO_EXTS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "ts", "rmvb", "m4v", "m2ts", "mpg", "mpeg",
];

/// 可用 mpv 播放的音频扩展名(小写)。
const AUDIO_EXTS: &[&str] = &["mp3", "flac", "wav", "aac", "ogg", "m4a", "opus", "ape"];

/// 常见图片扩展名(小写)。
const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "svg", "tiff",
];

/// 常见文档扩展名(小写)。
const DOC_EXTS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "epub", "csv",
];

/// 常见字幕扩展名(小写)。
const SUBTITLE_EXTS: &[&str] = &["ass", "ssa", "srt", "sub", "vtt", "sbv", "sup"];

/// 压缩包 / 镜像扩展名(小写)。
const ARCHIVE_EXTS: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "bz2", "xz", "zst", "iso", "img", "dmg",
];

/// 可执行 / 安装包扩展名(小写)。
const EXEC_EXTS: &[&str] = &["exe", "msi", "deb", "rpm", "apk", "appimage", "run", "bin"];

/// 种子扩展名(小写)。
const TORRENT_EXTS: &[&str] = &["torrent"];

/// 取小写扩展名(最后一段), 无扩展名时为空串。
fn ext_lower(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e): (&str, &str)| e.to_lowercase())
        .unwrap_or_default()
}

fn from_ext(ext: &str) -> FileType {
    if VIDEO_EXTS.contains(&ext) {
        FileType::Video
    } else if AUDIO_EXTS.contains(&ext) {
        FileType::Audio
    } else if IMAGE_EXTS.contains(&ext) {
        FileType::Image
    } else if DOC_EXTS.contains(&ext) || SUBTITLE_EXTS.contains(&ext) {
        FileType::Document
    } else if ARCHIVE_EXTS.contains(&ext) {
        FileType::Archive
    } else if EXEC_EXTS.contains(&ext) {
        FileType::Executable
    } else if TORRENT_EXTS.contains(&ext) {
        FileType::Torrent
    } else {
        FileType::Other
    }
}

/// 强 mime 才参与判定; `None` 表示无信息(通用二进制 / 下载占位 mime), 应回退扩展名表。
fn from_mime(mime: &str) -> Option<FileType> {
    let m = mime.trim().to_ascii_lowercase();
    if m.is_empty()
        || matches!(
            m.as_str(),
            "application/octet-stream"
                | "binary/octet-stream"
                | "application/x-download"
                | "application/force-download"
                | "application/unknown"
        )
    {
        return None;
    }
    if m.starts_with("video/") {
        return Some(FileType::Video);
    }
    if m.starts_with("audio/") {
        return Some(FileType::Audio);
    }
    if m.starts_with("image/") {
        return Some(FileType::Image);
    }
    if m.starts_with("text/") {
        return Some(FileType::Document);
    }
    match m.as_str() {
        "application/pdf"
        | "application/rtf"
        | "application/epub+zip"
        | "application/typescript"
        | "application/msword"
        | "application/vnd.ms-excel"
        | "application/vnd.ms-powerpoint"
        | "application/x-subrip" => Some(FileType::Document),
        "application/zip"
        | "application/x-zip-compressed"
        | "application/x-rar"
        | "application/x-rar-compressed"
        | "application/vnd.rar"
        | "application/x-7z-compressed"
        | "application/x-tar"
        | "application/gzip"
        | "application/x-gzip"
        | "application/x-bzip2"
        | "application/x-xz"
        | "application/x-zstd"
        | "application/x-iso9660-image"
        | "application/x-apple-diskimage" => Some(FileType::Archive),
        "application/x-msdownload"
        | "application/x-dosexec"
        | "application/x-msi"
        | "application/vnd.microsoft.portable-executable"
        | "application/x-executable"
        | "application/x-pie-executable"
        | "application/x-sharedlib"
        | "application/x-mach-binary"
        | "application/vnd.android.package-archive"
        | "application/x-deb"
        | "application/x-debian-package"
        | "application/x-rpm" => Some(FileType::Executable),
        "application/x-bittorrent" => Some(FileType::Torrent),
        _ if m.starts_with("application/vnd.openxmlformats-officedocument.")
            || m.starts_with("application/vnd.oasis.opendocument.")
            || m.starts_with("application/vnd.ms-") =>
        {
            Some(FileType::Document)
        }
        _ => None,
    }
}

/// 按「强 mime 优先, 否则扩展名」分类文件名。
pub(crate) fn classify(name: &str, mime: Option<&str>) -> FileType {
    if let Some(ft) = mime.and_then(from_mime) {
        return ft;
    }
    from_ext(&ext_lower(name))
}

/// 分类一个云端文件条目。
pub(crate) fn classify_file(f: &File) -> FileType {
    if f.is_folder() {
        return FileType::Folder;
    }
    classify(&f.name, f.mime_type.as_deref())
}

/// 类型 -> 图标 / 颜色。
pub(crate) fn glyph_color(ft: FileType) -> (Glyph, Color32) {
    match ft {
        FileType::Folder => (Glyph::Folder, Color32::from_rgb(232, 178, 84)),
        FileType::Video => (Glyph::Video, Color32::from_rgb(196, 130, 220)),
        FileType::Audio => (Glyph::Audio, Color32::from_rgb(104, 196, 136)),
        FileType::Image => (Glyph::Image, Color32::from_rgb(96, 184, 200)),
        FileType::Document => (Glyph::Doc, Color32::from_rgb(214, 178, 96)),
        FileType::Archive => (Glyph::Archive, Color32::from_rgb(208, 142, 110)),
        FileType::Executable | FileType::Torrent | FileType::Other => {
            (Glyph::File, Color32::from_rgb(120, 126, 140))
        }
    }
}

/// 文件条目 -> 图标 / 颜色。
pub(crate) fn file_visual(f: &File) -> (Glyph, Color32) {
    glyph_color(classify_file(f))
}

/// 判断文件名是否为常见字幕格式。
pub(crate) fn is_subtitle(name: &str) -> bool {
    SUBTITLE_EXTS.contains(&ext_lower(name).as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_media_by_ext() {
        assert_eq!(classify("movie.MKV", None), FileType::Video);
        assert_eq!(classify("clip.m2ts", None), FileType::Video);
        assert_eq!(classify("song.flac", None), FileType::Audio);
    }

    #[test]
    fn strong_mime_overrides_ext() {
        // .ts 既是 MPEG-TS 也是 TypeScript: mime 说了算。
        assert_eq!(classify("app.ts", Some("video/mp2t")), FileType::Video);
        assert_eq!(classify("app.ts", Some("text/plain")), FileType::Document);
        assert_eq!(
            classify("app.ts", Some("application/typescript")),
            FileType::Document
        );
        // mime 救回改名成 .bin / 无扩展名的视频。
        assert_eq!(classify("movie.bin", Some("video/mp4")), FileType::Video);
        assert_eq!(classify("noext", Some("video/x-matroska")), FileType::Video);
        assert_eq!(classify("noext", Some("image/png")), FileType::Image);
    }

    #[test]
    fn generic_mime_falls_back_to_ext() {
        for generic in [
            None,
            Some(""),
            Some("application/octet-stream"),
            Some("binary/octet-stream"),
        ] {
            assert_eq!(classify("movie.mkv", generic), FileType::Video);
            assert_eq!(classify("a.zip", generic), FileType::Archive);
        }
        // 无 mime 的 .ts 按扩展名归类。
        assert_eq!(classify("stream.ts", None), FileType::Video);
        assert_eq!(
            classify("stream.ts", Some("application/octet-stream")),
            FileType::Video
        );
    }

    #[test]
    fn classifies_non_media() {
        for name in ["a.zip", "a.iso", "a.exe", "a.apk", "a.torrent", "a.img"] {
            assert!(matches!(
                classify(name, None),
                FileType::Archive | FileType::Executable | FileType::Torrent
            ));
        }
        for name in ["a.pdf", "a.docx", "a.txt", "a.ass", "a.jpg"] {
            assert!(matches!(
                classify(name, None),
                FileType::Document | FileType::Image
            ));
        }
        // 未知 / 冷门扩展名归 Other, 不是媒体。
        for name in ["a.psd", "a.wps", "a.dat", "noext", "name."] {
            assert_eq!(classify(name, None), FileType::Other, "{name}");
        }
        // mime 侧同样生效。
        assert_eq!(
            classify("noext", Some("application/x-iso9660-image")),
            FileType::Archive
        );
        assert_eq!(
            classify(
                "noext",
                Some("application/vnd.microsoft.portable-executable")
            ),
            FileType::Executable
        );
    }

    #[test]
    fn icon_matches_media_routing() {
        // m2ts / mpg / mpeg 曾经图标是视频但判定不是媒体, 走整份下载。
        for name in ["a.m2ts", "a.mpg", "a.mpeg", "a.mp4"] {
            let ft = classify(name, None);
            assert_eq!(ft, FileType::Video, "{name}");
            assert_eq!(glyph_color(ft).0, Glyph::Video, "{name}");
        }
    }

    #[test]
    fn subtitle_detection() {
        assert!(is_subtitle("x.ass"));
        assert!(is_subtitle("x.SRT"));
        assert!(is_subtitle("x.zh-CN.vtt"));
        assert!(!is_subtitle("x.mkv"));
        assert!(!is_subtitle("x.txt"));
        assert_eq!(classify("x.srt", None), FileType::Document);
    }

    #[test]
    fn folders_are_folders() {
        let f = File {
            kind: "drive#folder".to_string(),
            name: "dir".to_string(),
            ..Default::default()
        };
        assert_eq!(classify_file(&f), FileType::Folder);
        let g = File {
            kind: "drive#file".to_string(),
            name: "clip.mp4".to_string(),
            mime_type: Some("video/mp4".to_string()),
            ..Default::default()
        };
        assert_eq!(classify_file(&g), FileType::Video);
    }
}
