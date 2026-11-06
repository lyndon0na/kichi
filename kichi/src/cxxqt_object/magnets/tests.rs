use super::{
    listing_summary, magnet_rows, save_outcome_text, save_progress_text, split_selection,
    unique_links, variant_to_index, QVariantList,
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
    assert!(listing_summary(&[listing(2, 2)], 0).contains("全部可秒传"));
    let partial = listing_summary(&[listing(1, 3)], 0);
    assert!(partial.contains("1 个可秒传"));
    assert!(partial.contains("其余暂不可秒传"));
    let none = listing_summary(&[listing(0, 2)], 0);
    assert!(none.contains("暂不可秒传"));
}

#[test]
fn summary_mentions_links_that_fell_back_to_cloud_download() {
    let text = listing_summary(&[listing(2, 2)], 1);
    assert!(text.contains("1 条磁链已改为普通云下载"));
}

fn folders_at(rows: &QVariantList, index: usize) -> Vec<String> {
    use cxx_qt_lib::{QList, QMap, QMapPair_QString_QVariant};
    let map = rows
        .at(index)
        .expect("应有一行")
        .value::<QMap<QMapPair_QString_QVariant>>()
        .expect("行应是 QVariantMap");
    let values = map
        .get(&QString::from("folders"))
        .and_then(|value| value.value::<QList<QVariant>>())
        .expect("folders 应是数组");
    values
        .iter()
        .filter_map(|value| value.value::<QString>().map(|name| name.to_string()))
        .collect()
}

#[test]
fn single_file_listings_do_not_gain_a_duplicate_root_folder() {
    let first = listing(1, 1);
    let second = MagnetListing {
        root_name: "第二季".to_owned(),
        ..listing(2, 2)
    };
    let single = magnet_rows(std::slice::from_ref(&first));
    assert_eq!(
        folders_at(&single, 0),
        Vec::<String>::new(),
        "单条磁链不加根目录"
    );
    let rows = magnet_rows(&[first, second]);
    assert_eq!(
        folders_at(&rows, 0),
        Vec::<String>::new(),
        "单文件磁链不再套一层同名目录"
    );
    assert_eq!(
        folders_at(&rows, 1),
        vec!["第二季".to_owned()],
        "多文件磁链仍按根目录分组"
    );
    assert_eq!(folders_at(&rows, 2), vec!["第二季".to_owned()]);
}

#[test]
fn global_selection_splits_back_into_each_listing() {
    let listings = [listing(1, 1), listing(1, 2)];
    let groups = split_selection(&listings, &[0, 2, 1, 9]);
    assert_eq!(groups, vec![(0, vec![0]), (1, vec![0, 1])]);
}

#[test]
fn links_are_normalized_and_deduped() {
    let hash = "0123456789ABCDEF0123456789ABCDEF01234567";
    let links = unique_links(&format!(
        "{hash}\n\nmagnet:?xt=urn:btih:def\nmagnet:?xt=urn:btih:def\n"
    ));
    assert_eq!(
        links,
        vec![
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".to_owned(),
            "magnet:?xt=urn:btih:def".to_owned()
        ]
    );
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
    let rows = magnet_rows(std::slice::from_ref(&listing));
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
