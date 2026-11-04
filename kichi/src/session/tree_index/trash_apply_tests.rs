//! `apply` 的回收站子树写穿测试。

use super::super::store::Store;
use super::super::test_support::{file, folder};
use super::*;

fn with_store<T>(f: impl FnOnce(&Store) -> T) -> T {
    let store = Store::open_in_memory("u1").expect("打开内存库");
    let value = f(&store);
    store.close();
    value
}

fn count(store: &Store, sql: &str) -> i64 {
    store
        .with_conn(|conn| conn.query_row(sql, [], |row| row.get(0)))
        .expect("计数")
}

#[test]
fn trashing_a_folder_marks_the_whole_subtree() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(conn, "", &[folder("d", "动画", "")], true, 100)?;
                put_listing(
                    conn,
                    "d",
                    &[folder("sub", "第 1 季", "d"), file("c", "c.mkv", "d")],
                    true,
                    100,
                )?;
                put_listing(conn, "sub", &[file("g", "g.mkv", "sub")], true, 100)
            })
            .expect("写入");
        store
            .with_conn(|conn| apply_change(conn, Change::Trash(&["d".to_owned()]), 500))
            .expect("移入回收站");

        let (trashed, deleted): (i64, Option<i64>) = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT SUM(trashed), MAX(deleted_at) FROM nodes",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .expect("读回收站状态");
        assert_eq!(trashed, 4, "文件夹、子文件夹与孙子文件都要一起移入回收站");
        assert_eq!(deleted, Some(500));

        let (parent, trashed): (String, i64) = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT parent_id, trashed FROM nodes WHERE id = 'c'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .expect("读子项");
        assert_eq!(parent, "d", "回收站行保留原始父目录");
        assert_eq!(trashed, 1);

        let dirty: i64 = store
            .with_conn(|conn| {
                conn.query_row("SELECT dirty FROM dirs WHERE id = ''", [], |row| row.get(0))
            })
            .expect("读根目录状态");
        assert_eq!(dirty, 1, "原父目录要标脏");
    });
}

#[test]
fn restoring_a_folder_brings_back_the_whole_subtree() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(conn, "", &[folder("d", "动画", "")], true, 100)?;
                put_listing(conn, "d", &[file("c", "c.mkv", "d")], true, 100)?;
                apply_change(conn, Change::Trash(&["d".to_owned()]), 500)
            })
            .expect("写入并移入回收站");
        store
            .with_conn(|conn| apply_change(conn, Change::Restore(&["d".to_owned()]), 600))
            .expect("还原");

        let (trashed, deleted): (i64, i64) = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT SUM(trashed), COUNT(deleted_at) FROM nodes",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .expect("读状态");
        assert_eq!(trashed, 0, "整棵子树都要回到在库状态");
        assert_eq!(deleted, 0, "还原后不再保留删除时间");
        let parent: String = store
            .with_conn(|conn| {
                conn.query_row("SELECT parent_id FROM nodes WHERE id = 'c'", [], |row| {
                    row.get(0)
                })
            })
            .expect("读子项");
        assert_eq!(parent, "d");
    });
}

#[test]
fn removing_a_folder_deletes_the_whole_subtree() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(conn, "", &[folder("d", "动画", "")], true, 100)?;
                put_listing(conn, "d", &[file("c", "c.mkv", "d")], true, 100)?;
                apply_change(conn, Change::Remove(&["d".to_owned()]), 500)
            })
            .expect("彻底删除");
        assert_eq!(count(store, "SELECT COUNT(*) FROM nodes"), 0);
    });
}

#[test]
fn removing_all_trashed_clears_every_trashed_row() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(
                    conn,
                    "a",
                    &[folder("d", "动画", "a"), file("keep", "keep.mkv", "a")],
                    true,
                    100,
                )?;
                put_listing(conn, "b", &[file("gone", "gone.mkv", "b")], true, 100)?;
                apply_change(
                    conn,
                    Change::Trash(&["d".to_owned(), "gone".to_owned()]),
                    500,
                )
            })
            .expect("写入");
        store
            .with_conn(|conn| apply_change(conn, Change::RemoveAllTrashed, 600))
            .expect("清空");

        assert_eq!(
            count(store, "SELECT COUNT(*) FROM nodes WHERE trashed = 1"),
            0
        );
        assert_eq!(
            count(store, "SELECT COUNT(*) FROM nodes"),
            1,
            "在库条目保留"
        );
        let dirty: i64 = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM dirs WHERE dirty = 1 AND id IN ('a', 'b')",
                    [],
                    |row| row.get(0),
                )
            })
            .expect("数脏目录");
        assert_eq!(dirty, 2, "每个被删条目的原父目录都要标脏");
    });
}

#[test]
fn replace_trash_rebuilds_the_snapshot() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(conn, "", &[folder("old", "旧回收站", "")], true, 100)?;
                apply_change(conn, Change::Trash(&["old".to_owned()]), 200)
            })
            .expect("旧快照");

        let entries = [TrashEntry {
            entry: file("fresh", "新回收站.mkv", ""),
            parent_id: "original".to_owned(),
            deleted_at: DateTime::from_timestamp(900, 0),
        }];
        store
            .with_conn(|conn| replace_trash(conn, &entries, 950))
            .expect("新快照");

        assert_eq!(
            count(store, "SELECT COUNT(*) FROM nodes WHERE id = 'old'"),
            0,
            "不在新快照里的旧回收站行要删掉"
        );
        let (parent, deleted, trashed): (String, Option<i64>, i64) = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT parent_id, deleted_at, trashed FROM nodes WHERE id = 'fresh'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
            })
            .expect("读新条目");
        assert_eq!(parent, "original", "父目录取自 TrashEntry.parent_id");
        assert_eq!(deleted, Some(900));
        assert_eq!(trashed, 1);
        let fetched = store
            .with_conn(|conn| super::super::store::meta_get(conn, "trash_fetched_at"))
            .expect("读快照时间");
        assert_eq!(fetched.as_deref(), Some("950"));
    });
}

#[test]
fn apply_change_keeps_parent_when_trashing() {
    with_store(|store| {
        store
            .with_conn(|conn| put_listing(conn, "dir", &[file("a", "a.mkv", "dir")], true, 100))
            .expect("写入");
        store
            .with_conn(|conn| apply_change(conn, Change::Trash(&["a".to_owned()]), 500))
            .expect("回收站");
        let (trashed, parent): (i64, String) = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT trashed, parent_id FROM nodes WHERE id = 'a'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .expect("读节点");
        assert_eq!(trashed, 1);
        assert_eq!(parent, "dir");
    });
}

#[test]
fn local_trash_actions_invalidate_the_trash_snapshot() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(conn, "", &[file("a", "a.mkv", "")], true, 100)?;
                replace_trash(conn, &[], 100)
            })
            .expect("写入并建空快照");

        let cached = store
            .with_conn(|conn| super::super::query::trash_listing_if_fresh(conn, 110, 30))
            .expect("读快照");
        assert!(cached.is_some(), "空快照也应命中");

        store
            .with_conn(|conn| apply_change(conn, Change::Trash(&["a".to_owned()]), 120))
            .expect("移入回收站");

        let cached = store
            .with_conn(|conn| super::super::query::trash_listing_if_fresh(conn, 130, 30))
            .expect("再读快照");
        assert!(cached.is_none(), "本地移入回收站后旧快照必须失效");
    });
}
