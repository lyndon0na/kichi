//! 磁链预览与秒传。
//!
//! 新任务弹窗提交单条磁链时，先只读 `resource/list` 拿到文件清单；用户勾选后
//! 再逐个秒传。这里只负责桥接状态与线程回投，网络细节在 `session::magnet`。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QMetaTypeType, QString, QVariant};
use kichi_core::MagnetListing;

use super::actions::{enqueue_ui, format_size, run_in_background};
use super::qobject;
use super::QVariantList;
use crate::session::magnet::MagnetSaveOutcome;
use crate::session::{Failure, State};

type QVariantMap = QMap<QMapPair_QString_QVariant>;

impl qobject::Kichi {
    /// 解析磁链文件清单。成功 / 失败都通过 `magnetPreviewFinished` 回投。
    pub fn preview_magnet_files(mut self: Pin<&mut Self>, link: QString) {
        if self.backend.session.state() != State::SignedIn {
            let message = "请先登录再解析磁链";
            self.as_mut().set_message(message);
            self.as_mut()
                .magnet_preview_finished(false, QString::from(message), false);
            return;
        }
        if self.magnet_loading {
            self.as_mut().magnet_preview_finished(
                false,
                QString::from("正在解析上一条磁链，请稍候"),
                false,
            );
            return;
        }
        let link = link.to_string().trim().to_owned();
        if link.is_empty() {
            self.as_mut()
                .magnet_preview_finished(false, QString::from("磁力链接为空"), false);
            return;
        }

        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.magnet_epoch = rust.magnet_epoch.wrapping_add(1);
            rust.magnet_loading = true;
            rust.magnet_error = QString::default();
            rust.magnet_files = QVariantList::default();
            rust.magnet_listing = None;
            rust.magnet_root_name = QString::default();
            rust.magnet_summary = QString::default();
            rust.magnet_saving = false;
            rust.magnet_save_progress = 0.0;
            rust.magnet_epoch
        };
        self.as_mut().notify();

        let session = self.backend.session.clone();
        run_in_background(
            move || session.resolve_magnet(&link),
            move |mut qichi, result| {
                if qichi.as_ref().rust().magnet_epoch != epoch {
                    return;
                }
                qichi.as_mut().rust_mut().magnet_loading = false;
                match result {
                    Ok(listing) => {
                        let rows = magnet_rows(&listing);
                        let summary = listing_summary(&listing);
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.magnet_files = rows;
                            rust.magnet_root_name = QString::from(listing.root_name.as_str());
                            rust.magnet_summary = QString::from(summary.as_str());
                            rust.magnet_error = QString::default();
                            rust.magnet_listing = Some(listing);
                        }
                        qichi.as_mut().notify();
                        qichi
                            .as_mut()
                            .magnet_preview_finished(true, QString::default(), false);
                    }
                    Err(err) => {
                        let message = err.to_string();
                        let can_fallback = !is_session_failure(&err);
                        qichi.as_mut().rust_mut().magnet_error = QString::from(message.as_str());
                        qichi.as_mut().notify();
                        if is_session_failure(&err) {
                            qichi.as_mut().on_failure(err);
                        }
                        qichi.as_mut().magnet_preview_finished(
                            false,
                            QString::from(message.as_str()),
                            can_fallback,
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
        let Some(listing) = self.magnet_listing.clone() else {
            self.as_mut().set_message("请先解析磁链");
            return;
        };
        let selection: Vec<usize> = indices.iter().filter_map(variant_to_index).collect();
        if selection.is_empty() {
            self.as_mut().set_message("请至少选择一个文件");
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
                    &listing,
                    &selection,
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
                            if is_session_failure(&failure) {
                                qichi.as_mut().on_failure(failure);
                            }
                        }
                        qichi.as_mut().set_message(&message);
                    }
                    Err(err) => {
                        if is_session_failure(&err) {
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
        rust.magnet_listing = None;
        rust.magnet_root_name = QString::default();
        rust.magnet_summary = QString::default();
        rust.magnet_error = QString::default();
        rust.magnet_loading = false;
        rust.magnet_saving = false;
        rust.magnet_save_progress = 0.0;
    }
}

fn magnet_rows(listing: &MagnetListing) -> QVariantList {
    let mut rows = QVariantList::default();
    for (index, file) in listing.files.iter().enumerate() {
        let text = |value: &str| QVariant::from(&QString::from(value));
        let path = if file.folders.is_empty() {
            String::new()
        } else {
            file.folders.join(" / ")
        };
        let note = if file.available() {
            ""
        } else {
            "云端暂无缓存，无法秒传"
        };
        // 嵌套列表必须用 cxx-qt-lib 的 `QList<QVariant>` 才能转成 QVariant；
        // QML 侧会拿到普通 JS 数组（见 MagnetFileTree::folderNames）。
        let mut folder_list: cxx_qt_lib::QList<QVariant> = cxx_qt_lib::QList::default();
        for folder in &file.folders {
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
    }
    rows
}

/// 弹窗顶部摘要：文件数、总大小、可秒传数量。
fn listing_summary(listing: &MagnetListing) -> String {
    let count = listing.files.len();
    let available = listing.available_count();
    let size = format_size(listing.total_size);
    if available == 0 {
        format!("共 {count} 个文件 · {size}；这些文件暂不可秒传，请改用普通云下载")
    } else if available < count {
        format!("共 {count} 个文件 · {size}；{available} 个可秒传，其余暂不可秒传")
    } else {
        format!("共 {count} 个文件 · {size}；全部可秒传")
    }
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

fn is_session_failure(failure: &Failure) -> bool {
    matches!(
        failure,
        Failure::Auth(_) | Failure::Captcha { .. } | Failure::RateLimited { .. }
    )
}

fn to_i32(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{
        listing_summary, magnet_rows, save_outcome_text, save_progress_text, variant_to_index,
    };
    use crate::session::magnet::MagnetSaveOutcome;
    use crate::session::Failure;
    use cxx_qt_lib::{QString, QVariant};
    use kichi_core::{MagnetFile, MagnetListing};

    fn listing(available: usize, total: usize) -> MagnetListing {
        let files = (0..total)
            .map(|index| MagnetFile {
                name: format!("文件{index}"),
                folders: Vec::new(),
                size: 1024,
                gcid: (index < available).then(|| "HASH".to_owned()),
            })
            .collect();
        MagnetListing {
            root_name: "示例".to_owned(),
            files,
            total_size: 1024 * total as u64,
        }
    }

    #[test]
    fn summary_reports_unavailable_files() {
        assert!(listing_summary(&listing(2, 2)).contains("全部可秒传"));
        let partial = listing_summary(&listing(1, 3));
        assert!(partial.contains("1 个可秒传"));
        assert!(partial.contains("其余暂不可秒传"));
        let none = listing_summary(&listing(0, 2));
        assert!(none.contains("暂不可秒传"));
    }

    #[test]
    fn progress_text_reports_position_in_batch() {
        assert_eq!(save_progress_text(3, 12), "正在保存磁链文件 3/12");
    }

    #[test]
    fn outcome_text_counts_every_group() {
        let outcome = MagnetSaveOutcome {
            saved: 2,
            unavailable: 1,
            failed: 3,
            failure: Some(Failure::Other("网络错误".to_owned())),
        };
        let text = save_outcome_text(&outcome);
        assert!(text.contains("已保存 2 个文件"));
        assert!(text.contains("1 个暂不可秒传"));
        assert!(text.contains("3 个保存失败"));
        assert!(text.contains("网络错误"));
    }

    #[test]
    fn rows_carry_folder_chain_for_tree_view() {
        use cxx_qt_lib::{QList, QMap, QMapPair_QString_QVariant};

        let listing = MagnetListing {
            root_name: "示例".to_owned(),
            files: vec![MagnetFile {
                name: "第 1 集.mkv".to_owned(),
                folders: vec!["动画".to_owned(), "第一季".to_owned()],
                size: 1024,
                gcid: Some("HASH".to_owned()),
            }],
            total_size: 1024,
        };
        let rows = magnet_rows(&listing);
        let map = rows
            .at(0)
            .expect("应有一行")
            .value::<QMap<QMapPair_QString_QVariant>>()
            .expect("行应是 QVariantMap");
        let folders = map
            .get(&QString::from("folders"))
            .and_then(|value| value.value::<QList<QVariant>>())
            .expect("folders 应是数组");
        let names: Vec<String> = folders
            .iter()
            .filter_map(|value| value.value::<QString>().map(|name| name.to_string()))
            .collect();
        assert_eq!(names, vec!["动画".to_owned(), "第一季".to_owned()]);
        assert_eq!(
            map.get(&QString::from("path"))
                .and_then(|value| value.value::<QString>())
                .map(|text| text.to_string()),
            Some("动画 / 第一季".to_owned())
        );
    }

    #[test]
    fn indices_accept_both_int_and_double_variants() {
        assert_eq!(variant_to_index(&QVariant::from(&1i32)), Some(1));
        assert_eq!(variant_to_index(&QVariant::from(&2.0f64)), Some(2));
        assert_eq!(variant_to_index(&QVariant::from(&2.4f64)), Some(2));
        assert_eq!(variant_to_index(&QVariant::from(&-1i32)), None);
        assert_eq!(variant_to_index(&QVariant::from(&f64::NAN)), None);
        assert_eq!(
            variant_to_index(&QVariant::from(&QString::from("0"))),
            None,
            "字符串不能被当成 0 号文件"
        );
    }
}
