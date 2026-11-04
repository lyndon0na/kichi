//! 本地索引的 SQLite 存储：连接、schema、版本与账号归属。
//!
//! 索引只是可重建的缓存：文件损坏或 `user_version` 不认识时直接删库重建。
//! 纯追加的表（如 `page_snapshots`）在打开时用 `IF NOT EXISTS` 补建，**不提升版本**：
//! 旧二进制看到更高版本会整库删除重建，追加表不该触发这种清库。
//! 数据库按账号分文件（见 [`super::super::paths::index_path`]），
//! 但 `KICHI_INDEX_FILE` 可能指向共享路径，所以 `meta.owner` 仍要校验。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{params, Connection};

/// schema 版本；不兼容的表结构改动必须 +1。
///
/// 纯追加表不提升版本：新旧的 kichi 二进制共用同一账号库时，互相不会把对方的库
/// 当成「未知版本」删掉重扫。
pub(super) const SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = "
CREATE TABLE nodes (
  id TEXT PRIMARY KEY,
  parent_id TEXT NOT NULL DEFAULT '',
  name TEXT NOT NULL,
  norm_name TEXT NOT NULL,
  kind INTEGER NOT NULL,
  size INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER,
  modified_at INTEGER,
  md5 TEXT,
  starred INTEGER NOT NULL DEFAULT 0,
  trashed INTEGER NOT NULL DEFAULT 0,
  deleted_at INTEGER,
  duration REAL,
  width INTEGER,
  height INTEGER
);
CREATE INDEX nodes_parent ON nodes(parent_id);
CREATE INDEX nodes_name ON nodes(norm_name);
CREATE INDEX nodes_md5 ON nodes(md5, size) WHERE md5 IS NOT NULL AND md5 <> '';
CREATE INDEX nodes_trashed ON nodes(trashed);

CREATE TABLE dirs (
  id TEXT PRIMARY KEY,
  complete INTEGER NOT NULL DEFAULT 0,
  dirty INTEGER NOT NULL DEFAULT 0,
  fetched_at INTEGER
);

CREATE TABLE meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
";

/// 页面快照表。纯追加表，打开时按需补建，不提升 `user_version`。
const PAGE_SNAPSHOTS_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS page_snapshots (
  kind TEXT PRIMARY KEY,
  payload TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
";

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("本地索引已关闭")]
    Closed,
    #[error("本地索引读写失败：{0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("本地索引文件操作失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("本地索引版本 {0} 无法识别")]
    Version(i64),
}

/// 一条连接 + 关闭标记。
///
/// 生产路径只持有 `Mutex<Option<Connection>>`；`close` 先置标记再尝试拿锁，
/// 拿不到（后台正在写）也立即逻辑关闭，等后台线程放掉 Arc 后连接自然销毁。
pub(super) struct Store {
    inner: Mutex<Option<Connection>>,
    path: Option<PathBuf>,
    closed: AtomicBool,
}

impl Store {
    /// 打开（必要时重建）指定路径的索引库。
    pub(super) fn open(path: &Path, owner: &str) -> Result<Self, IndexError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
                set_dir_permissions(parent);
            }
        }
        let conn = match Self::open_checked(path) {
            Ok(conn) => conn,
            Err(_) => {
                // 损坏 / 版本不认识 / 空文件：索引可重建，旧文件没有保留价值
                remove_db_files(path);
                Self::open_fresh(path)?
            }
        };
        let mut conn = conn;
        ensure_owner(&mut conn, owner)?;
        set_file_permissions(path);
        Ok(Self {
            inner: Mutex::new(Some(conn)),
            path: Some(path.to_path_buf()),
            closed: AtomicBool::new(false),
        })
    }

    /// 测试用的内存库。
    #[cfg(test)]
    pub(super) fn open_in_memory(owner: &str) -> Result<Self, IndexError> {
        let mut conn = Connection::open_in_memory()?;
        configure(&conn)?;
        create_schema(&conn)?;
        ensure_owner(&mut conn, owner)?;
        Ok(Self {
            inner: Mutex::new(Some(conn)),
            path: None,
            closed: AtomicBool::new(false),
        })
    }

    pub(super) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// 逻辑关闭并尽量立即释放连接（不阻塞调用方）。
    pub(super) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = self.inner.try_lock() {
            *guard = None;
        }
    }

    /// 借一次连接执行。关闭后一律返回 [`IndexError::Closed`]。
    pub(super) fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, IndexError> {
        if self.is_closed() {
            return Err(IndexError::Closed);
        }
        let guard = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        // 等锁期间可能已经关闭：拿锁后再看一次
        if self.is_closed() {
            return Err(IndexError::Closed);
        }
        let conn = guard.as_ref().ok_or(IndexError::Closed)?;
        f(conn).map_err(IndexError::from)
    }

    /// 数据库路径；内存库为 `None`（测试用）。
    #[cfg(test)]
    pub(super) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn open_checked(path: &Path) -> Result<Connection, IndexError> {
        let conn = Connection::open(path)?;
        configure(&conn)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            return Err(IndexError::Version(version));
        }
        // 追加表按需补建；`IF NOT EXISTS` 对已建好的库是空操作。
        conn.execute_batch(PAGE_SNAPSHOTS_SCHEMA)?;
        Ok(conn)
    }

    fn open_fresh(path: &Path) -> Result<Connection, IndexError> {
        let conn = Connection::open(path)?;
        configure(&conn)?;
        create_schema(&conn)?;
        Ok(conn)
    }
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_millis(2000))?;
    // WAL：全量建库的大事务提交前，读线程仍能看旧快照
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch(PAGE_SNAPSHOTS_SCHEMA)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// 同一个共享文件被另一个账号打开：旧数据全部作废。
fn ensure_owner(conn: &mut Connection, owner: &str) -> Result<(), IndexError> {
    if meta_get(conn, "owner")?.as_deref() == Some(owner) {
        return Ok(());
    }
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM nodes", [])?;
    tx.execute("DELETE FROM dirs", [])?;
    tx.execute("DELETE FROM meta", [])?;
    tx.execute("DELETE FROM page_snapshots", [])?;
    tx.execute("INSERT INTO meta(key, value) VALUES('owner', ?1)", [owner])?;
    tx.commit()?;
    Ok(())
}

/// 页面快照按 `kind` 覆盖写；数据是可丢弃的展示缓存。
pub(super) fn put_page_snapshot(
    conn: &Connection,
    kind: &str,
    payload: &str,
    updated_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO page_snapshots(kind, payload, updated_at) VALUES(?1, ?2, ?3) \
         ON CONFLICT(kind) DO UPDATE SET payload = excluded.payload, \
         updated_at = excluded.updated_at",
        params![kind, payload, updated_at],
    )?;
    Ok(())
}

pub(super) fn page_snapshot(
    conn: &Connection,
    kind: &str,
) -> rusqlite::Result<Option<(String, i64)>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT payload, updated_at FROM page_snapshots WHERE kind = ?1",
        [kind],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

pub(super) fn meta_get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    use rusqlite::OptionalExtension;
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .optional()
}

pub(super) fn meta_set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES(?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

fn remove_db_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    // WAL 模式的附属文件；主库删掉后它们已无意义
    for suffix in ["-wal", "-shm"] {
        let mut side = path.as_os_str().to_owned();
        side.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(side));
    }
}

#[cfg(unix)]
fn set_dir_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_dir_permissions(_path: &Path) {}

#[cfg(unix)]
fn set_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_file_permissions(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("kichi-index-{tag}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    fn schema_version(store: &Store) -> i64 {
        store
            .with_conn(|conn| conn.pragma_query_value(None, "user_version", |row| row.get(0)))
            .expect("读 user_version")
    }

    #[test]
    fn fresh_database_gets_the_schema_version() {
        let store = Store::open_in_memory("u1").expect("打开内存库");
        assert_eq!(schema_version(&store), SCHEMA_VERSION);
        let owner = store
            .with_conn(|conn| meta_get(conn, "owner"))
            .expect("读 owner")
            .expect("owner 应已写入");
        assert_eq!(owner, "u1");
    }

    #[test]
    fn page_snapshot_upserts_and_reads_back() {
        let store = Store::open_in_memory("u1").expect("打开内存库");
        store
            .with_conn(|conn| put_page_snapshot(conn, "library", "v1", 1))
            .expect("写快照");
        assert_eq!(
            store
                .with_conn(|conn| page_snapshot(conn, "library"))
                .expect("读快照"),
            Some(("v1".to_owned(), 1))
        );
        store
            .with_conn(|conn| put_page_snapshot(conn, "library", "v2", 2))
            .expect("覆盖快照");
        assert_eq!(
            store
                .with_conn(|conn| page_snapshot(conn, "library"))
                .expect("读快照"),
            Some(("v2".to_owned(), 2))
        );
        assert_eq!(
            store
                .with_conn(|conn| page_snapshot(conn, "missing"))
                .expect("读空快照"),
            None
        );
    }

    #[test]
    fn switching_owner_wipes_page_snapshots() {
        let dir = temp_dir("page-owner");
        let path = dir.join("index.sqlite3");
        {
            let store = Store::open(&path, "owner-a").expect("首次打开");
            store
                .with_conn(|conn| put_page_snapshot(conn, "library", "old", 1))
                .expect("写快照");
            store.close();
        }
        let store = Store::open(&path, "owner-b").expect("换账号打开");
        assert_eq!(
            store
                .with_conn(|conn| page_snapshot(conn, "library"))
                .expect("读快照"),
            None,
            "换账号不能看到上一个账号的页面快照"
        );
        store.close();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn additive_table_is_created_without_bumping_version() {
        let dir = temp_dir("additive-upgrade");
        let path = dir.join("index.sqlite3");
        {
            let store = Store::open(&path, "u1").expect("首次打开");
            store
                .with_conn(|conn| {
                    conn.execute(
                        "INSERT INTO nodes(id, parent_id, name, norm_name, kind) \
                         VALUES('n1', '', 'x', 'x', 0)",
                        [],
                    )?;
                    conn.execute_batch("DROP TABLE page_snapshots;")
                })
                .expect("造旧库");
            store.close();
        }
        let store = Store::open(&path, "u1").expect("旧库应原地补表");
        assert_eq!(
            schema_version(&store),
            SCHEMA_VERSION,
            "追加表不能提升版本，否则旧二进制会把新库当未知版本删掉"
        );
        let count: i64 = store
            .with_conn(|conn| conn.query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0)))
            .expect("数节点");
        assert_eq!(count, 1, "补表不能丢掉已经建好的目录索引");
        store
            .with_conn(|conn| put_page_snapshot(conn, "library", "v", 1))
            .expect("补出的快照表可用");
        store.close();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_version_is_deleted_and_rebuilt() {
        let dir = temp_dir("version");
        let path = dir.join("index.sqlite3");
        {
            let conn = Connection::open(&path).expect("建旧库");
            conn.execute_batch("CREATE TABLE marker(x); PRAGMA user_version = 99;")
                .expect("写旧版本");
        }
        let store = Store::open(&path, "u1").expect("旧版本应重建");
        assert_eq!(schema_version(&store), SCHEMA_VERSION);
        let has_marker: i64 = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = 'marker'",
                    [],
                    |row| row.get(0),
                )
            })
            .expect("查 marker");
        assert_eq!(has_marker, 0, "旧表必须随重建消失");
        store.close();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_file_is_deleted_and_rebuilt() {
        let dir = temp_dir("corrupt");
        let path = dir.join("index.sqlite3");
        std::fs::write(&path, b"this is not a sqlite database").expect("写坏文件");
        let store = Store::open(&path, "u1").expect("坏库应重建");
        assert_eq!(schema_version(&store), SCHEMA_VERSION);
        assert_eq!(store.path(), Some(path.as_path()));
        store.close();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn opening_with_another_owner_wipes_the_index() {
        let dir = temp_dir("owner");
        let path = dir.join("index.sqlite3");
        {
            let store = Store::open(&path, "owner-a").expect("首次打开");
            store
                .with_conn(|conn| {
                    conn.execute(
                        "INSERT INTO nodes(id, parent_id, name, norm_name, kind) \
                         VALUES('n1', '', 'x', 'x', 0)",
                        [],
                    )
                })
                .expect("写一条");
            store.close();
        }
        let store = Store::open(&path, "owner-b").expect("换账号打开");
        let count: i64 = store
            .with_conn(|conn| conn.query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0)))
            .expect("数节点");
        assert_eq!(count, 0, "共享路径换了账号必须清空旧数据");
        let owner = store
            .with_conn(|conn| meta_get(conn, "owner"))
            .expect("读 owner");
        assert_eq!(owner.as_deref(), Some("owner-b"));
        store.close();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn database_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        let path = dir.join("index.sqlite3");
        let store = Store::open(&path, "u1").expect("打开");
        let mode = std::fs::metadata(&path)
            .expect("读权限")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        store.close();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn closed_store_rejects_reads() {
        let store = Store::open_in_memory("u1").expect("打开内存库");
        store.close();
        assert!(matches!(
            store.with_conn(|conn| conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))),
            Err(IndexError::Closed)
        ));
    }
}
