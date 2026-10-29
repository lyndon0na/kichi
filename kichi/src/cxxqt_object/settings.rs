//! 设置桥接：设置页的读写入口 + 文件列表排序状态 + 窗口几何存盘。
//!
//! QML 侧只读属性做展示，`set*` 方法负责「改 → 持久化 → 通知」。
//! 写盘失败不吞：错误进状态栏，内存里的值仍然生效（本次会话可用）。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use super::qobject;
use crate::settings::{self, Quality, Settings, SortField};

impl qobject::Kichi {
    /// 改清晰度偏好。
    pub fn set_preferred_quality(mut self: Pin<&mut Self>, value: QString) {
        let quality = Quality::parse(&value.to_string());
        self.as_mut().update_settings(|settings| {
            settings.preferred_quality = quality;
        });
    }

    /// 改播放器命令与参数（空串 = 用内置默认）。
    pub fn set_player(mut self: Pin<&mut Self>, command: QString, args: QString) {
        let command = command.to_string();
        let args = args.to_string();
        if let Err(reason) = validate_player(&command) {
            self.as_mut().set_message(&reason, false);
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.player_command = command.trim().to_owned();
            settings.player_args = args.trim().to_owned();
        });
    }

    /// 恢复播放器默认命令与参数。
    pub fn reset_player(mut self: Pin<&mut Self>) {
        self.as_mut().update_settings(|settings| {
            settings.player_command.clear();
            settings.player_args.clear();
        });
    }

    /// 通知开关。
    pub fn set_notify_enabled(mut self: Pin<&mut Self>, enabled: bool) {
        self.as_mut().update_settings(|settings| {
            settings.notify_enabled = enabled;
        });
    }

    /// 播放时自动加载字幕开关。
    pub fn set_auto_subtitles(mut self: Pin<&mut Self>, enabled: bool) {
        self.as_mut().update_settings(|settings| {
            settings.auto_subtitles = enabled;
        });
    }

    /// 关闭到托盘。
    ///
    /// 打开时立刻启托盘，起不来就**保持开关打开但取消勾选的假象**——
    /// 不，更直接：把原因告诉用户并自动关掉这个开关。宁可功能不可用，
    /// 也不能让用户以为「关窗 = 缩到托盘」结果真的退出了。
    pub fn set_close_to_tray(mut self: Pin<&mut Self>, enabled: bool) {
        if enabled && !super::tray::start() {
            {
                let mut rust = self.as_mut().rust_mut();
                rust.tray_available = false;
            }
            self.as_mut().set_message(
                "系统托盘不可用（当前桌面没有托盘或 D-Bus 不支持），「关闭到托盘」已保持关闭",
                false,
            );
            self.as_mut().update_settings(|settings| {
                settings.close_to_tray = false;
            });
            return;
        }
        if !enabled {
            super::tray::stop();
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.tray_available = enabled;
        }
        self.as_mut().update_settings(|settings| {
            settings.close_to_tray = enabled;
        });
    }

    /// 改排序：字段 + 方向。
    pub fn set_sorting(mut self: Pin<&mut Self>, field: QString, ascending: bool) {
        let field = SortField::parse(&field.to_string());
        let unchanged = self.sort_field == field && self.sort_ascending == ascending;
        if unchanged {
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.sort_field = field;
            settings.sort_ascending = ascending;
        });
        // 排序是展示策略：本地重排，不必重新拉网
        self.as_mut().re_sort_entries();
    }

    /// 窗口几何存盘（QML 防抖后调用；最大化时只更新标志）。
    pub fn save_window_geometry(
        mut self: Pin<&mut Self>,
        width: i32,
        height: i32,
        maximized: bool,
    ) {
        let (width, height) = (width.max(0), height.max(0));
        let settings = self.settings.clone();
        let changed = settings.window_maximized != maximized
            || (!maximized && (settings.window_width != width || settings.window_height != height));
        if !changed {
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.window_maximized = maximized;
            // 最大化时保留上一次的普通尺寸，还原窗口才有东西可回
            if !maximized {
                settings.window_width = width;
                settings.window_height = height;
            }
        });
    }

    /// 改下载目录（空串 = 恢复默认 `~/Downloads`）。
    ///
    /// 这里**不执行 mkdir**：只记路径，真正下载时才建目录。若是相对路径就按
    /// 用户实际填的保存，取不到家目录时也能用。
    pub fn set_download_dir(mut self: Pin<&mut Self>, path: QString) {
        let path = path.to_string();
        let trimmed = path.trim().to_owned();
        if trimmed.chars().any(|c| c.is_control()) {
            self.as_mut()
                .set_message("下载目录里不能有换行或控制字符", false);
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.download_dir = trimmed;
        });
    }

    /// 调起系统原生目录选择器，选中后走正常的下载目录保存流程。
    pub fn choose_download_dir(mut self: Pin<&mut Self>) {
        let start = self.settings.download_dir_path();
        let spawned = std::thread::Builder::new()
            .name("kichi-folder-picker".into())
            .spawn(move || match super::folder_picker::choose(&start) {
                Ok(Some(path)) => match path.to_str() {
                    Some(path) => {
                        let path = path.to_owned();
                        super::actions::enqueue_ui(move |mut kichi| {
                            kichi
                                .as_mut()
                                .set_download_dir(QString::from(path.as_str()));
                        });
                    }
                    None => {
                        let reason = format!(
                            "系统选择器返回的目录不是有效 UTF-8，无法保存：{}",
                            path.display()
                        );
                        super::actions::enqueue_ui(move |mut kichi| {
                            kichi.as_mut().set_message(&reason, false);
                        });
                    }
                },
                Ok(None) => {}
                Err(reason) => {
                    super::actions::enqueue_ui(move |mut kichi| {
                        kichi.as_mut().set_message(&reason, false);
                    });
                }
            });
        if let Err(err) = spawned {
            self.as_mut()
                .set_message(&format!("无法启动系统目录选择器：{err}"), false);
        }
    }

    /// 统一的设置改动出口：先改内存、同步属性、再落盘。
    pub(crate) fn update_settings(mut self: Pin<&mut Self>, change: impl FnOnce(&mut Settings)) {
        let snapshot = {
            let mut rust = self.as_mut().rust_mut();
            change(&mut rust.settings);
            rust.settings.clone()
        };
        self.as_mut().sync_settings(&snapshot);
        let path = self.settings_path_buf();
        let outcome = settings::save(&path, &snapshot);
        self.as_mut().notify();
        if let Err(err) = outcome {
            self.as_mut()
                .set_message(&format!("设置未能保存：{err}"), false);
        }
    }

    /// 把设置快照刷到 QML 属性上。
    fn sync_settings(mut self: Pin<&mut Self>, snapshot: &Settings) {
        let (
            quality,
            command,
            args,
            notify,
            auto_subs,
            tray,
            field,
            ascending,
            width,
            height,
            maximized,
        ) = (
            snapshot.preferred_quality.as_str(),
            snapshot.player_command.as_str(),
            snapshot.player_args.as_str(),
            snapshot.notify_enabled,
            snapshot.auto_subtitles,
            snapshot.close_to_tray,
            snapshot.sort_field.as_str(),
            snapshot.sort_ascending,
            snapshot.window_width,
            snapshot.window_height,
            snapshot.window_maximized,
        );
        let download_dir = snapshot.download_dir.clone();
        let mut rust = self.as_mut().rust_mut();
        rust.preferred_quality = QString::from(quality);
        rust.player_command = QString::from(command);
        rust.player_args = QString::from(args);
        rust.notify_enabled = notify;
        rust.auto_subtitles = auto_subs;
        rust.close_to_tray = tray;
        rust.sort_field = snapshot.sort_field;
        rust.sort_ascending = ascending;
        rust.sort_field_text = QString::from(field);
        rust.window_width = width;
        rust.window_height = height;
        rust.window_maximized = maximized;
        rust.download_dir = QString::from(download_dir.as_str());
    }
}

impl qobject::Kichi {
    fn settings_path_buf(&self) -> std::path::PathBuf {
        crate::settings::default_path()
    }
}

/// 播放器命令的轻量校验（不执行、不联网）。
///
/// 空串合法（= 用默认）。含换行等控制字符一律拒绝：命令会被按空格切分，
/// 藏指令没有意义，只会让用户困惑。
fn validate_player(command: &str) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("播放器命令里不能有换行或控制字符".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_command_validation() {
        assert!(validate_player("").is_ok(), "空串表示用默认");
        assert!(validate_player("flatpak run io.mpv.Mpv").is_ok());
        assert!(
            validate_player("mpv\nrm -rf /").is_err(),
            "控制字符必须拒绝"
        );
    }

    #[test]
    fn quality_text_roundtrip() {
        for quality in [
            Quality::Ask,
            Quality::Original,
            Quality::Transcode("1080P"),
            Quality::Transcode("720P"),
            Quality::Transcode("480P"),
        ] {
            assert_eq!(Quality::parse(quality.as_str()), quality, "{quality:?}");
        }
        assert_eq!(SortField::parse(SortField::Size.as_str()), SortField::Size);
    }
}

#[cfg(test)]
mod disk_tests {
    use super::*;

    /// 端到端：改一项设置 -> 文件真的写出来 -> 再读回来一致。
    #[test]
    fn update_persists_to_the_configured_file() {
        let dir = std::env::temp_dir().join(format!("kichi-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录失败");
        let path = dir.join("settings.json");
        // 用环境变量指定路径，模拟真实运行时的 default_path()
        // SAFETY: 单测进程内串行，且只此处读写该变量
        unsafe { std::env::set_var("KICHI_SETTINGS_FILE", &path) };

        let mut settings = Settings::default();
        settings.preferred_quality = Quality::Original;
        settings.sort_field = SortField::Size;
        settings.sort_ascending = false;
        super::settings::save(&super::settings::default_path(), &settings).expect("保存失败");

        let (loaded, warning) = super::settings::load(&super::settings::default_path());
        assert!(warning.is_none());
        assert_eq!(loaded.preferred_quality, Quality::Original);
        assert_eq!(loaded.sort_field, SortField::Size);
        assert!(!loaded.sort_ascending);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
