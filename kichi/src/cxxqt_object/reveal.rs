//! 在系统文件管理器里定位一个本地文件，或打开一个下载文件夹。
//!
//! `xdg-open` 只能打开目录；要选中文件走 `org.freedesktop.FileManager1`：
//! `ShowItems([file://…], "")`。Dolphin / Nautilus / Thunar 等实现该接口。
//! 没有 `dbus-send`、没有该服务或调用失败时，退回 `xdg-open` 父目录。
//!
//! 调用全部在后台线程：`dbus-send` 最坏会等一个回复超时，不能卡住界面。

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

enum Outcome {
    /// 文件管理器已经接住
    Done,
    /// 要给用户看的说明（文件不在 / 两边都打不开）
    Message(String),
}

/// 起一个后台线程去定位文件；线程起不来时返回错误，由调用方提示。
pub(crate) fn spawn(path: PathBuf) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("kichi-reveal".into())
        .spawn(move || {
            if let Outcome::Message(text) = reveal(&path) {
                super::actions::enqueue_ui(move |mut kichi| {
                    kichi.as_mut().set_message(&text);
                });
            }
        })
        .map(|_| ())
}

/// 判定顺序：目录 → 直接打开；文件还在 → 选中；选中失败退回父目录；
/// 文件不在 → 提示 + 父目录。
fn reveal(path: &Path) -> Outcome {
    // 文件夹父行的「打开位置」给的是整个文件夹，直接打开它
    if path.is_dir() {
        if open_dir(path) {
            return Outcome::Done;
        }
        return Outcome::Message(format!("无法打开文件夹：{}", path.display()));
    }
    let parent = path.parent().filter(|parent| parent.is_dir());
    if path.is_file() {
        if show_items(path).is_ok() {
            return Outcome::Done;
        }
        if parent.is_some_and(open_dir) {
            return Outcome::Done; // 选中失败但目录打开了，不再打扰用户
        }
        return Outcome::Message(format!("无法打开文件管理器：{}", path.display()));
    }
    // 文件已不在：提示 + 打开父目录（父目录还在时）
    if parent.is_some_and(open_dir) {
        return Outcome::Message(format!(
            "文件已不在：{}（可能已被移动或删除，已打开所在目录）",
            path.display()
        ));
    }
    Outcome::Message(format!(
        "文件已不在：{}（可能已被移动或删除）",
        path.display()
    ))
}

/// `org.freedesktop.FileManager1.ShowItems([file://…], "")`。
fn show_items(path: &Path) -> Result<(), String> {
    let status = Command::new("dbus-send")
        .args([
            "--session",
            // 必须：没有它，服务不存在时也可能退出 0
            "--print-reply",
            "--reply-timeout=3000",
            "--dest=org.freedesktop.FileManager1",
            "--type=method_call",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
        ])
        .arg(format!("array:string:{}", file_uri(path)))
        .arg("string:")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| err.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("dbus-send 退出码 {}", status.code().unwrap_or(-1)))
    }
}

/// 用 `xdg-open` 打开目录；spawn / 退出码失败都返回 false。
fn open_dir(dir: &Path) -> bool {
    Command::new("xdg-open")
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// 本地路径 → `file://` URI（百分号编码；`dbus-send` 的 argument 直接吃）。
pub(super) fn file_uri(path: &Path) -> String {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut uri = String::from("file://");
    for &byte in absolute.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::file_uri;
    use std::path::Path;

    #[test]
    fn plain_path_stays_readable() {
        assert_eq!(
            file_uri(Path::new("/tmp/a.mkv")),
            "file:///tmp/a.mkv",
            "普通路径原样"
        );
    }

    #[test]
    fn spaces_and_chinese_are_percent_encoded() {
        assert_eq!(
            file_uri(Path::new("/tmp/a b.mkv")),
            "file:///tmp/a%20b.mkv",
            "空格"
        );
        assert_eq!(
            file_uri(Path::new("/tmp/第 1 集.mkv")),
            "file:///tmp/%E7%AC%AC%201%20%E9%9B%86.mkv",
            "中文按 UTF-8 逐字节编码"
        );
    }

    #[test]
    fn dbus_and_uri_specials_are_escaped() {
        assert_eq!(file_uri(Path::new("/tmp/a#b")), "file:///tmp/a%23b", "#");
        assert_eq!(file_uri(Path::new("/tmp/a%b")), "file:///tmp/a%25b", "%");
        assert_eq!(
            file_uri(Path::new("/tmp/a,b")),
            "file:///tmp/a%2Cb",
            "逗号是 dbus-send array 的分隔符"
        );
    }
}
