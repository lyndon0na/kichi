//! 磁链预览与秒传。
//!
//! 新任务弹窗提交磁链时，先只读 `resource/list` 拿到文件清单；用户勾选后
//! 再逐个秒传。多条磁链合并成一份清单，无法秒传的单条回落普通云下载。
//! 这里只负责桥接状态与线程回投，网络细节在 `session::magnet`。

use std::collections::BTreeSet;
use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QMetaTypeType, QString, QStringList, QVariant};
use kichi_core::{normalize_task_url, MagnetListing};

use super::actions::{enqueue_ui, format_size, run_in_background};
use super::qobject;
use super::QVariantList;
use crate::session::magnet::MagnetSaveOutcome;
use crate::session::State;

type QVariantMap = QMap<QMapPair_QString_QVariant>;

impl qobject::Kichi {
    /// 解析一条或多条磁链（换行分隔）的文件清单。成功 / 失败都通过
    /// `magnetPreviewFinished` 回投；无法秒传的磁链单独列在回落链接里。
    pub fn preview_magnet_files(mut self: Pin<&mut Self>, links: QString) {
        if self.backend.session.state() != State::SignedIn {
            let message = "请先登录再解析磁链";
            self.as_mut().set_message(message);
            self.as_mut().magnet_preview_finished(
                false,
                QString::from(message),
                false,
                QStringList::default(),
            );
            return;
        }
        if self.magnet_loading {
            self.as_mut().magnet_preview_finished(
                false,
                QString::from("正在解析上一条磁链，请稍候"),
                false,
                QStringList::default(),
            );
            return;
        }
        let links = unique_links(&links.to_string());
        if links.is_empty() {
            self.as_mut().magnet_preview_finished(
                false,
                QString::from("磁力链接为空"),
                false,
                QStringList::default(),
            );
            return;
        }

        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.magnet_epoch = rust.magnet_epoch.wrapping_add(1);
            rust.magnet_loading = true;
            rust.magnet_error = QString::default();
            rust.magnet_files = QVariantList::default();
            rust.magnet_listings = Vec::new();
            rust.magnet_root_name = QString::default();
            rust.magnet_summary = QString::default();
            rust.magnet_saving = false;
            rust.magnet_save_progress = 0.0;
            rust.magnet_epoch
        };
        self.as_mut().notify();

        let session = self.backend.session.clone();
        run_in_background(
            move || session.resolve_magnets(&links),
            move |mut qichi, result| {
                if qichi.as_ref().rust().magnet_epoch != epoch {
                    return;
                }
                qichi.as_mut().rust_mut().magnet_loading = false;
                match result {
                    Ok(batch) if !batch.listings.is_empty() => {
                        let rows = magnet_rows(&batch.listings);
                        let summary = listing_summary(&batch.listings, batch.fallback_links.len());
                        let root_name = if batch.listings.len() == 1 {
                            batch.listings[0].root_name.clone()
                        } else {
                            format!("{} 条磁链", batch.listings.len())
                        };
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.magnet_files = rows;
                            rust.magnet_root_name = QString::from(root_name.as_str());
                            rust.magnet_summary = QString::from(summary.as_str());
                            rust.magnet_error = QString::default();
                            rust.magnet_listings = batch.listings;
                        }
                        qichi.as_mut().notify();
                        qichi.as_mut().magnet_preview_finished(
                            true,
                            QString::default(),
                            true,
                            string_list(&batch.fallback_links),
                        );
                    }
                    Ok(batch) => {
                        // 一条可秒传的都没有：整批改走普通云下载。
                        let message = "磁链里的文件暂不可秒传，请改用普通云下载";
                        qichi.as_mut().rust_mut().magnet_error = QString::from(message);
                        qichi.as_mut().notify();
                        qichi.as_mut().magnet_preview_finished(
                            false,
                            QString::from(message),
                            true,
                            string_list(&batch.fallback_links),
                        );
                    }
                    Err(err) => {
                        let message = err.to_string();
                        let can_fallback = !err.is_session_failure();
                        qichi.as_mut().rust_mut().magnet_error = QString::from(message.as_str());
                        qichi.as_mut().notify();
                        if err.is_session_failure() {
                            qichi.as_mut().on_failure(err);
                        }
                        qichi.as_mut().magnet_preview_finished(
                            false,
                            QString::from(message.as_str()),
                            can_fallback,
                            QStringList::default(),
                        );
                    }
                }
            },
        );
    }

    /// 秒传选中的文件；`indices` 是 `magnetFiles` 的下标。
    /// `keep_structure` 为 false 时忽略磁链目录层级，全部平铺到目标目录。
    pub fn save_magnet_files(
        mut self: Pin<&mut Self>,
        indices: QVariantList,
        folder_id: QString,
        keep_structure: bool,
    ) {
        if self.backend.session.state() != State::SignedIn {
            self.as_mut().set_message("请先登录再保存磁链文件");
            return;
        }
        if self.magnet_saving {
            self.as_mut().set_message("正在保存上一条磁链，请稍候");
            return;
        }
        if self.magnet_listings.is_empty() {
            self.as_mut().set_message("请先解析磁链");
            return;
        }
        let selection: Vec<usize> = indices.iter().filter_map(variant_to_index).collect();
        if selection.is_empty() {
            self.as_mut().set_message("请至少选择一个文件");
            return;
        }
        let listings = self.magnet_listings.clone();
        let groups = split_selection(&listings, &selection);
        if groups.is_empty() {
            self.as_mut().set_message("请至少选择一个可秒传的文件");
            return;
        }

        let epoch = self.magnet_epoch;
        let folder = folder_id.to_string().trim().to_owned();
        let folder_for_cache = folder.clone();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.magnet_saving = true;
            rust.magnet_save_progress = 0.0;
            rust.magnet_error = QString::default();
            rust.status_message = QString::from("正在保存磁链文件…");
        }
        self.as_mut().notify();

        let session = self.backend.session.clone();
        run_in_background(
            move || {
                session.save_magnet_files(
                    &listings,
                    &groups,
                    &folder,
                    keep_structure,
                    |done, total| {
                        let text = save_progress_text(done, total);
                        let ratio = if total == 0 {
                            0.0
                        } else {
                            done as f64 / total as f64
                        };
                        enqueue_ui(move |mut qichi| {
                            if qichi.as_ref().rust().magnet_epoch != epoch {
                                return;
                            }
                            {
                                let mut rust = qichi.as_mut().rust_mut();
                                rust.magnet_save_progress = ratio;
                                rust.status_message = QString::from(text.as_str());
                            }
                            qichi.as_mut().notify();
                        });
                    },
                )
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().magnet_epoch != epoch {
                    return;
                }
                {
                    let mut rust = qichi.as_mut().rust_mut();
                    rust.magnet_saving = false;
                    rust.magnet_save_progress = 0.0;
                }
                match result {
                    Ok(outcome) => {
                        let message = save_outcome_text(&outcome);
                        if outcome.saved > 0 {
                            // 保存目标可能不是当前目录：让目标目录下次进入时重读，
                            // 用户正看着它才静默刷新。
                            qichi.as_mut().mark_file_dir_stale(&folder_for_cache);
                            if qichi.as_ref().rust().current_dir_id.to_string() == folder_for_cache
                            {
                                qichi.as_mut().refresh_files_background();
                            }
                        }
                        if let Some(failure) = outcome.failure {
                            if failure.is_session_failure() {
                                qichi.as_mut().on_failure(failure);
                            }
                        }
                        qichi.as_mut().set_message(&message);
                    }
                    Err(err) => {
                        if err.is_session_failure() {
                            qichi.as_mut().on_failure(err);
                        } else {
                            qichi.as_mut().set_message(&err.to_string());
                        }
                    }
                }
            },
        );
    }

    /// 退出登录 / 切换账号：丢弃磁链清单与在途结果。
    pub(crate) fn reset_magnet(mut self: Pin<&mut Self>) {
        let mut rust = self.as_mut().rust_mut();
        rust.magnet_epoch = rust.magnet_epoch.wrapping_add(1);
        rust.magnet_files = QVariantList::default();
        rust.magnet_listings.clear();
        rust.magnet_root_name = QString::default();
        rust.magnet_summary = QString::default();
        rust.magnet_error = QString::default();
        rust.magnet_loading = false;
        rust.magnet_saving = false;
        rust.magnet_save_progress = 0.0;
    }
}

fn magnet_rows(listings: &[MagnetListing]) -> QVariantList {
    let grouped = listings.len() > 1;
    let mut rows = QVariantList::default();
    let mut index = 0usize;
    for listing in listings {
        // 单文件磁链的根名通常就是文件名，再套一层同名目录只是多余的壳。
        let flat_single = listing.files.len() == 1 && listing.files[0].folders.is_empty();
        let prefix_root = grouped && !flat_single;
        for file in &listing.files {
            let text = |value: &str| QVariant::from(&QString::from(value));
            // 多条磁链共用一个选择框时，用磁链根目录当最外层分组。
            let folders: Vec<&str> = if prefix_root {
                std::iter::once(listing.root_name.as_str())
                    .chain(file.folders.iter().map(String::as_str))
                    .collect()
            } else {
                file.folders.iter().map(String::as_str).collect()
            };
            let path = folders.join(" / ");
            let note = if file.available() {
                ""
            } else {
                "云端暂无缓存，无法秒传"
            };
            // 嵌套列表必须用 cxx-qt-lib 的 `QList<QVariant>` 才能转成 QVariant；
            // QML 侧会拿到普通 JS 数组（见 MagnetFileTree::folderNames）。
            let mut folder_list: cxx_qt_lib::QList<QVariant> = cxx_qt_lib::QList::default();
            for folder in &folders {
                folder_list.append(text(folder));
            }

            let mut map = QVariantMap::default();
            map.insert_clone(&QString::from("index"), &QVariant::from(&to_i32(index)));
            map.insert_clone(&QString::from("name"), &text(&file.name));
            map.insert_clone(&QString::from("path"), &text(&path));
            map.insert_clone(&QString::from("folders"), &QVariant::from(&folder_list));
            map.insert_clone(&QString::from("size"), &text(&format_size(file.size)));
            map.insert_clone(&QString::from("note"), &text(note));
            map.insert_clone(
                &QString::from("available"),
                &QVariant::from(&file.available()),
            );
            rows.append(QVariant::from(&map));
            index += 1;
        }
    }
    rows
}

/// 弹窗顶部摘要：文件数、总大小、可秒传数量、回落的磁链数。
fn listing_summary(listings: &[MagnetListing], fallback_count: usize) -> String {
    let count: usize = listings.iter().map(|listing| listing.files.len()).sum();
    let available: usize = listings.iter().map(MagnetListing::available_count).sum();
    let size = format_size(listings.iter().map(|listing| listing.total_size).sum());
    let mut text = if available == 0 {
        format!("共 {count} 个文件 · {size}；这些文件暂不可秒传，请改用普通云下载")
    } else if available < count {
        format!("共 {count} 个文件 · {size}；{available} 个可秒传，其余暂不可秒传")
    } else {
        format!("共 {count} 个文件 · {size}；全部可秒传")
    };
    if fallback_count > 0 {
        text.push_str(&format!("；{fallback_count} 条磁链已改为普通云下载"));
    }
    text
}

/// 去掉空行，规范化裸 infohash，并按规范化后的链接去重。
fn unique_links(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut links = Vec::new();
    for line in text.lines() {
        let link = normalize_task_url(line);
        if link.is_empty() {
            continue;
        }
        if seen.insert(link.clone()) {
            links.push(link);
        }
    }
    links
}

fn string_list(values: &[String]) -> QStringList {
    let mut list = QStringList::default();
    for value in values {
        list.append(QString::from(value.as_str()));
    }
    list
}

/// 全局行下标 → 每份清单各自的文件下标。
fn split_selection(listings: &[MagnetListing], selection: &[usize]) -> Vec<(usize, Vec<usize>)> {
    let selected: BTreeSet<usize> = selection.iter().copied().collect();
    let mut groups = Vec::new();
    let mut offset = 0usize;
    for (index, listing) in listings.iter().enumerate() {
        let end = offset + listing.files.len();
        let locals: Vec<usize> = selected
            .range(offset..end)
            .map(|global| global - offset)
            .collect();
        if !locals.is_empty() {
            groups.push((index, locals));
        }
        offset = end;
    }
    groups
}

fn save_progress_text(done: usize, total: usize) -> String {
    format!("正在保存磁链文件 {done}/{total}")
}

fn save_outcome_text(outcome: &MagnetSaveOutcome) -> String {
    let mut text = if outcome.saved > 0 {
        format!("已保存 {} 个文件", outcome.saved)
    } else {
        "没有文件被保存".to_owned()
    };
    if outcome.unavailable > 0 {
        text.push_str(&format!("；{} 个暂不可秒传", outcome.unavailable));
    }
    if outcome.failed > 0 {
        text.push_str(&format!("；{} 个保存失败", outcome.failed));
    }
    if let Some(failure) = &outcome.failure {
        text.push_str(&format!("（{failure}）"));
    }
    text
}

fn variant_to_index(value: &QVariant) -> Option<usize> {
    // QML 的 JS 数组元素可能以 int 或 double 形式进入 QVariantList；
    // 只接受真正的数值类型，避免字符串被 Qt 静默转成 0 后选中第一个文件。
    let number = match value.type_id() {
        QMetaTypeType::Int | QMetaTypeType::Long => value.value::<i32>().map(f64::from),
        QMetaTypeType::UInt | QMetaTypeType::ULong => value.value::<u32>().map(f64::from),
        QMetaTypeType::LongLong => value.value::<i64>().map(|v| v as f64),
        QMetaTypeType::ULongLong => value.value::<u64>().map(|v| v as f64),
        QMetaTypeType::Short => value.value::<i16>().map(f64::from),
        QMetaTypeType::UShort => value.value::<u16>().map(f64::from),
        QMetaTypeType::UChar => value.value::<u8>().map(f64::from),
        QMetaTypeType::SChar => value.value::<i8>().map(f64::from),
        QMetaTypeType::Float => value.value::<f32>().map(f64::from),
        QMetaTypeType::Double => value.value::<f64>(),
        _ => None,
    }?;
    if !number.is_finite() || !(0.0..=i32::MAX as f64).contains(&number) {
        return None;
    }
    usize::try_from(number.round() as i32).ok()
}

fn to_i32(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests;
