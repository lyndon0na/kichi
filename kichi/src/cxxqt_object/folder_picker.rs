//! 系统原生目录选择器：通过 XDG Desktop Portal 调起。
//!
//! 不能用 `QtQuick.Dialogs` / `Qt.labs.platform`：它们会在当前进程里构造
//! QWidget，而 kichi 是纯 `QGuiApplication`。Portal 的界面由桌面环境的独立
//! 进程提供，因此不会破坏 Qt 的进程模型。

use std::path::{Path, PathBuf};

use ashpd::desktop::file_chooser::SelectedFiles;
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
            "系统未提供目录选择器（请确认 xdg-desktop-portal 和 KDE 后端已安装并运行）".to_owned(),
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
            "系统未提供文件选择器（请确认 xdg-desktop-portal 和 KDE 后端已安装并运行）".to_owned(),
        ),
        Err(err) => Err(format!("无法打开系统文件选择器：{err}")),
    }
}

fn path_from_uri(uri: &Url) -> Result<PathBuf, String> {
    if uri.scheme() != "file" {
        return Err(format!("系统选择器返回了非本地路径：{uri}"));
    }
    uri.to_file_path()
        .map_err(|_| format!("无法把系统选择器返回的地址转换为本地路径：{uri}"))
}

#[cfg(test)]
mod tests {
    use super::path_from_uri;
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
}
