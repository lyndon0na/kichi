//! 按官方分类 / MIME / 扩展名判断媒体类型（列表接口常不下发分类字段）。

pub(crate) const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "webm", "ts", "m2ts", "m4v", "flv", "wmv", "rmvb", "rm", "mpg",
    "mpeg", "3gp", "ogv",
];

/// 音频扩展名表：能播放，但不尝试挂外挂字幕。
pub(crate) const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "m4a", "ogg", "opus", "aac", "ape", "wma",
];

/// 图片扩展名表：只有图片和视频的列表缩略图会进入本地缓存。
pub(crate) const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "avif", "heic", "heif", "tif", "tiff",
];

fn extension_matches(
    name: &str,
    category: &str,
    mime: &str,
    extension: &str,
    kind: &str,
    allowed: &[&str],
) -> bool {
    let from_extension = |text: &str| {
        let ext = text.trim().trim_start_matches('.').to_ascii_lowercase();
        allowed.contains(&ext.as_str())
    };
    if !extension.is_empty() && from_extension(extension) {
        return true;
    }
    if name
        .rsplit_once('.')
        .is_some_and(|(_, ext)| from_extension(ext))
    {
        return true;
    }
    category.trim().eq_ignore_ascii_case(kind)
        || mime
            .trim()
            .to_ascii_lowercase()
            .starts_with(&format!("{}/", kind.to_ascii_lowercase()))
}

pub(crate) fn is_video(name: &str, category: &str, mime: &str, extension: &str) -> bool {
    extension_matches(name, category, mime, extension, "VIDEO", VIDEO_EXTENSIONS)
}

pub(super) fn is_audio(name: &str, category: &str, mime: &str, extension: &str) -> bool {
    extension_matches(name, category, mime, extension, "AUDIO", AUDIO_EXTENSIONS)
}

pub(super) fn is_image(name: &str, category: &str, mime: &str, extension: &str) -> bool {
    extension_matches(name, category, mime, extension, "IMAGE", IMAGE_EXTENSIONS)
}

/// 是否按媒体处理（决定是否显示播放入口）。
///
/// **实测（2026-10-26）**：文件列表接口在不带 `kind` 过滤时，
/// `file_category` / `mime_type` / `file_extension` 全是空串（带 `kind`
/// 过滤才下发），因此不能只信官方分类，必须按扩展名兜底，否则视频文件
/// 在界面上没有播放按钮。官方分类可用时优先采信。
pub(crate) fn is_media(name: &str, category: &str, mime: &str, extension: &str) -> bool {
    is_video(name, category, mime, extension) || is_audio(name, category, mime, extension)
}

/// 只按文件名判断是否媒体（本地索引没有官方分类字段时用）。
pub fn is_media_name(name: &str) -> bool {
    is_media(name, "", "", "")
}

/// 只按文件名判断是否视频（本地索引没有官方分类字段时用）。
pub fn is_video_name(name: &str) -> bool {
    is_video(name, "", "", "")
}

/// 只按文件名判断是否图片（快照 / 索引条目探测本地缩略图缓存时用）。
pub fn is_image_name(name: &str) -> bool {
    is_image(name, "", "", "")
}
