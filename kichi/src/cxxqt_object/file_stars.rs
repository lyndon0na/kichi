//! 文件页与资料库页的星标写操作。
//!
//! 星标入口共用资源锁与乐观回滚：文件页快照只恢复星标字段，资料库快照只
//! 恢复本动作碰过的条目，不覆盖同一页面里其它并发星标。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};

use super::actions::run_in_background;
use super::file_action_lock::FileActionDomain;
use super::file_actions::{clean_ids, item_subject};
use super::file_optimistic::is_unconfirmed_create;
use super::qobject;
use crate::session::Failure;

#[derive(Clone, Copy, PartialEq, Eq)]
enum StarOrigin {
    Files,
    Library,
}

impl qobject::Kichi {
    /// 文件页（含搜索结果）批量加星标 / 取消星标。
    pub fn set_starred(mut self: Pin<&mut Self>, ids: QStringList, starred: bool) {
        let ids = clean_ids(ids);
        let first_name = ids
            .first()
            .and_then(|id| self.as_ref().rust().entry_name(id));
        self.as_mut()
            .change_starred(ids, first_name, starred, StarOrigin::Files);
    }

    /// 资料库页单条加星标 / 取消星标。
    pub fn set_library_starred(mut self: Pin<&mut Self>, id: QString, starred: bool) {
        let id = id.to_string().trim().to_owned();
        let name = self
            .library_entries
            .iter()
            .find(|entry| entry.entry.id == id)
            .map(|entry| entry.entry.name.clone());
        self.as_mut()
            .change_starred(vec![id], name, starred, StarOrigin::Library);
    }

    /// 星标写请求共用入口：先在当前页面生效，失败再恢复。
    fn change_starred(
        mut self: Pin<&mut Self>,
        ids: Vec<String>,
        first_name: Option<String>,
        starred: bool,
        origin: StarOrigin,
    ) {
        let action = star_action(starred);
        let ids: Vec<String> = ids
            .into_iter()
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect();
        let domain = match origin {
            StarOrigin::Files => FileActionDomain::Files,
            StarOrigin::Library => FileActionDomain::Library,
        };
        let Some(action_id) = self.as_mut().begin_file_action(&ids, domain) else {
            self.as_mut()
                .set_message("相关条目正在处理中，请稍后再设置星标");
            return;
        };
        let source_loading = match origin {
            StarOrigin::Files => self.as_ref().rust().listing,
            StarOrigin::Library => self.as_ref().rust().library_loading,
        };
        if source_loading {
            let message = match origin {
                StarOrigin::Files => "文件列表正在刷新，请稍后再设置星标",
                StarOrigin::Library => "资料库正在刷新，请稍后再设置星标",
            };
            self.as_mut()
                .fail_file_action(action_id, action, Failure::Other(message.into()));
            return;
        }
        if ids.is_empty() {
            self.as_mut().fail_file_action(
                action_id,
                action,
                Failure::Other("没有选中条目".into()),
            );
            return;
        }
        if ids
            .iter()
            .any(|id| is_unconfirmed_create(self.as_ref().rust(), id))
        {
            self.as_mut().fail_file_action(
                action_id,
                action,
                Failure::Other("文件夹正在同步真实 id，请稍后再设置星标".into()),
            );
            return;
        }

        let subject = item_subject(&ids, first_name);
        let progress = if starred {
            format!("正在为{subject}加星标……")
        } else {
            format!("正在取消{subject}的星标……")
        };
        let success = if starred {
            format!("已为{subject}加星标")
        } else {
            format!("已取消{subject}的星标")
        };
        let session = self.backend.session.clone();
        let request_ids = ids.clone();
        let optimistic_before = if origin == StarOrigin::Files {
            let Some(before) = self.as_mut().optimistic_starred(&ids, starred) else {
                self.as_mut().fail_file_action(
                    action_id,
                    action,
                    Failure::Other("条目已不在当前列表中".into()),
                );
                return;
            };
            Some(before)
        } else {
            None
        };
        let library_before = if origin == StarOrigin::Library {
            let Some(snapshot) = self.as_mut().optimistic_library_starred(&ids, starred) else {
                self.as_mut().fail_file_action(
                    action_id,
                    action,
                    Failure::Other("条目已不在当前资料库中".into()),
                );
                return;
            };
            Some(snapshot)
        } else {
            None
        };
        let library_kind = library_before.as_ref().map(|snapshot| snapshot.kind);
        self.as_mut().set_file_action_message(action_id, &progress);
        run_in_background(
            move || {
                let refs: Vec<&str> = request_ids.iter().map(String::as_str).collect();
                session.set_starred(&refs, starred)
            },
            move |mut qichi, result| match result {
                Ok(()) => {
                    match origin {
                        StarOrigin::Files => {
                            qichi.as_mut().mark_library_caches_stale(None);
                        }
                        StarOrigin::Library => {
                            qichi.as_mut().mark_library_caches_stale(library_kind);
                            // 文件页缓存里可能也有同一条目；下次进入时立即校准星标状态。
                            qichi.as_mut().invalidate_file_caches();
                        }
                    }
                    qichi.as_mut().end_file_action(action_id, action, &success);
                }
                Err(err) => {
                    match origin {
                        StarOrigin::Files => {
                            if let Some(snapshot) = optimistic_before {
                                qichi.as_mut().restore_starred(snapshot);
                            }
                        }
                        StarOrigin::Library => {
                            if let Some(snapshot) = library_before {
                                qichi.as_mut().rollback_library_starred(snapshot);
                            }
                        }
                    }
                    qichi.as_mut().fail_file_action(action_id, action, err);
                }
            },
        );
    }
}

fn star_action(starred: bool) -> &'static str {
    if starred {
        "star"
    } else {
        "unstar"
    }
}
