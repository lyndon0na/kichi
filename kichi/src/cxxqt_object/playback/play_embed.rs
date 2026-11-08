//! 内嵌播放器的 QML 接口：浮层控制、轨道与退出收尾。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};

use super::qobject;
use super::QVariantList;
use crate::player;

type QVariantMap = QMap<QMapPair_QString_QVariant>;

impl qobject::Kichi {
    /// 真退出 kichi：连同正在播放的 mpv / 内嵌画面一起关掉（缩托盘不调）。
    pub fn shutdown_players(&self) {
        player::shutdown_all();
        crate::player::embed::shutdown_all();
    }

    /// 改走外部播放器前彻底收掉内嵌：等 mpv 线程收尾，它才会恢复
    /// `WAYLAND_SOCKET`；否则紧接着 spawn 的外部播放器会继承这个 fd，
    /// 连到内嵌代理而不是桌面合成器。浮层状态一并清掉。
    pub(crate) fn stop_embed_and_clear(mut self: Pin<&mut Self>) {
        if !crate::player::embed::stop_and_wait(std::time::Duration::from_millis(1200)) {
            eprintln!("[kichi] 内嵌播放器仍没收尾，外部播放器可能继承 WAYLAND_SOCKET");
        }
        let mut rust = self.as_mut().rust_mut();
        rust.embed_active = false;
        rust.embed_title = QString::default();
        rust.embed_pending_epoch = None;
        rust.embed_live_epoch = None;
    }

    /// 只收起属于这次会话的内嵌浮层；新会话已经接管时不动它。
    pub(crate) fn clear_embed_state(mut self: Pin<&mut Self>, epoch: u64) {
        let mut rust = self.as_mut().rust_mut();
        if rust.embed_pending_epoch != Some(epoch) {
            return;
        }
        rust.embed_pending_epoch = None;
        rust.embed_live_epoch = None;
        rust.embed_active = false;
        rust.embed_title = QString::default();
    }

    /// 内嵌浮层：暂停 / 继续
    pub fn embed_toggle_pause(&self) {
        crate::player::embed::toggle_pause();
    }

    /// 内嵌浮层：跳到某个秒数
    pub fn embed_seek(&self, seconds: f64) {
        crate::player::embed::seek_absolute(seconds);
    }

    /// 内嵌浮层：调音量（0–100）。只改内存并立刻作用到 mpv；
    /// 拖动 / 连按结束后由界面调 `embedCommitAudio` 统一落盘，避免刷盘。
    pub fn embed_set_volume(mut self: Pin<&mut Self>, volume: i32) {
        let volume = volume.clamp(0, 100);
        crate::player::embed::set_volume(volume);
        if self.embed_volume == volume {
            return;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.embed_volume = volume;
        }
        self.as_mut().notify();
    }

    /// 内嵌浮层：静音 / 取消静音（同样等 `embedCommitAudio` 落盘）。
    pub fn embed_toggle_mute(mut self: Pin<&mut Self>) {
        let muted = !self.embed_muted;
        crate::player::embed::set_muted(muted);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.embed_muted = muted;
        }
        self.as_mut().notify();
    }

    /// 把当前音量 / 静音写进设置（界面在停止调节 0.5 秒后调用）。
    pub fn embed_commit_audio(mut self: Pin<&mut Self>) {
        let (volume, muted) = (self.embed_volume, self.embed_muted);
        if self.settings.player_volume == volume && self.settings.player_muted == muted {
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.player_volume = volume;
            settings.player_muted = muted;
        });
    }

    /// 内嵌浮层：关闭这次播放（会话收尾后照常回投 Stopped）
    pub fn embed_stop(&self) {
        crate::player::embed::stop();
    }

    /// 当前播放位置（秒）
    pub fn embed_position(&self) -> f64 {
        crate::player::embed::position().unwrap_or(0.0)
    }

    /// 片长（秒）
    pub fn embed_duration(&self) -> f64 {
        crate::player::embed::duration().unwrap_or(0.0)
    }

    /// 是否处于暂停
    pub fn embed_paused(&self) -> bool {
        crate::player::embed::is_paused()
    }

    /// 是否正在加载 / 缓冲（等缓存、跳转中或起播前空闲）
    pub fn embed_buffering(&self) -> bool {
        crate::player::embed::buffering()
    }

    /// 当前可切换的轨道（音轨 + 字幕轨）；菜单打开时现取，保证勾选状态最新。
    pub fn embed_tracks(&self) -> QVariantList {
        let mut list = QVariantList::default();
        for track in crate::player::embed::tracks() {
            let mut map = QVariantMap::default();
            map.insert_clone(
                &QString::from("id"),
                &QVariant::from(&i32::try_from(track.id).unwrap_or(0)),
            );
            map.insert_clone(
                &QString::from("kind"),
                &QVariant::from(&QString::from(if track.audio { "audio" } else { "sub" })),
            );
            map.insert_clone(
                &QString::from("label"),
                &QVariant::from(&QString::from(track.label.as_str())),
            );
            map.insert_clone(&QString::from("selected"), &QVariant::from(&track.selected));
            list.append(QVariant::from(&map));
        }
        list
    }

    /// 章节列表：`{ time: 起点秒数, title }`；进度条画刻度 / 悬停提示用。
    pub fn embed_chapters(&self) -> QVariantList {
        let mut list = QVariantList::default();
        for chapter in crate::player::embed::chapters() {
            let mut map = QVariantMap::default();
            map.insert_clone(&QString::from("time"), &QVariant::from(&chapter.time));
            map.insert_clone(
                &QString::from("title"),
                &QVariant::from(&QString::from(chapter.title.as_str())),
            );
            list.append(QVariant::from(&map));
        }
        list
    }

    /// 切换音轨 / 字幕轨；`id < 0` 表示关闭字幕。
    pub fn embed_select_track(&self, kind: QString, id: i32) {
        crate::player::embed::select_track(kind.to_string() == "audio", i64::from(id));
    }

    /// 内嵌会话当前音量（mpv 原生键位改过也读得到）。
    pub fn embed_current_volume(&self) -> i32 {
        crate::player::embed::volume()
    }

    /// 内嵌会话当前是否静音。
    pub fn embed_current_muted(&self) -> bool {
        crate::player::embed::muted()
    }

    /// 把 mpv 原生键位改过的音量 / 静音同步到界面属性；落盘仍走 `embedCommitAudio`。
    pub fn embed_sync_audio(mut self: Pin<&mut Self>, volume: i32, muted: bool) {
        let volume = volume.clamp(0, 100);
        if self.embed_volume == volume && self.embed_muted == muted {
            return;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.embed_volume = volume;
            rust.embed_muted = muted;
        }
        self.as_mut().notify();
    }

    /// 当前倍速（1.0 = 原速）；没有会话时返回 1.0。
    pub fn embed_speed(&self) -> f64 {
        crate::player::embed::speed()
    }

    /// 改倍速（50–300%）并写进设置，下次开播继续沿用。
    pub fn embed_set_speed(mut self: Pin<&mut Self>, percent: i32) {
        let percent = percent.clamp(
            i32::from(crate::settings::PLAYER_SPEED_MIN),
            i32::from(crate::settings::PLAYER_SPEED_MAX),
        );
        crate::player::embed::set_speed(f64::from(percent) / 100.0);
        let percent = u16::try_from(percent).unwrap_or(100);
        if self.settings.player_speed_percent == percent {
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.player_speed_percent = percent;
        });
    }

    /// 拖动倍速滑块时的实时预览：只改会话 / mpv，不写设置。
    pub fn embed_preview_speed(&self, percent: i32) {
        let percent = percent.clamp(
            i32::from(crate::settings::PLAYER_SPEED_MIN),
            i32::from(crate::settings::PLAYER_SPEED_MAX),
        );
        crate::player::embed::set_speed(f64::from(percent) / 100.0);
    }

    /// 把播放窗口收到的按键交给 mpv（mpv 原生键位）。
    pub fn embed_keypress(&self, key: QString) {
        crate::player::embed::keypress(&key.to_string());
    }
}
