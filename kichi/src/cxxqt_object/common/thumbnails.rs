//! 文件列表缩略图：远程地址留在 Rust，QML 只拿本地 `file://` URI。
//!
//! 列表映射不会带出 `thumbnail_url`；QML 按文件 id 请求，后台下载并写缓存，
//! 完成后通过独立信号回传。同一 id 的并发请求只允许一个。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use super::actions::run_in_background;
use super::qobject;
use crate::session::{thumbnail_cache, Failure};

impl qobject::Kichi {
    /// 已缓存缩略图的本地 URI；没有命中时返回空串。
    pub fn thumbnail_path(&self, id: QString) -> QString {
        let id = id.to_string();
        thumbnail_cache::find(&id)
            .map(|path| QString::from(super::reveal::file_uri(&path).as_str()))
            .unwrap_or_default()
    }

    /// 请求缩略图。缓存命中直接发结果；未命中才启动后台下载。
    pub fn request_thumbnail(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        if id.is_empty() {
            return;
        }

        if let Some(path) = thumbnail_cache::find(&id) {
            self.as_mut().thumbnail_ready(
                QString::from(id.as_str()),
                QString::from(super::reveal::file_uri(&path).as_str()),
            );
            return;
        }

        if self.thumbnail_requests.contains(&id) {
            return;
        }

        let Some(url) = self.thumbnail_url_for(&id) else {
            // 没有可下载地址：明确回空，让 QML 立刻回到文件图标。
            self.as_mut()
                .thumbnail_ready(QString::from(id.as_str()), QString::default());
            return;
        };

        let session = self.backend.session.clone();
        let epoch = self.thumbnail_epoch;
        let request_id = id.clone();
        let result_id = id.clone();
        self.as_mut().rust_mut().thumbnail_requests.insert(id);

        run_in_background(
            move || {
                let bytes = session.fetch_thumbnail(&url)?;
                thumbnail_cache::store(&request_id, &bytes).map_err(Failure::Other)
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().thumbnail_epoch != epoch {
                    return;
                }
                qichi
                    .as_mut()
                    .rust_mut()
                    .thumbnail_requests
                    .remove(&result_id);
                let path = result
                    .ok()
                    .map(|path| super::reveal::file_uri(&path))
                    .unwrap_or_default();
                qichi.as_mut().thumbnail_ready(
                    QString::from(result_id.as_str()),
                    QString::from(path.as_str()),
                );
            },
        );
    }

    /// 退出登录时清掉当前账号的在途缩略图请求。
    pub(crate) fn reset_thumbnails(mut self: Pin<&mut Self>) {
        let mut rust = self.as_mut().rust_mut();
        rust.thumbnail_epoch = rust.thumbnail_epoch.wrapping_add(1);
        rust.thumbnail_requests.clear();
    }

    /// 先查文件相关列表，再查离线任务产物；只返回 Rust 侧远程地址。
    fn thumbnail_url_for(&self, id: &str) -> Option<String> {
        find_thumbnail_url(
            self.entries
                .iter()
                .chain(self.trash_list.iter().map(|entry| &entry.entry))
                .chain(self.search_index.values().map(|hit| &hit.entry))
                .chain(self.library_entries.iter().map(|entry| &entry.entry)),
            id,
        )
        .or_else(|| find_task_thumbnail_url(self.task_list.iter(), id))
    }
}

fn find_thumbnail_url<'a>(
    mut entries: impl Iterator<Item = &'a kichi_core::FileEntry>,
    id: &str,
) -> Option<String> {
    entries
        .find(|entry| entry.id == id)
        .map(|entry| entry.thumbnail_url.trim().to_owned())
        .filter(|url| !url.is_empty())
}

fn find_task_thumbnail_url<'a>(
    mut tasks: impl Iterator<Item = &'a kichi_core::OfflineTask>,
    id: &str,
) -> Option<String> {
    tasks
        .find(|task| task.file_id.as_deref() == Some(id))
        .map(|task| task.thumbnail_url.trim().to_owned())
        .filter(|url| !url.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kichi_core::{EntryKind, FileEntry, OfflineTask, TaskPhase};

    fn entry(id: &str, thumbnail_url: &str) -> FileEntry {
        FileEntry {
            id: id.to_owned(),
            name: "sample.mp4".to_owned(),
            kind: EntryKind::File,
            size: 1,
            created_at: None,
            modified_at: None,
            parent_id: String::new(),
            md5: None,
            audit_flagged: false,
            starred: false,
            media: true,
            video: true,
            thumbnail_url: thumbnail_url.to_owned(),
        }
    }

    #[test]
    fn only_non_empty_thumbnail_urls_are_returned() {
        let entries = [
            entry("has", " https://thumb.example/x "),
            entry("empty", ""),
        ];
        assert_eq!(
            find_thumbnail_url(entries.iter(), "has").as_deref(),
            Some("https://thumb.example/x")
        );
        assert_eq!(find_thumbnail_url(entries.iter(), "empty"), None);
        assert_eq!(find_thumbnail_url(entries.iter(), "missing"), None);
    }

    #[test]
    fn offline_task_resource_thumbnail_is_available_by_file_id() {
        let task = OfflineTask {
            id: "task".to_owned(),
            name: "movie.mkv".to_owned(),
            source_url: None,
            phase: TaskPhase::Completed,
            progress: 100,
            message: None,
            created_at: None,
            file_id: Some("file".to_owned()),
            output_parent_id: None,
            file_size: 1,
            kind: Some(EntryKind::File),
            thumbnail_url: " https://thumb.example/task ".to_owned(),
            media: true,
            video: true,
        };
        assert_eq!(
            find_task_thumbnail_url(std::iter::once(&task), "file").as_deref(),
            Some("https://thumb.example/task")
        );
        assert_eq!(
            find_task_thumbnail_url(std::iter::once(&task), "other"),
            None
        );
    }
}
