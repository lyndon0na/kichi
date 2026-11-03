//! `.torrent` 预览入口：选择 / 拖入本地种子，解析后把磁力链接交回 QML。
//!
//! 真正的离线任务仍由 `tasks_submit` 在用户确认后统一提交；这里只做本地文件
//! 校验和预测，不访问网络。

use std::path::{Path, PathBuf};
use std::pin::Pin;

use cxx_qt_lib::{QString, QStringList};
use kichi_core::torrent::parse_torrent_file;

use super::actions::{enqueue_ui, format_size};
use super::qobject;
use crate::session::State;

impl qobject::Kichi {
    /// 解析拖入新任务弹窗的本地 URI，逐个发 `torrentParsed` / `torrentParseFailed`。
    pub fn preview_torrent_files(mut self: Pin<&mut Self>, uris: QStringList) {
        if self.backend.session.state() != State::SignedIn {
            self.as_mut().set_message("请先登录再解析 .torrent 文件");
            return;
        }

        let mut paths = Vec::new();
        let mut errors = Vec::new();
        for uri in uris.iter() {
            let uri = uri.to_string();
            if uri.trim().is_empty() {
                continue;
            }
            match super::folder_picker::drop_path_from_uri(&uri) {
                Ok(path) if is_torrent_path(&path) => paths.push(path),
                Ok(path) => errors.push(format!("{}：只能拖入 .torrent 文件", file_label(&path))),
                Err(reason) => errors.push(reason),
            }
        }
        self.as_mut().start_torrent_preview(paths, errors);
    }

    /// 通过 XDG Portal 选择 `.torrent` 文件，解析结果同样回投 QML。
    pub fn choose_torrent_files(mut self: Pin<&mut Self>) {
        if self.backend.session.state() != State::SignedIn {
            self.as_mut().set_message("请先登录再解析 .torrent 文件");
            return;
        }
        let start = self.settings.download_dir_path();
        let spawned = std::thread::Builder::new()
            .name("kichi-torrent-picker".into())
            .spawn(
                move || match super::folder_picker::choose_torrent_files(&start) {
                    Ok(paths) => {
                        enqueue_ui(move |mut qichi| {
                            if qichi.backend.session.state() != State::SignedIn {
                                return;
                            }
                            let mut valid = Vec::new();
                            let mut errors = Vec::new();
                            for path in paths {
                                if is_torrent_path(&path) {
                                    valid.push(path);
                                } else {
                                    errors.push(format!(
                                        "{}：只能选择 .torrent 文件",
                                        file_label(&path)
                                    ));
                                }
                            }
                            qichi.as_mut().start_torrent_preview(valid, errors);
                        });
                    }
                    Err(reason) => {
                        enqueue_ui(move |mut qichi| {
                            if qichi.backend.session.state() != State::SignedIn {
                                return;
                            }
                            qichi
                                .as_mut()
                                .torrent_parse_failed(QString::from(reason.as_str()));
                        });
                    }
                },
            );
        if let Err(err) = spawned {
            self.as_mut()
                .set_message(&format!("无法启动系统文件选择器：{err}"));
        }
    }

    fn start_torrent_preview(
        mut self: Pin<&mut Self>,
        paths: Vec<PathBuf>,
        mut errors: Vec<String>,
    ) {
        if paths.is_empty() {
            for error in errors {
                self.as_mut()
                    .torrent_parse_failed(QString::from(error.as_str()));
            }
            return;
        }

        let spawned = std::thread::Builder::new()
            .name("kichi-torrent-parse".into())
            .spawn(move || {
                let mut parsed = Vec::new();
                for path in paths {
                    match parse_torrent_file(&path) {
                        Ok(info) => {
                            let detail = torrent_detail(info.file_count, info.total_size);
                            let magnet = info.magnet();
                            parsed.push((info.name, magnet, detail));
                        }
                        Err(err) => errors.push(format!("{}：{err}", file_label(&path))),
                    }
                }
                enqueue_ui(move |mut qichi| {
                    if qichi.backend.session.state() != State::SignedIn {
                        return;
                    }
                    for (name, magnet, detail) in parsed {
                        qichi.as_mut().torrent_parsed(
                            QString::from(name.as_str()),
                            QString::from(magnet.as_str()),
                            QString::from(detail.as_str()),
                        );
                    }
                    for error in errors {
                        qichi
                            .as_mut()
                            .torrent_parse_failed(QString::from(error.as_str()));
                    }
                });
            });
        if let Err(err) = spawned {
            self.as_mut()
                .torrent_parse_failed(QString::from(format!("无法解析 .torrent：{err}").as_str()));
        }
    }
}

fn torrent_detail(file_count: usize, total_size: u64) -> String {
    let kind = if file_count > 1 {
        format!("{file_count} 个文件")
    } else {
        "单文件".to_owned()
    };
    format!("{kind} · {}", format_size(total_size))
}

fn is_torrent_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(".torrent 文件")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::torrent_detail;

    #[test]
    fn detail_reports_file_count_and_size() {
        assert_eq!(torrent_detail(1, 1024), "单文件 · 1 KB");
        assert_eq!(torrent_detail(3, 2 * 1024 * 1024), "3 个文件 · 2.0 MB");
    }
}
