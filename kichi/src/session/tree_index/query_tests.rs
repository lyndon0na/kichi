//! `query` 的读路径测试。

use super::super::apply::{apply_change, put_listing, Change};
use super::super::store::Store;
use super::super::test_support::{file, folder};
use super::*;

fn with_store<T>(f: impl FnOnce(&Store) -> T) -> T {
    let store = Store::open_in_memory("u1").expect("打开内存库");
    let value = f(&store);
    store.close();
    value
}

fn names(entries: &[FileEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.name.as_str()).collect()
}

#[test]
fn subdirectories_only_return_live_children() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(
                    conn,
                    "dir",
                    &[file("a", "a.mkv", "dir"), folder("d", "动画", "dir")],
                    true,
                    100,
                )?;
                conn.execute("UPDATE nodes SET trashed = 1 WHERE id = 'd'", [])
            })
            .expect("写入");
        let entries = store
            .with_conn(|conn| list_dir(conn, "dir"))
            .expect("读目录");
        assert_eq!(names(&entries), vec!["a.mkv"]);
        assert_eq!(entries[0].kind, EntryKind::File);
        assert!(entries[0].media, "扩展名应推出可播放标记");
    });
}

#[test]
fn parent_chain_is_root_to_leaf() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(conn, "", &[folder("bangumi", "追番", "")], true, 100)?;
                put_listing(
                    conn,
                    "bangumi",
                    &[folder("anime", "动画", "bangumi")],
                    true,
                    100,
                )?;
                put_listing(
                    conn,
                    "anime",
                    &[folder("season", "第 1 季", "anime")],
                    true,
                    100,
                )
            })
            .expect("写入");
        let trail = store
            .with_conn(|conn| path_to(conn, "season"))
            .expect("读路径")
            .expect("路径应存在");
        assert_eq!(
            trail,
            vec![
                ("bangumi".to_owned(), "追番".to_owned()),
                ("anime".to_owned(), "动画".to_owned()),
                ("season".to_owned(), "第 1 季".to_owned()),
            ]
        );
        let missing = store
            .with_conn(|conn| path_to(conn, "missing"))
            .expect("读缺失路径");
        assert!(missing.is_none());
    });
}

#[test]
fn cycles_stop_at_the_depth_limit() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                for index in 0..(MAX_PATH_DEPTH + 5) {
                    let id = format!("f{index}");
                    let parent = if index == 0 {
                        String::new()
                    } else {
                        format!("f{index}")
                    };
                    // 每个节点都指向自己：没有深度上限会无限循环
                    conn.execute(
                        "INSERT INTO nodes(id, parent_id, name, norm_name, kind) \
                             VALUES(?1, ?2, ?1, ?1, 1)",
                        rusqlite::params![id, parent],
                    )?;
                }
                Ok(())
            })
            .expect("造环");
        let trail = store.with_conn(|conn| path_to(conn, "f1")).expect("读路径");
        assert!(trail.is_none(), "超过深度上限应放弃路径");
    });
}

#[test]
fn search_uses_the_normalized_name() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(
                    conn,
                    "",
                    &[
                        file("a", "測試檔案.MKV", ""),
                        file("b", "テスト動画.mp4", ""),
                        file("c", "README.txt", ""),
                    ],
                    true,
                    100,
                )
            })
            .expect("写入");
        let hits = |query: &str| {
            store
                .with_conn(|conn| search(conn, query, 50))
                .expect("搜索")
        };
        assert_eq!(names(&hits("测试")), vec!["測試檔案.MKV"]);
        assert_eq!(names(&hits("てすと")), vec!["テスト動画.mp4"]);
        assert_eq!(names(&hits("readme")), vec!["README.txt"]);
        assert!(hits("不存在的关键词").is_empty());
        assert!(hits("   ").is_empty(), "空关键词不能匹配全部");
    });
}

#[test]
fn search_respects_the_limit() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(
                    conn,
                    "",
                    &[
                        file("a", "a.txt", ""),
                        file("b", "b.txt", ""),
                        file("c", "c.txt", ""),
                    ],
                    true,
                    100,
                )
            })
            .expect("写入");
        let hits = store
            .with_conn(|conn| search(conn, "txt", 2))
            .expect("搜索");
        assert_eq!(hits.len(), 2);
    });
}

#[test]
fn starred_returns_only_flagged_live_entries() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(
                    conn,
                    "",
                    &[file("a", "a.mkv", ""), file("b", "b.mkv", "")],
                    true,
                    100,
                )?;
                conn.execute("UPDATE nodes SET starred = 1 WHERE id = 'a'", [])?;
                conn.execute("UPDATE nodes SET trashed = 1 WHERE id = 'b'", [])
            })
            .expect("写入");
        let hits = store.with_conn(|conn| starred(conn, 50)).expect("星标");
        assert_eq!(names(&hits), vec!["a.mkv"]);
    });
}

#[test]
fn starred_order_matches_the_cloud_fallback() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(
                    conn,
                    "",
                    &[
                        file("a", "same.mkv", ""),
                        file("b", "same.mkv", ""),
                        file("c", "zzz.mkv", ""),
                        file("d", "aaa.mkv", ""),
                    ],
                    true,
                    100,
                )?;
                conn.execute(
                    "UPDATE nodes SET starred = 1, modified_at = 20 WHERE id IN ('a', 'b')",
                    [],
                )?;
                conn.execute(
                    "UPDATE nodes SET starred = 1, modified_at = 10 WHERE id = 'c'",
                    [],
                )?;
                conn.execute(
                    "UPDATE nodes SET starred = 1, modified_at = NULL WHERE id = 'd'",
                    [],
                )
            })
            .expect("写入");
        let hits = store.with_conn(|conn| starred(conn, 50)).expect("星标");
        let ids: Vec<&str> = hits.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["a", "b", "c", "d"],
            "更新时间降序；同时间按名称再按 id；无时间排最后"
        );
        assert!(store
            .with_conn(|conn| starred(conn, 0))
            .expect("零上限")
            .is_empty());
    });
}

#[test]
fn trash_snapshot_is_fresh_and_only_lists_roots() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(conn, "", &[folder("d", "动画", "")], true, 100)?;
                put_listing(conn, "d", &[file("c", "c.mkv", "d")], true, 100)?;
                apply_change(conn, Change::Trash(&["d".to_owned()]), 500)?;
                super::super::store::meta_set(conn, "trash_fetched_at", "500")
            })
            .expect("写入");

        let fresh = store
            .with_conn(|conn| trash_listing_if_fresh(conn, 510, 30))
            .expect("读快照")
            .expect("快照应新鲜");
        assert_eq!(fresh.len(), 1, "本地递归移入的子孙不重复展示");
        assert_eq!(fresh[0].entry.id, "d");
        assert_eq!(fresh[0].parent_id, "");
        assert_eq!(fresh[0].deleted_at, DateTime::from_timestamp(500, 0));

        assert!(
            store
                .with_conn(|conn| trash_listing_if_fresh(conn, 531, 30))
                .expect("读过期")
                .is_none(),
            "超过窗口要回网络"
        );
        assert!(store
            .with_conn(|conn| trash_listing_if_fresh(conn, 510, 30))
            .expect("读快照")
            .is_some());
    });
}

#[test]
fn search_with_trash_returns_both_live_and_trashed_rows() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(
                    conn,
                    "",
                    &[folder("d", "动画", ""), file("a", "a.mkv", "")],
                    true,
                    100,
                )?;
                put_listing(conn, "d", &[file("c", "c 动画.mkv", "d")], true, 100)?;
                apply_change(conn, Change::Trash(&["d".to_owned()]), 500)
            })
            .expect("写入");

        let live = store
            .with_conn(|conn| search_with_trash(conn, "a.mkv"))
            .expect("搜在库");
        assert_eq!(live.len(), 1);
        assert!(!live[0].trashed);
        assert_eq!(live[0].deleted_at, None);

        let trashed = store
            .with_conn(|conn| search_with_trash(conn, "动画"))
            .expect("搜回收站");
        let ids: Vec<&str> = trashed.iter().map(|hit| hit.entry.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "d"], "在库与回收站行都要命中");
        let child = trashed
            .iter()
            .find(|hit| hit.entry.id == "c")
            .expect("子项命中");
        assert!(child.trashed, "递归移入的子孙也带回收站标记");
        assert_eq!(child.entry.parent_id, "d", "保留原始父目录");
        assert_eq!(child.deleted_at, DateTime::from_timestamp(500, 0));
    });
}

#[test]
fn folder_paths_resolve_original_location_behind_trashed_ancestors() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(conn, "", &[folder("bangumi", "追番", "")], true, 100)?;
                put_listing(
                    conn,
                    "bangumi",
                    &[folder("anime", "动画", "bangumi")],
                    true,
                    100,
                )?;
                put_listing(
                    conn,
                    "anime",
                    &[folder("season", "第 1 季", "anime")],
                    true,
                    100,
                )?;
                put_listing(
                    conn,
                    "season",
                    &[file("e01", "第 01 集.mkv", "season")],
                    true,
                    100,
                )?;
                apply_change(conn, Change::Trash(&["season".to_owned()]), 500)
            })
            .expect("写入");

        let paths = store
            .with_conn(|conn| folder_paths(conn, &["season".to_owned(), "anime".to_owned()]))
            .expect("读路径");
        assert_eq!(
            paths.get("season").map(String::as_str),
            Some("追番/动画/第 1 季"),
            "祖先也在回收站时仍要拼出原始位置"
        );
        assert_eq!(paths.get("anime").map(String::as_str), Some("追番/动画"));

        let missing = store
            .with_conn(|conn| folder_paths(conn, &["missing".to_owned()]))
            .expect("读缺失路径");
        assert!(missing.is_empty(), "缺链时不拿 id 冒充路径");
        let root = store
            .with_conn(|conn| folder_paths(conn, &[String::new()]))
            .expect("读根目录");
        assert!(root.is_empty(), "根目录由界面自己显示");
    });
}

#[test]
fn folder_paths_stop_at_cycles_and_drop_the_whole_chain() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO nodes(id, parent_id, name, norm_name, kind) \
                     VALUES('x', 'y', 'x', 'x', 1), ('y', 'x', 'y', 'y', 1)",
                    [],
                )
            })
            .expect("写入环");
        let paths = store
            .with_conn(|conn| folder_paths(conn, &["x".to_owned(), "y".to_owned()]))
            .expect("读路径");
        assert!(paths.is_empty(), "环形父链不能返回半个路径");
    });
}

#[test]
fn listing_any_ignores_freshness_but_needs_known_rows() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(conn, "dir", &[file("a", "a.mkv", "dir")], true, 1_000)?;
                put_listing(
                    conn,
                    "partial",
                    &[file("b", "b.mkv", "partial")],
                    false,
                    2_000,
                )
            })
            .expect("写入");

        let stale = store
            .with_conn(|conn| listing_any(conn, "dir"))
            .expect("读旧列表")
            .expect("过期也要能先渲染旧列表");
        assert_eq!(names(&stale), vec!["a.mkv"]);

        let partial = store
            .with_conn(|conn| listing_any(conn, "partial"))
            .expect("读部分列表")
            .expect("未补全的目录也要能先渲染已知条目");
        assert_eq!(names(&partial), vec!["b.mkv"]);

        assert!(
            store
                .with_conn(|conn| listing_any(conn, "other"))
                .expect("读未知目录")
                .is_none(),
            "从没见过的目录仍要显示加载态"
        );
    });
}

#[test]
fn listing_any_returns_none_for_an_unknown_empty_directory() {
    with_store(|store| {
        store
            .with_conn(|conn| put_listing(conn, "empty", &[], true, 1_000))
            .expect("写入");
        assert!(
            store
                .with_conn(|conn| listing_any(conn, "empty"))
                .expect("读空目录")
                .is_none(),
            "没有行可渲染时让调用方显示加载态"
        );
    });
}

#[test]
fn local_file_bytes_sums_live_files_only() {
    with_store(|store| {
        let mut a = file("a", "a.mkv", "");
        a.size = 100;
        let mut b = file("b", "b.mkv", "");
        b.size = 250;
        store
            .with_conn(|conn| {
                put_listing(conn, "", &[a, b, folder("d", "动画", "")], true, 1_000)?;
                conn.execute("UPDATE nodes SET trashed = 1 WHERE id = 'b'", [])
            })
            .expect("写入");
        assert_eq!(
            store.with_conn(local_file_bytes).expect("读大小和"),
            100,
            "回收站文件与文件夹都不占配额"
        );
    });
}

#[test]
fn move_targets_returns_only_live_subfolders_with_state() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(
                    conn,
                    "dir",
                    &[
                        file("f", "a.mkv", "dir"),
                        folder("a", "A", "dir"),
                        folder("b", "b", "dir"),
                        folder("c", "C", "dir"),
                    ],
                    true,
                    1_000,
                )?;
                conn.execute("UPDATE nodes SET trashed = 1 WHERE id = 'c'", [])
            })
            .expect("写入");

        let fresh = store
            .with_conn(|conn| move_targets(conn, "dir", 1_020, 30))
            .expect("读选择器");
        assert!(fresh.known);
        assert!(fresh.fresh, "完整、干净且 30 秒内应直接渲染");
        assert_eq!(
            names(&fresh.folders),
            vec!["A", "b"],
            "只留在库文件夹，按名字排序"
        );

        let stale = store
            .with_conn(|conn| move_targets(conn, "dir", 1_031, 30))
            .expect("读过期");
        assert!(stale.known, "目录已知时可以先用旧行占位");
        assert!(!stale.fresh);

        store
            .with_conn(|conn| conn.execute("UPDATE dirs SET dirty = 1 WHERE id = 'dir'", []))
            .expect("置脏");
        let dirty = store
            .with_conn(|conn| move_targets(conn, "dir", 1_010, 30))
            .expect("读脏目录");
        assert!(dirty.known);
        assert!(!dirty.fresh, "脏目录必须后台补全");

        let unknown = store
            .with_conn(|conn| move_targets(conn, "other", 1_010, 30))
            .expect("读未知目录");
        assert!(!unknown.known);
        assert!(!unknown.fresh);
        assert!(unknown.folders.is_empty());
    });
}

#[test]
fn move_targets_exposes_partial_rows_but_not_freshness() {
    with_store(|store| {
        store
            .with_conn(|conn| put_listing(conn, "new", &[folder("n", "新建", "new")], false, 2_000))
            .expect("写入");
        let view = store
            .with_conn(|conn| move_targets(conn, "new", 2_010, 30))
            .expect("读部分目录");
        assert!(view.known, "写入过一批行就算已知目录");
        assert!(!view.fresh, "不完整的一批数据不能当作新鲜快照");
        assert_eq!(names(&view.folders), vec!["新建"]);
    });
}

#[test]
fn freshness_requires_complete_clean_and_recent() {
    with_store(|store| {
        store
            .with_conn(|conn| put_listing(conn, "dir", &[file("a", "a", "dir")], true, 1_000))
            .expect("写入");
        let fresh = store
            .with_conn(|conn| listing_if_fresh(conn, "dir", 1_020, 30))
            .expect("读新鲜度");
        assert!(fresh.is_some(), "30 秒内应直接渲染");
        let stale = store
            .with_conn(|conn| listing_if_fresh(conn, "dir", 1_031, 30))
            .expect("读过期");
        assert!(stale.is_none());
        store
            .with_conn(|conn| conn.execute("UPDATE dirs SET dirty = 1 WHERE id = 'dir'", []))
            .expect("置脏");
        let dirty = store
            .with_conn(|conn| listing_if_fresh(conn, "dir", 1_010, 30))
            .expect("读脏目录");
        assert!(dirty.is_none());
        let unknown = store
            .with_conn(|conn| listing_if_fresh(conn, "other", 1_010, 30))
            .expect("读未知目录");
        assert!(unknown.is_none());
    });
}
