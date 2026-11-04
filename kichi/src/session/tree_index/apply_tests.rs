//! `apply` 的目录列表、改名、移动与标脏测试。

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
fn complete_listing_replaces_live_rows() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(
                    conn,
                    "",
                    &[file("a", "a.mkv", ""), file("b", "b.mkv", "")],
                    true,
                    100,
                )
            })
            .expect("首批");
        store
            .with_conn(|conn| {
                put_listing(
                    conn,
                    "",
                    &[file("a", "a.mkv", ""), file("c", "c.mkv", "")],
                    true,
                    200,
                )
            })
            .expect("第二批");
        assert_eq!(count(store, "SELECT COUNT(*) FROM nodes"), 2);
        let b: i64 = store
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM nodes WHERE id = 'b'", [], |row| {
                    row.get(0)
                })
            })
            .expect("查 b");
        assert_eq!(b, 0, "完整列表里消失的条目要删掉");
    });
}

#[test]
fn incomplete_listing_never_deletes() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(
                    conn,
                    "",
                    &[file("a", "a.mkv", ""), file("b", "b.mkv", "")],
                    true,
                    100,
                )
            })
            .expect("完整列表");
        store
            .with_conn(|conn| put_listing(conn, "", &[file("a", "a.mkv", "")], false, 200))
            .expect("局部列表");
        assert_eq!(count(store, "SELECT COUNT(*) FROM nodes"), 2, "b 不能被删");
    });
}

#[test]
fn trash_rows_survive_a_complete_listing() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(conn, "dir", &[file("a", "a.mkv", "dir")], true, 100)?;
                conn.execute(
                    "INSERT INTO nodes(id, parent_id, name, norm_name, kind, trashed) \
                         VALUES('t', 'dir', 'gone.mkv', 'gone.mkv', 0, 1)",
                    [],
                )?;
                Ok(())
            })
            .expect("写回收站行");
        store
            .with_conn(|conn| put_listing(conn, "dir", &[file("b", "b.mkv", "dir")], true, 200))
            .expect("刷新");
        let trashed: i64 = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM nodes WHERE id = 't' AND trashed = 1 \
                         AND parent_id = 'dir'",
                    [],
                    |row| row.get(0),
                )
            })
            .expect("查回收站行");
        assert_eq!(trashed, 1, "回收站行要保留原 parent_id");
    });
}

#[test]
fn incomplete_listing_does_not_extend_a_complete_snapshot() {
    with_store(|store| {
        store
            .with_conn(|conn| put_listing(conn, "dir", &[file("a", "a", "dir")], true, 100))
            .expect("完整快照");
        store
            .with_conn(|conn| put_listing(conn, "dir", &[file("b", "b", "dir")], false, 900))
            .expect("局部写入");
        let (complete, fetched_at): (i64, i64) = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT complete, fetched_at FROM dirs WHERE id = 'dir'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .expect("读目录状态");
        assert_eq!(complete, 1);
        assert_eq!(fetched_at, 100, "不完整数据不能延长完整快照的新鲜期");
    });
}

#[test]
fn marking_a_dir_dirty_invalidates_its_snapshot() {
    with_store(|store| {
        store
            .with_conn(|conn| put_listing(conn, "dir", &[file("a", "a", "dir")], true, 1_000))
            .expect("写入");
        assert!(store
            .with_conn(|conn| super::super::query::listing_if_fresh(conn, "dir", 1_010, 30))
            .expect("读新鲜度")
            .is_some());
        store
            .with_conn(|conn| mark_dir_dirty(conn, "dir"))
            .expect("标脏");
        assert!(store
            .with_conn(|conn| super::super::query::listing_if_fresh(conn, "dir", 1_010, 30))
            .expect("读脏目录")
            .is_none());
    });
}

#[test]
fn rename_updates_the_normalized_name() {
    with_store(|store| {
        store
            .with_conn(|conn| put_listing(conn, "", &[file("a", "old.txt", "")], true, 100))
            .expect("写入");
        store
            .with_conn(|conn| {
                apply_change(
                    conn,
                    Change::Rename {
                        id: "a",
                        name: "測試.TXT",
                    },
                    500,
                )
            })
            .expect("改名");
        let (name, norm): (String, String) = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT name, norm_name FROM nodes WHERE id = 'a'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .expect("读节点");
        assert_eq!(name, "測試.TXT");
        assert_eq!(norm, "测试.txt");
    });
}

#[test]
fn move_updates_parent_and_marks_both_dirs() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                put_listing(conn, "old", &[folder("d", "动画", "old")], true, 100)?;
                put_listing(conn, "d", &[file("c", "c.mkv", "d")], true, 100)?;
                put_listing(conn, "new", &[], true, 100)
            })
            .expect("写入");
        store
            .with_conn(|conn| {
                apply_change(
                    conn,
                    Change::Move {
                        ids: &["d".to_owned()],
                        parent_id: "new",
                    },
                    500,
                )
            })
            .expect("移动");
        let moved: String = store
            .with_conn(|conn| {
                conn.query_row("SELECT parent_id FROM nodes WHERE id = 'd'", [], |row| {
                    row.get(0)
                })
            })
            .expect("读节点");
        assert_eq!(moved, "new");
        let child: String = store
            .with_conn(|conn| {
                conn.query_row("SELECT parent_id FROM nodes WHERE id = 'c'", [], |row| {
                    row.get(0)
                })
            })
            .expect("读子项");
        assert_eq!(child, "d", "移动文件夹不能改子孙的父目录");
        let dirty: i64 = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM dirs WHERE dirty = 1 AND id IN ('old', 'new')",
                    [],
                    |row| row.get(0),
                )
            })
            .expect("数脏目录");
        assert_eq!(dirty, 2);
    });
}

#[test]
fn starring_updates_every_given_id_in_one_change() {
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
                apply_change(
                    conn,
                    Change::Starred {
                        ids: &["a".to_owned(), "b".to_owned()],
                        starred: true,
                    },
                    500,
                )?;
                apply_change(
                    conn,
                    Change::Starred {
                        ids: &["a".to_owned()],
                        starred: false,
                    },
                    600,
                )
            })
            .expect("星标");
        let (a, b): (i64, i64) = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT (SELECT starred FROM nodes WHERE id = 'a'), \
                            (SELECT starred FROM nodes WHERE id = 'b')",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .expect("读星标");
        assert_eq!(a, 0);
        assert_eq!(b, 1);
    });
}

#[test]
fn backfilled_trail_links_parents_and_stays_live() {
    with_store(|store| {
        store
            .with_conn(|conn| {
                backfill_trail(
                    conn,
                    &[
                        ("root-folder".to_owned(), "追番".to_owned()),
                        ("season".to_owned(), "第 1 季".to_owned()),
                    ],
                )
            })
            .expect("回填");
        let (parent, kind): (String, i64) = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT parent_id, kind FROM nodes WHERE id = 'season'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .expect("读节点");
        assert_eq!(parent, "root-folder");
        assert_eq!(kind, 1, "回填的是文件夹");
    });
}

#[test]
fn folder_entries_are_stored_as_folders() {
    with_store(|store| {
        store
            .with_conn(|conn| put_listing(conn, "", &[folder("d", "动画", "")], true, 100))
            .expect("写入");
        let kind: i64 = store
            .with_conn(|conn| {
                conn.query_row("SELECT kind FROM nodes WHERE id = 'd'", [], |row| {
                    row.get(0)
                })
            })
            .expect("读 kind");
        assert_eq!(kind, 1);
    });
}
