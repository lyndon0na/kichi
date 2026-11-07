//! 系统原生目录选择器：通过 XDG Desktop Portal 调起。
//!
//! 不能用 `QtQuick.Dialogs` / `Qt.labs.platform`：它们会在当前进程里构造
//! QWidget，而 kichi 是纯 `QGuiApplication`。Portal 的界面由桌面环境的独立
//! 进程提供，因此不会破坏 Qt 的进程模型。

use std::path::{Path, PathBuf};

use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
use ashpd::url::Url;

/// 打开系统目录选择器。
///
/// 返回 `Ok(None)` 表示用户取消，不是错误；Portal 不可用或返回的 URI
/// 不是本地目录时才返回错误文案。
pub(crate) fn choose(start: &Path) -> Result<Option<PathBuf>, String> {
    let request = SelectedFiles::open_file()
        .title("选择下载目录")
        .accept_label("选择此目录")
        .directory(true)
        .modal(true)
        .current_folder(start)
        .map_err(|err| format!("无法设置目录选择器的起始位置：{err}"))?;

    let result = ashpd::zbus::block_on(async { request.send().await?.response() });
    match result {
        Ok(files) => files.uris().first().map(path_from_uri).transpose(),
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => Ok(None),
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Other)) => {
            Err("系统目录选择器没有返回结果".to_owned())
        }
        Err(ashpd::Error::PortalNotFound(_)) => Err(
            "系统未提供目录选择器（请确认 xdg-desktop-portal 及桌面门户后端已安装并运行）"
                .to_owned(),
        ),
        Err(err) => Err(format!("无法打开系统目录选择器：{err}")),
    }
}

/// 打开系统文件选择器，允许多选普通文件。
///
/// 返回空表表示用户取消。Portal 不保证调用方没有传入目录选择模式，因此
/// 真正的“只接受普通文件”由上传入队前的元数据检查负责。
pub(crate) fn choose_files(start: &Path) -> Result<Vec<PathBuf>, String> {
    let request = SelectedFiles::open_file()
        .title("选择要上传的文件")
        .accept_label("上传这些文件")
        .multiple(true)
        .directory(false)
        .modal(true)
        .current_folder(start)
        .map_err(|err| format!("无法设置文件选择器的起始位置：{err}"))?;

    let result = ashpd::zbus::block_on(async { request.send().await?.response() });
    match result {
        Ok(files) => files.uris().iter().map(path_from_uri).collect(),
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => Ok(Vec::new()),
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Other)) => {
            Err("系统文件选择器没有返回结果".to_owned())
        }
        Err(ashpd::Error::PortalNotFound(_)) => Err(
            "系统未提供文件选择器（请确认 xdg-desktop-portal 及桌面门户后端已安装并运行）"
                .to_owned(),
        ),
        Err(err) => Err(format!("无法打开系统文件选择器：{err}")),
    }
}

/// 打开系统文件选择器，只选择 `.torrent` 文件（可多选）。
pub(crate) fn choose_torrent_files(start: &Path) -> Result<Vec<PathBuf>, String> {
    let request = SelectedFiles::open_file()
        .title("选择 BitTorrent 种子文件")
        .accept_label("解析这些文件")
        .multiple(true)
        .directory(false)
        .modal(true)
        .current_folder(start)
        .map_err(|err| format!("无法设置文件选择器的起始位置：{err}"))?
        .filter(
            FileFilter::new("BitTorrent 种子")
                .glob("*.torrent")
                .glob("*.TORRENT"),
        );

    let result = ashpd::zbus::block_on(async { request.send().await?.response() });
    match result {
        Ok(files) => files.uris().iter().map(path_from_uri).collect(),
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => Ok(Vec::new()),
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Other)) => {
            Err("系统文件选择器没有返回结果".to_owned())
        }
        Err(ashpd::Error::PortalNotFound(_)) => Err(
            "系统未提供文件选择器（请确认 xdg-desktop-portal 及桌面门户后端已安装并运行）"
                .to_owned(),
        ),
        Err(err) => Err(format!("无法打开系统文件选择器：{err}")),
    }
}

/// 打开系统目录选择器，选择一个文件夹。
///
/// KDE 的 Portal 目录模式不支持多选；需要一次上传多个文件夹时走文件页拖放。
/// 返回空表表示用户取消。
pub(crate) fn choose_folders(start: &Path) -> Result<Vec<PathBuf>, String> {
    let request = SelectedFiles::open_file()
        .title("选择要上传的文件夹")
        .accept_label("上传此文件夹")
        .multiple(false)
        .directory(true)
        .modal(true)
        .current_folder(start)
        .map_err(|err| format!("无法设置目录选择器的起始位置：{err}"))?;

    let result = ashpd::zbus::block_on(async { request.send().await?.response() });
    match result {
        Ok(files) => files.uris().iter().map(path_from_uri).collect(),
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => Ok(Vec::new()),
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Other)) => {
            Err("系统目录选择器没有返回结果".to_owned())
        }
        Err(ashpd::Error::PortalNotFound(_)) => Err(
            "系统未提供目录选择器（请确认 xdg-desktop-portal 及桌面门户后端已安装并运行）"
                .to_owned(),
        ),
        Err(err) => Err(format!("无法打开系统目录选择器：{err}")),
    }
}

fn path_from_uri(uri: &Url) -> Result<PathBuf, String> {
    if uri.scheme() != "file" {
        return Err(format!("系统选择器返回了非本地路径：{uri}"));
    }
    uri.to_file_path()
        .map_err(|_| format!("无法把系统选择器返回的地址转换为本地路径：{uri}"))
}

/// 把文件管理器拖放得到的 URI 转成本地路径。
///
/// 拖放和 Portal 使用两套不同的错误文案：前者是用户直接拖进来的东西，
/// 后者是系统选择器返回的结果。
pub(crate) fn drop_path_from_uri(uri: &str) -> Result<PathBuf, String> {
    let parsed = Url::parse(uri).map_err(|_| "拖入的不是有效的本地文件地址".to_owned())?;
    if parsed.scheme() != "file" {
        return Err("只能拖入本地文件或文件夹".to_owned());
    }
    parsed
        .to_file_path()
        .map_err(|_| "无法把拖入的地址转换为本地路径".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{drop_path_from_uri, path_from_uri};
    use ashpd::url::Url;

    #[test]
    fn local_uri_becomes_a_path() {
        let uri = Url::parse("file:///tmp/kichi%20downloads").expect("URI");
        assert_eq!(
            path_from_uri(&uri).expect("local path"),
            std::path::PathBuf::from("/tmp/kichi downloads")
        );
    }

    #[test]
    fn non_file_uri_is_rejected() {
        let uri = Url::parse("https://example.com/downloads").expect("URI");
        assert!(path_from_uri(&uri).is_err());
    }

    #[test]
    fn dropped_file_uri_is_decoded_and_non_file_is_rejected() {
        assert_eq!(
            drop_path_from_uri("file:///tmp/%E7%94%B5%E5%BD%B1%20%E7%89%87.mkv").expect("本地路径"),
            std::path::PathBuf::from("/tmp/电影 片.mkv")
        );
        assert_eq!(
            drop_path_from_uri("https://example.com/a.mkv").expect_err("非本地 URI"),
            "只能拖入本地文件或文件夹"
        );
        assert_eq!(
            drop_path_from_uri("not a uri").expect_err("无效 URI"),
            "拖入的不是有效的本地文件地址"
        );
    }
}
