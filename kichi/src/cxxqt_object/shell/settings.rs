//! 设置桥接：设置页的读写入口 + 文件列表排序状态 + 窗口几何存盘。
//!
//! QML 侧只读属性做展示，`set*` 方法负责「改 → 持久化 → 通知」。
//! 写盘失败不吞：错误进状态栏，内存里的值仍然生效（本次会话可用）。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use super::qobject;
use crate::session::limits;
use crate::settings::{
    self, ContentViewMode, PlayerBackend, PreferredLanguage, Quality, Settings, SortField,
};

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
            self.as_mut().set_message(&reason);
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.player_backend = PlayerBackend::Custom;
            settings.player_command = command.trim().to_owned();
            settings.player_args = args.trim().to_owned();
        });
    }

    /// 切换到预设播放器，或只打开自定义项（命令仍由 set_player 保存）。
    pub fn set_player_backend(mut self: Pin<&mut Self>, value: QString) {
        let backend = PlayerBackend::parse(&value.to_string());
        if self.player_backend.to_string() == backend.as_str() {
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.player_backend = backend;
        });
    }

    /// 恢复默认 mpv，并清掉自定义命令。
    pub fn reset_player(mut self: Pin<&mut Self>) {
        self.as_mut().update_settings(|settings| {
            settings.player_backend = PlayerBackend::Mpv;
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

    /// 内嵌播放首选音轨语言（下次开播生效）。
    pub fn set_player_audio_lang(mut self: Pin<&mut Self>, value: QString) {
        let language = PreferredLanguage::parse(&value.to_string());
        if self.settings.player_audio_lang == language {
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.player_audio_lang = language;
        });
    }

    /// 内嵌播放首选字幕语言（下次开播生效）。
    pub fn set_player_sub_lang(mut self: Pin<&mut Self>, value: QString) {
        let language = PreferredLanguage::parse(&value.to_string());
        if self.settings.player_sub_lang == language {
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.player_sub_lang = language;
        });
    }

    /// 内嵌播放是否读取 embed-mpv.conf；打开时顺手生成模板，方便用户编辑。
    pub fn set_player_mpv_conf(mut self: Pin<&mut Self>, enabled: bool) {
        if enabled {
            let path = crate::session::paths::embed_mpv_conf_path();
            if let Err(err) = crate::player::embed::conf::ensure_file(&path) {
                self.as_mut()
                    .set_message(&format!("无法创建 embed-mpv.conf：{err}"));
                return;
            }
        }
        self.as_mut().update_settings(|settings| {
            settings.player_mpv_conf = enabled;
        });
    }

    /// 用系统默认编辑器打开 embed-mpv.conf；文件不存在时先生成模板。
    pub fn open_player_mpv_conf(mut self: Pin<&mut Self>) {
        let path = crate::session::paths::embed_mpv_conf_path();
        if let Err(err) = crate::player::embed::conf::ensure_file(&path) {
            self.as_mut()
                .set_message(&format!("无法创建 embed-mpv.conf：{err}"));
            return;
        }
        self.open_in_editor(&path, "embed-mpv.conf");
    }

    /// 内嵌播放是否加载 embed-input.conf；打开时顺手生成模板，方便用户编辑。
    pub fn set_player_input_conf(mut self: Pin<&mut Self>, enabled: bool) {
        if enabled {
            let path = crate::session::paths::embed_input_conf_path();
            if let Err(err) = crate::player::embed::conf::ensure_input_file(&path) {
                self.as_mut()
                    .set_message(&format!("无法创建 embed-input.conf：{err}"));
                return;
            }
        }
        self.as_mut().update_settings(|settings| {
            settings.player_input_conf = enabled;
        });
    }

    /// 内嵌播放是否使用 Vulkan 渲染；关闭后回退 OpenGL，下次开播生效。
    pub fn set_player_vulkan(mut self: Pin<&mut Self>, enabled: bool) {
        self.as_mut().update_settings(|settings| {
            settings.player_vulkan = enabled;
        });
    }

    /// 用系统默认编辑器打开 embed-input.conf；文件不存在时先生成模板。
    pub fn open_player_input_conf(mut self: Pin<&mut Self>) {
        let path = crate::session::paths::embed_input_conf_path();
        if let Err(err) = crate::player::embed::conf::ensure_input_file(&path) {
            self.as_mut()
                .set_message(&format!("无法创建 embed-input.conf：{err}"));
            return;
        }
        self.open_in_editor(&path, "embed-input.conf");
    }

    fn open_in_editor(mut self: Pin<&mut Self>, path: &std::path::Path, label: &str) {
        let spawned = std::process::Command::new("xdg-open")
            .arg(path)
            // 内嵌开播期间 arm 了一次性 WAYLAND_SOCKET：编辑器的 libwayland 不能继承。
            .env_remove("WAYLAND_SOCKET")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Err(err) = spawned {
            self.as_mut()
                .set_message(&format!("打开 {label} 失败：{err}"));
        }
    }

    /// 字幕字号；正在播放时立即预览。
    pub fn set_subtitle_font_size(mut self: Pin<&mut Self>, value: i32) {
        let value = clamp_u16(
            value,
            settings::SUB_FONT_SIZE_MIN,
            settings::SUB_FONT_SIZE_MAX,
        );
        self.as_mut().update_settings(|settings| {
            settings.player_sub_font_size = value;
        });
        apply_embed_subtitle_style(&self.settings);
    }

    /// 字幕垂直位置（0 顶部 / 100 底部）；正在播放时立即预览。
    pub fn set_subtitle_position(mut self: Pin<&mut Self>, value: i32) {
        let value = clamp_u16(value, 0, settings::SUB_POS_MAX);
        self.as_mut().update_settings(|settings| {
            settings.player_sub_pos = value;
        });
        apply_embed_subtitle_style(&self.settings);
    }

    /// 字幕文字颜色；非法值直接忽略（界面只给固定色板）。
    pub fn set_subtitle_color(mut self: Pin<&mut Self>, value: QString) {
        let value = value.to_string();
        if !settings::valid_color(&value) {
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.player_sub_color = value;
        });
        apply_embed_subtitle_style(&self.settings);
    }

    /// 字幕描边粗细；正在播放时立即预览。
    pub fn set_subtitle_outline_size(mut self: Pin<&mut Self>, value: i32) {
        let value = clamp_u16(value, 0, settings::SUB_OUTLINE_SIZE_MAX);
        self.as_mut().update_settings(|settings| {
            settings.player_sub_outline_size = value;
        });
        apply_embed_subtitle_style(&self.settings);
    }

    /// 字幕描边 / 边框颜色；正在播放时立即预览。
    pub fn set_subtitle_outline_color(mut self: Pin<&mut Self>, value: QString) {
        let value = value.to_string();
        if !settings::valid_color(&value) {
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.player_sub_outline_color = value;
        });
        apply_embed_subtitle_style(&self.settings);
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

    /// 切换文件类内容页共用的显示方式。只影响展示，不重新请求数据。
    pub fn set_content_view(mut self: Pin<&mut Self>, value: QString) {
        let mode = ContentViewMode::parse(&value.to_string());
        if self.content_view == mode {
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.content_view = mode;
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
            self.as_mut().set_message("下载目录里不能有换行或控制字符");
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.download_dir = trimmed;
        });
    }

    /// 同时下载的文件数（跨批与文件夹批内共用）。
    pub fn set_download_parallel(mut self: Pin<&mut Self>, value: i32) {
        let value = usize::try_from(value)
            .unwrap_or(limits::DEFAULT_PARALLEL)
            .clamp(1, limits::MAX_PARALLEL);
        self.as_mut().update_settings(|settings| {
            settings.download_parallel = value;
        });
        self.as_mut().apply_download_limits();
        // 调高并发后立即填满新增的空闲槽。
        self.as_mut().advance_downloads();
    }

    /// 单个文件的分片连接数；1 = 不分片。
    pub fn set_download_connections(mut self: Pin<&mut Self>, value: i32) {
        let value = usize::try_from(value)
            .unwrap_or(limits::DEFAULT_CONNECTIONS)
            .clamp(1, limits::MAX_CONNECTIONS);
        self.as_mut().update_settings(|settings| {
            settings.download_connections = value;
        });
        self.as_mut().apply_download_limits();
    }

    /// 全局限速 MiB/s；0 = 不限。
    pub fn set_download_rate_mib(mut self: Pin<&mut Self>, value: i32) {
        let value = u64::try_from(value).unwrap_or(0).min(limits::MAX_RATE_MIB);
        self.as_mut().update_settings(|settings| {
            settings.download_rate_mib = value;
        });
        self.as_mut().apply_download_limits();
    }

    /// 同时上传的文件数；单文件任务跨批共用，文件夹批次独占后批内共用。
    pub fn set_upload_parallel(mut self: Pin<&mut Self>, value: i32) {
        let value = usize::try_from(value)
            .unwrap_or(limits::DEFAULT_UPLOAD_PARALLEL)
            .clamp(1, limits::MAX_UPLOAD_PARALLEL);
        self.as_mut().update_settings(|settings| {
            settings.upload_parallel = value;
        });
        self.as_mut().apply_upload_limits();
        // 调高并发后立即填满新增的空闲槽。
        self.as_mut().advance_uploads();
    }

    /// 上传全局限速 MiB/s；0 = 不限。
    pub fn set_upload_rate_mib(mut self: Pin<&mut Self>, value: i32) {
        let value = u64::try_from(value)
            .unwrap_or(0)
            .min(limits::MAX_UPLOAD_RATE_MIB);
        self.as_mut().update_settings(|settings| {
            settings.upload_rate_mib = value;
        });
        self.as_mut().apply_upload_limits();
    }

    /// 正常退出后，下次恢复登录态时是否自动继续未完成传输。
    pub fn set_auto_resume_downloads(mut self: Pin<&mut Self>, enabled: bool) {
        self.as_mut().update_settings(|settings| {
            settings.auto_resume_downloads = enabled;
        });
    }

    /// 正常退出后是否自动继续未完成的上传；默认关闭。
    pub fn set_auto_resume_uploads(mut self: Pin<&mut Self>, enabled: bool) {
        self.as_mut().update_settings(|settings| {
            settings.auto_resume_uploads = enabled;
        });
    }

    /// 本地目录索引 / 页面快照开关。
    ///
    /// 关闭：取消在途建库与探针、放开库句柄，之后所有读写入口都不再碰本地库
    /// （库文件保留）。打开：复用已有库，空库补建、已有库顺手对一次账。
    pub fn set_local_index_enabled(mut self: Pin<&mut Self>, enabled: bool) {
        if self.local_index_enabled == enabled {
            return;
        }
        self.backend.session.set_local_index_enabled(enabled);
        if enabled {
            self.as_mut().update_settings(|settings| {
                settings.local_index_enabled = true;
            });
            self.as_mut().rust_mut().index_error_reported = false;
            self.as_mut().ensure_tree_index_build();
            return;
        }
        let was_busy = {
            let mut rust = self.as_mut().rust_mut();
            let busy = rust.index_building;
            if let Some(cancel) = rust.index_build_cancel.take() {
                cancel.cancel();
            }
            rust.index_building = false;
            rust.index_build_scanned = 0;
            rust.index_scan_tap = None;
            rust.local_index_ready = false;
            rust.index_build_epoch = rust.index_build_epoch.wrapping_add(1);
            if let Some(cancel) = rust.index_probe_cancel.take() {
                cancel.cancel();
            }
            rust.index_probing = false;
            rust.index_probe_epoch = rust.index_probe_epoch.wrapping_add(1);
            busy
        };
        self.as_mut().update_settings(|settings| {
            settings.local_index_enabled = false;
        });
        if was_busy {
            // 建库和全盘搜索共用一次遍历：关掉索引就是唯一的「中断」入口，
            // 已扫出的结果保留，但搜索状态必须收尾，别让转圈一直挂着。
            self.as_mut().settle_search(
                "本地索引已关闭，扫描已停止；当前结果只来自已扫出的条目".to_owned(),
            );
            self.as_mut().set_message("");
        }
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
                            kichi.as_mut().set_message(&reason);
                        });
                    }
                },
                Ok(None) => {}
                Err(reason) => {
                    super::actions::enqueue_ui(move |mut kichi| {
                        kichi.as_mut().set_message(&reason);
                    });
                }
            });
        if let Err(err) = spawned {
            self.as_mut()
                .set_message(&format!("无法启动系统目录选择器：{err}"));
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
            self.as_mut().set_message(&format!("设置未能保存：{err}"));
        }
    }

    /// 把设置快照刷到 QML 属性上。
    fn sync_settings(mut self: Pin<&mut Self>, snapshot: &Settings) {
        let (
            quality,
            player_backend,
            command,
            args,
            notify,
            auto_subs,
            tray,
            content_view,
            field,
            ascending,
            width,
            height,
            maximized,
        ) = (
            snapshot.preferred_quality.as_str(),
            snapshot.player_backend.as_str(),
            snapshot.player_command.as_str(),
            snapshot.player_args.as_str(),
            snapshot.notify_enabled,
            snapshot.auto_subtitles,
            snapshot.close_to_tray,
            snapshot.content_view,
            snapshot.sort_field.as_str(),
            snapshot.sort_ascending,
            snapshot.window_width,
            snapshot.window_height,
            snapshot.window_maximized,
        );
        let download_dir = snapshot.download_dir.clone();
        let mut rust = self.as_mut().rust_mut();
        rust.preferred_quality = QString::from(quality);
        rust.player_backend = QString::from(player_backend);
        rust.player_command = QString::from(command);
        rust.player_args = QString::from(args);
        rust.notify_enabled = notify;
        rust.auto_subtitles = auto_subs;
        rust.player_audio_lang = QString::from(snapshot.player_audio_lang.as_str());
        rust.player_sub_lang = QString::from(snapshot.player_sub_lang.as_str());
        rust.player_mpv_conf = snapshot.player_mpv_conf;
        rust.player_input_conf = snapshot.player_input_conf;
        rust.player_vulkan = snapshot.player_vulkan;
        rust.embed_volume = snapshot.player_volume;
        rust.embed_muted = snapshot.player_muted;
        rust.embed_sub_font_size = i32::from(snapshot.player_sub_font_size);
        rust.embed_sub_pos = i32::from(snapshot.player_sub_pos);
        rust.embed_sub_color = QString::from(snapshot.player_sub_color.as_str());
        rust.embed_sub_outline_size = i32::from(snapshot.player_sub_outline_size);
        rust.embed_sub_outline_color = QString::from(snapshot.player_sub_outline_color.as_str());
        rust.close_to_tray = tray;
        rust.content_view = content_view;
        rust.content_view_text = QString::from(content_view.as_str());
        rust.sort_field = snapshot.sort_field;
        rust.sort_ascending = ascending;
        rust.sort_field_text = QString::from(field);
        rust.window_width = width;
        rust.window_height = height;
        rust.window_maximized = maximized;
        rust.download_dir = QString::from(download_dir.as_str());
        rust.download_parallel =
            i32::try_from(snapshot.download_parallel).unwrap_or(limits::DEFAULT_PARALLEL as i32);
        rust.download_connections = i32::try_from(snapshot.download_connections)
            .unwrap_or(limits::DEFAULT_CONNECTIONS as i32);
        rust.download_rate_mib = i32::try_from(snapshot.download_rate_mib).unwrap_or(i32::MAX);
        rust.auto_resume_downloads = snapshot.auto_resume_downloads;
        rust.auto_resume_uploads = snapshot.auto_resume_uploads;
        rust.local_index_enabled = snapshot.local_index_enabled;
        rust.upload_parallel = i32::try_from(snapshot.upload_parallel)
            .unwrap_or(limits::DEFAULT_UPLOAD_PARALLEL as i32);
        rust.upload_rate_mib = i32::try_from(snapshot.upload_rate_mib).unwrap_or(i32::MAX);
    }
}

impl qobject::Kichi {
    fn settings_path_buf(&self) -> std::path::PathBuf {
        crate::settings::default_path()
    }

    pub(super) fn apply_download_limits(&self) {
        limits::configure(
            self.settings.download_parallel,
            self.settings.download_connections,
            self.settings.download_rate_mib,
        );
    }

    pub(super) fn apply_upload_limits(&self) {
        limits::configure_uploads(self.settings.upload_parallel, self.settings.upload_rate_mib);
    }
}

fn clamp_u16(value: i32, min: u16, max: u16) -> u16 {
    u16::try_from(value).unwrap_or(min).clamp(min, max)
}

/// 把设置里的字幕样式立刻交给正在播放的内嵌会话（没有会话时静默忽略）。
fn apply_embed_subtitle_style(settings: &Settings) {
    crate::player::embed::set_subtitle_style(crate::player::embed::SubtitleStyle::new(
        i32::from(settings.player_sub_font_size),
        i32::from(settings.player_sub_pos),
        &settings.player_sub_color,
        i32::from(settings.player_sub_outline_size),
        &settings.player_sub_outline_color,
    ));
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

        let settings = Settings {
            preferred_quality: Quality::Original,
            sort_field: SortField::Size,
            sort_ascending: false,
            ..Settings::default()
        };
        super::settings::save(&super::settings::default_path(), &settings).expect("保存失败");

        let (loaded, warning) = super::settings::load(&super::settings::default_path());
        assert!(warning.is_none());
        assert_eq!(loaded.preferred_quality, Quality::Original);
        assert_eq!(loaded.sort_field, SortField::Size);
        assert!(!loaded.sort_ascending);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
