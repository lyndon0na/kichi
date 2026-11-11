//! 文件 / 文件夹属性：按 id 取一次详情，补齐缺字段并把可显示文本交给界面。
//!
//! 列表条目已经带名称 / 类型 / 大小 / 时间，但部分条目（尤其历史快照与本地索引）
//! 可能缺创建时间或修改时间；属性弹窗打开时统一按 id 请求一次详情，不阻塞列表。
//! 文件夹同样支持详情（`api.md` 已确认），大小仍显示「—」，不编造文件夹总大小。
//!
//! 路径只从「真实拿到的」父链还原：
//! - 当前目录里的条目直接用面包屑；
//! - 全盘搜索命中的条目用扫描时尽力还原的 `folder_path`；
//! - 都拼不出来时返回空串，界面显示「—」，绝不拿 id 冒充路径。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::{EntryKind, FileEntry};

use super::actions::{format_size, run_in_background};
use super::files::ROOT_NAME;
use super::files_view::local_time_text;
use super::qobject;
use crate::session::Failure;

/// 缺失字段在界面上统一显示为破折号（与资料库表格口径一致）。
const MISSING: &str = "—";

impl qobject::Kichi {
    /// 右键「属性…」：后台读一次详情，成功后发 `entryPropertiesReady`。
    ///
    /// 失败同样回投（`ok = false`），由弹窗原地显示原因；文件页本身不等待。
    pub fn request_entry_properties(self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        if id.trim().is_empty() {
            return;
        }
        let session = self.backend.session.clone();
        let signal_id = id.clone();
        run_in_background(
            move || {
                let client = session.drive()?;
                client.entry_detail(&id).map_err(Failure::from)
            },
            move |mut qichi, result| match result {
                Ok(entry) => {
                    let path = properties_path(&qichi, &entry);
                    let name = entry.name.clone();
                    let kind = kind_text(entry.kind).to_owned();
                    let size = size_text(&entry);
                    let created = time_text(entry.created_at);
                    let modified = time_text(entry.modified_at);
                    let entry_id = entry.id.clone();
                    qichi.as_mut().entry_properties_ready(
                        QString::from(signal_id.as_str()),
                        true,
                        QString::default(),
                        QString::from(name.as_str()),
                        QString::from(kind.as_str()),
                        QString::from(size.as_str()),
                        QString::from(created.as_str()),
                        QString::from(modified.as_str()),
                        QString::from(entry_id.as_str()),
                        QString::from(path.as_str()),
                    );
                }
                Err(far) => {
                    let message = far.to_string();
                    let auth = matches!(
                        &far,
                        Failure::Auth(_) | Failure::Captcha { .. } | Failure::RateLimited { .. }
                    );
                    qichi.as_mut().entry_properties_ready(
                        QString::from(signal_id.as_str()),
                        false,
                        QString::from(message.as_str()),
                        QString::default(),
                        QString::default(),
                        QString::default(),
                        QString::default(),
                        QString::default(),
                        QString::default(),
                        QString::default(),
                    );
                    if auth {
                        qichi.as_mut().on_failure(far);
                    }
                }
            },
        );
    }
}

fn kind_text(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Folder => "文件夹",
        EntryKind::File => "文件",
    }
}

/// 文件给真实大小；文件夹没有可靠的聚合大小，一律显示破折号。
fn size_text(entry: &FileEntry) -> String {
    match entry.kind {
        EntryKind::Folder => MISSING.to_owned(),
        EntryKind::File => format_size(entry.size),
    }
}

fn time_text(time: Option<chrono::DateTime<chrono::Utc>>) -> String {
    let text = local_time_text(time);
    if text.is_empty() {
        MISSING.to_owned()
    } else {
        text
    }
}

/// 条目所在的完整路径；拼接依据不充分时返回空串。
fn properties_path(this: &qobject::Kichi, entry: &FileEntry) -> String {
    let rust = this.rust();

    // 全盘搜索命中：`folder_path` 是扫描时从真实父链还原的路径（相对根目录）。
    if let Some(hit) = rust.search_index.get(&entry.id) {
        if hit.in_trash {
            return String::new();
        }
        if let Some(path) = hit
            .folder_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            let mut segments: Vec<&str> = path
                .split('/')
                .map(str::trim)
                .filter(|segment| !segment.is_empty())
                .collect();
            segments.insert(0, ROOT_NAME);
            segments.push(entry.name.as_str());
            return segments.join(" / ");
        }
        if hit.folder_id.is_empty() {
            return format!("{ROOT_NAME} / {}", entry.name);
        }
        return String::new();
    }

    // 当前目录里的条目：面包屑就是它的完整路径。
    if entry.parent_id == rust.current_dir_id.to_string() {
        let mut segments: Vec<String> =
            rust.crumbs.iter().map(|crumb| crumb.name.clone()).collect();
        if segments.is_empty() {
            segments.push(ROOT_NAME.to_owned());
        }
        segments.push(entry.name.clone());
        return segments.join(" / ");
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_have_no_invented_total_size() {
        let entry = entry("folder", EntryKind::Folder, 0);
        assert_eq!(size_text(&entry), MISSING);
    }

    #[test]
    fn files_use_the_shared_size_formatter() {
        let entry = entry("file", EntryKind::File, 1536);
        assert_eq!(size_text(&entry), "2 KB");
    }

    #[test]
    fn missing_times_show_a_dash() {
        assert_eq!(time_text(None), MISSING);
    }

    #[test]
    fn kind_text_matches_the_ui_vocabulary() {
        assert_eq!(kind_text(EntryKind::File), "文件");
        assert_eq!(kind_text(EntryKind::Folder), "文件夹");
    }

    fn entry(name: &str, kind: EntryKind, size: u64) -> FileEntry {
        FileEntry {
            id: name.to_owned(),
            name: name.to_owned(),
            kind,
            size,
            created_at: None,
            modified_at: None,
            parent_id: String::new(),
            md5: None,
            audit_flagged: false,
            starred: false,
            media: false,
            video: false,
            thumbnail_url: String::new(),
        }
    }
}
