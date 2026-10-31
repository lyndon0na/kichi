//! 分享：创建弹窗的结果、管理页列表、批量撤销。

use std::pin::Pin;
use std::time::{Duration, Instant};

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QStringList, QVariant};
use kichi_core::{EntryKind, MyShare};

use super::actions::run_in_background;
use super::file_optimistic::is_unconfirmed_create;
use super::qobject;
use super::QVariantList;
use crate::session::Failure;

type QVariantMap = QMap<QMapPair_QString_QVariant>;

/// 切页 / 手动刷新的短冷却。
const REFRESH_COOLDOWN: Duration = Duration::from_secs(3);
/// 页面重新可见时，分享列表缓存超过这个时间才后台校准。
const SHARES_REFRESH_TTL: Duration = Duration::from_secs(30);

impl qobject::Kichi {
    pub fn refresh_shares(mut self: Pin<&mut Self>) {
        self.as_mut().refresh_shares_inner(false, true);
    }

    pub fn ensure_shares_fresh(mut self: Pin<&mut Self>) {
        let now = Instant::now();
        if self.shares_request_running
            || self.shares_loaded
                && self
                    .share_refresh_at
                    .is_some_and(|at| now.saturating_duration_since(at) < SHARES_REFRESH_TTL)
        {
            return;
        }
        self.as_mut().refresh_shares_inner(false, false);
    }

    fn refresh_shares_force(mut self: Pin<&mut Self>) {
        self.as_mut().refresh_shares_inner(true, false);
    }

    fn refresh_shares_inner(mut self: Pin<&mut Self>, force: bool, manual: bool) {
        let session = self.backend.session.clone();
        let has_cache = self.shares_loaded;
        if self.shares_request_running {
            let show_manual = {
                let mut rust = self.as_mut().rust_mut();
                if force {
                    rust.share_refresh_pending = true;
                }
                let show = manual && has_cache && !rust.shares_loading;
                if show {
                    rust.shares_loading = true;
                }
                show
            };
            if show_manual {
                self.as_mut().notify();
            }
            return;
        }
        let (epoch, visible) = {
            let now = Instant::now();
            let mut rust = self.as_mut().rust_mut();
            if !force && !refresh_due(rust.share_refresh_at, now) {
                return;
            }
            let visible = manual || !has_cache;
            rust.shares_loading = visible;
            rust.shares_request_running = true;
            rust.share_refresh_at = Some(now);
            (rust.shares_epoch, visible)
        };
        if visible {
            self.as_mut().notify();
        }
        run_in_background(
            move || session.shares(),
            move |mut qichi, result| {
                if qichi.as_ref().rust().shares_epoch != epoch {
                    return;
                }
                match result {
                    Ok(shares) => qichi.as_mut().show_shares(shares),
                    Err(err) => {
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.shares_loading = false;
                            rust.shares_request_running = false;
                            rust.share_refresh_at = Some(Instant::now());
                        }
                        qichi.as_mut().on_refresh_failure(err);
                    }
                }
                let again = {
                    let mut rust = qichi.as_mut().rust_mut();
                    let pending = rust.share_refresh_pending;
                    rust.share_refresh_pending = false;
                    pending
                };
                if again {
                    qichi.as_mut().refresh_shares_force();
                }
            },
        );
    }

    pub fn create_share(
        mut self: Pin<&mut Self>,
        file_ids: QStringList,
        expiration_days: i32,
        need_password: bool,
    ) {
        if self.share_action_running {
            return;
        }
        let ids = clean_ids(file_ids);
        if ids.is_empty() {
            self.as_mut().share_action_done(
                QString::from("create"),
                false,
                QString::from("没有选中要分享的条目"),
            );
            return;
        }
        if ids
            .iter()
            .any(|id| is_unconfirmed_create(self.as_ref().rust(), id))
        {
            self.as_mut().share_action_done(
                QString::from("create"),
                false,
                QString::from("文件夹正在同步真实 id，请稍后再分享"),
            );
            return;
        }
        let days = i64::from(expiration_days);
        if days != -1 && days <= 0 {
            self.as_mut().share_action_done(
                QString::from("create"),
                false,
                QString::from("分享有效期无效"),
            );
            return;
        }

        let count = ids.len();
        let subject = if count == 1 {
            "选中的条目".to_owned()
        } else {
            format!("选中的 {count} 项")
        };
        let session = self.backend.session.clone();
        let action_epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.share_action_running = true;
            rust.share_created_url = QString::default();
            rust.share_created_pass_code = QString::default();
            rust.share_created_expiration = QString::default();
            rust.status_message = QString::from("正在创建分享链接……");
            rust.share_action_epoch
        };
        self.as_mut().notify();

        run_in_background(
            move || {
                let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
                session.create_share(&refs, days, need_password)
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().share_action_epoch != action_epoch {
                    return;
                }
                match result {
                    Ok(share) => {
                        let message = if share.pass_code.is_empty() {
                            format!("{subject}的分享链接已创建")
                        } else {
                            format!("{subject}的分享链接已创建（已生成访问密码）")
                        };
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.share_action_running = false;
                            rust.share_created_url = QString::from(share.share_url.as_str());
                            rust.share_created_pass_code = QString::from(share.pass_code.as_str());
                            rust.share_created_expiration =
                                QString::from(expiration_label(days).as_str());
                            rust.status_message = QString::default();
                        }
                        qichi.as_mut().notify();
                        qichi.as_mut().share_action_done(
                            QString::from("create"),
                            true,
                            QString::from(message.as_str()),
                        );
                        qichi.as_mut().refresh_shares_force();
                    }
                    Err(err) => qichi.as_mut().fail_share_action("create", err),
                }
            },
        );
    }

    pub fn revoke_shares(mut self: Pin<&mut Self>, share_ids: QStringList) {
        if self.share_action_running {
            return;
        }
        let ids = clean_ids(share_ids);
        if ids.is_empty() {
            self.as_mut().share_action_done(
                QString::from("revoke"),
                false,
                QString::from("没有选中要撤销的分享"),
            );
            return;
        }
        let count = ids.len();
        let message = if count == 1 {
            "正在撤销 1 个分享链接……".to_owned()
        } else {
            format!("正在撤销 {count} 个分享链接……")
        };
        let done_message = if count == 1 {
            "分享链接已撤销，源文件未删除".to_owned()
        } else {
            format!("{count} 个分享链接已撤销，源文件未删除")
        };
        let session = self.backend.session.clone();
        let action_epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.share_action_running = true;
            rust.status_message = QString::from(message.as_str());
            rust.share_action_epoch
        };
        self.as_mut().notify();

        run_in_background(
            move || {
                let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
                session.revoke_shares(&refs)
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().share_action_epoch != action_epoch {
                    return;
                }
                match result {
                    Ok(()) => {
                        qichi.as_mut().finish_share_action("revoke", &done_message);
                        qichi.as_mut().refresh_shares_force();
                    }
                    Err(err) => qichi.as_mut().fail_share_action("revoke", err),
                }
            },
        );
    }

    pub(crate) fn reset_shares(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.shares_epoch = rust.shares_epoch.wrapping_add(1);
            rust.share_action_epoch = rust.share_action_epoch.wrapping_add(1);
            rust.shares = QVariantList::default();
            rust.shares_loading = false;
            rust.shares_request_running = false;
            rust.shares_loaded = false;
            rust.share_list.clear();
            rust.share_refresh_pending = false;
            rust.share_refresh_at = None;
            rust.share_action_running = false;
            rust.share_created_url = QString::default();
            rust.share_created_pass_code = QString::default();
            rust.share_created_expiration = QString::default();
        }
        self.as_mut().notify();
    }

    fn show_shares(mut self: Pin<&mut Self>, mut shares: Vec<MyShare>) {
        shares.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                .then_with(|| a.share_id.cmp(&b.share_id))
        });
        let mut list = QVariantList::default();
        for share in &shares {
            list.append(QVariant::from(&share_map(share)));
        }
        let notify;
        {
            let mut rust = self.as_mut().rust_mut();
            let changed = rust.share_list != shares;
            let visible = rust.shares_loading;
            if changed {
                rust.shares = list;
                rust.share_list = shares;
            }
            rust.shares_loading = false;
            rust.shares_request_running = false;
            rust.shares_loaded = true;
            rust.share_refresh_at = Some(Instant::now());
            notify = changed || visible;
        }
        if notify {
            self.as_mut().notify();
        }
    }

    fn finish_share_action(mut self: Pin<&mut Self>, action: &str, message: &str) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.share_action_running = false;
            rust.status_message = QString::default();
        }
        self.as_mut().notify();
        self.as_mut()
            .share_action_done(QString::from(action), true, QString::from(message));
    }

    fn fail_share_action(mut self: Pin<&mut Self>, action: &str, failure: Failure) {
        self.as_mut().rust_mut().share_action_running = false;
        let text = failure.to_string();
        self.as_mut().on_failure(failure);
        self.as_mut()
            .share_action_done(QString::from(action), false, QString::from(text.as_str()));
    }
}

fn refresh_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= REFRESH_COOLDOWN)
}

fn clean_ids(ids: QStringList) -> Vec<String> {
    ids.iter()
        .map(|id| id.to_string().trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect()
}

fn expiration_label(days: i64) -> String {
    match days {
        -1 | 0 => "永久".to_owned(),
        days if days > 0 => format!("{days} 天"),
        _ => "未知".to_owned(),
    }
}

fn share_map(share: &MyShare) -> QVariantMap {
    let text = |value: &str| QVariant::from(&QString::from(value));
    let kind = match share.kind {
        Some(EntryKind::Folder) => "folder",
        Some(EntryKind::File) => "file",
        None => "",
    };
    let title = if share.title.trim().is_empty() {
        if share.file_count > 0 {
            format!("分享的 {} 个文件", share.file_count)
        } else {
            "未命名分享".to_owned()
        }
    } else {
        share.title.clone()
    };
    let (status_text, status_tone) =
        if share.status.trim().is_empty() || share.status.eq_ignore_ascii_case("OK") {
            ("有效".to_owned(), "success")
        } else {
            (format!("状态：{}", share.status.trim()), "warning")
        };
    let created = share
        .created_at
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let stats = format!(
        "文件 {} · 浏览 {} · 转存 {}",
        share.file_count, share.view_count, share.restore_count
    );

    let mut map = QVariantMap::default();
    map.insert_clone(&QString::from("id"), &text(&share.share_id));
    map.insert_clone(&QString::from("title"), &text(&title));
    map.insert_clone(&QString::from("kind"), &text(kind));
    map.insert_clone(&QString::from("url"), &text(&share.share_url));
    map.insert_clone(&QString::from("passCode"), &text(&share.pass_code));
    map.insert_clone(
        &QString::from("hasPassword"),
        &QVariant::from(&share.has_password()),
    );
    map.insert_clone(
        &QString::from("accessText"),
        &text(if share.has_password() {
            "密码保护"
        } else {
            "公开"
        }),
    );
    map.insert_clone(
        &QString::from("expirationText"),
        &text(&expiration_label(share.expiration_days)),
    );
    map.insert_clone(&QString::from("createdText"), &text(&created));
    map.insert_clone(&QString::from("statsText"), &text(&stats));
    map.insert_clone(&QString::from("statusText"), &text(&status_text));
    map.insert_clone(&QString::from("statusTone"), &text(status_tone));
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share(kind: Option<EntryKind>) -> MyShare {
        MyShare {
            share_id: "S1".to_owned(),
            share_url: "https://mypikpak.com/s/S1".to_owned(),
            title: "电影合集".to_owned(),
            pass_code: String::new(),
            share_to: "publiclink".to_owned(),
            created_at: None,
            expiration_days: 7,
            view_count: 0,
            restore_count: 0,
            file_count: 1,
            status: "OK".to_owned(),
            kind,
        }
    }

    #[test]
    fn share_map_exposes_kind_with_empty_fallback() {
        let read = |share: &MyShare| {
            share_map(share)
                .get(&QString::from("kind"))
                .and_then(|v| v.value::<QString>())
                .map(|s| s.to_string())
                .unwrap_or_default()
        };
        assert_eq!(read(&share(Some(EntryKind::Folder))), "folder");
        assert_eq!(read(&share(Some(EntryKind::File))), "file");
        assert_eq!(read(&share(None)), "");
    }

    #[test]
    fn expiration_labels_match_ui_options() {
        assert_eq!(expiration_label(-1), "永久");
        assert_eq!(expiration_label(0), "永久");
        assert_eq!(expiration_label(1), "1 天");
        assert_eq!(expiration_label(30), "30 天");
        assert_eq!(expiration_label(-2), "未知");
    }

    #[test]
    fn refresh_cooldown_is_three_seconds() {
        let now = Instant::now();
        assert!(refresh_due(None, now));
        assert!(!refresh_due(Some(now), now + Duration::from_secs(2)));
        assert!(refresh_due(Some(now), now + Duration::from_secs(3)));
    }
}
