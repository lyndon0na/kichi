//! 运行日志初始化: 同时输出到 stderr 与缓存目录下的日志文件。
//!
//! 级别由环境变量 `KICHI_LOG`(优先)或 `RUST_LOG` 控制, 例如
//! `KICHI_LOG=debug` 或 `KICHI_LOG=kichi_core=trace,warn`。
//! 未设置时默认 `info`, 并对本项目 crate 打开 `debug`。
//!
//! 日志文件**按大小轮转**: 写满 [`LOG_MAX_BYTES`] 后改名为 `kichi.log.1`,
//! 旧备份依次位移, 最多保留 [`LOG_KEEP`] 个。单文件上限与备份数量可分别用
//! `KICHI_LOG_MAX_MB` / `KICHI_LOG_FILES` 覆盖。

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

/// 单个日志文件的大小上限(默认 1 MiB)。
const LOG_MAX_BYTES: u64 = 1 << 20;
/// 保留的历史备份数量(默认 3 个)。
const LOG_KEEP: usize = 3;

/// 日志文件路径; 取不到缓存目录时回退到临时目录。
fn log_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("kichi")
        .join("kichi.log")
}

/// 环境变量覆盖的轮转上限(单位 MiB); 非法值回退默认。
fn max_bytes() -> u64 {
    env_num("KICHI_LOG_MAX_MB")
        .map(|mb| mb.clamp(1, 1024) * 1024 * 1024)
        .unwrap_or(LOG_MAX_BYTES)
}

/// 环境变量覆盖的备份数量; 上限 16, 避免误配置写出一堆文件。
fn keep_files() -> usize {
    env_num("KICHI_LOG_FILES")
        .map(|n| (n as usize).min(16))
        .unwrap_or(LOG_KEEP)
}

fn env_num(key: &str) -> Option<u64> {
    std::env::var(key).ok()?.trim().parse().ok()
}

/// 第 n 个备份的路径: `kichi.log` -> `kichi.log.1` / `.2` / ...
fn backup_path(base: &Path, n: usize) -> PathBuf {
    let mut s = base.as_os_str().to_os_string();
    s.push(format!(".{n}"));
    PathBuf::from(s)
}

/// 把 `base` 轮转为 `base.1`(`.1` -> `.2` ... 最旧的丢弃)。
/// 调用方必须先关掉自己的文件句柄, 再调用这里。
fn rotate(base: &Path, keep: usize) {
    if keep == 0 {
        let _ = std::fs::remove_file(base);
        return;
    }
    let _ = std::fs::remove_file(backup_path(base, keep));
    for i in (1..keep).rev() {
        let _ = std::fs::rename(backup_path(base, i), backup_path(base, i + 1));
    }
    let _ = std::fs::rename(base, backup_path(base, 1));
}

/// 以追加方式打开日志文件; 失败时返回 None(此时只往 stderr 写)。
fn open_append(path: &Path) -> Option<std::fs::File> {
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// 按大小轮转的日志文件。
struct LogSink {
    path: PathBuf,
    /// 当前文件句柄; 写失败或轮转失败后为 None(之后只写 stderr)。
    file: Option<std::fs::File>,
    /// 当前文件已写字节数, 用于在不 stat 的前提下判断是否写满。
    written: u64,
    max_bytes: u64,
    keep: usize,
}

impl LogSink {
    fn new(path: PathBuf, max_bytes: u64, keep: usize) -> Self {
        // 兜底轮转: 上次运行可能在写满后立刻退出(轮转只发生在写入时)。
        if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) >= max_bytes {
            rotate(&path, keep);
        }
        let file = open_append(&path);
        // 已写字节以文件实际大小为准, 跨重启接着算。
        let written = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Self {
            path,
            file,
            written,
            max_bytes,
            keep,
        }
    }

    fn write(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.maybe_rotate(buf);
        let Some(f) = self.file.as_mut() else {
            return Ok(());
        };
        match f.write_all(buf) {
            Ok(()) => {
                self.written += buf.len() as u64;
                Ok(())
            }
            Err(e) => {
                // 磁盘满 / 句柄失效: 放弃文件输出, 之后只走 stderr, 不再反复报错。
                self.file = None;
                Err(e)
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }

    /// 写满时轮转。**只在行尾切**: tracing 的 fmt 层写一条 event 会分多次
    /// `write` 调用, 在行中间轮转会把同一条日志劈到两个文件里。
    fn maybe_rotate(&mut self, buf: &[u8]) {
        if self.written + buf.len() as u64 <= self.max_bytes {
            return;
        }
        if buf.last() != Some(&b'\n') {
            return;
        }
        // 当前文件还是空的(单条日志本身就超限): 没什么可轮转的, 直接写。
        if self.written == 0 {
            return;
        }
        // 先关句柄再改名, 避免 rename 与写入交错; 失败则退化为只写 stderr。
        self.file = None;
        rotate(&self.path, self.keep);
        self.file = open_append(&self.path);
        self.written = 0;
    }
}

/// 同时写 stderr 与日志文件的 `MakeWriter`。
#[derive(Clone)]
struct TeeWriter {
    sink: Arc<Mutex<LogSink>>,
}

struct Tee<'a> {
    sink: &'a Arc<Mutex<LogSink>>,
}

impl<'a> MakeWriter<'a> for TeeWriter {
    type Writer = Tee<'a>;
    fn make_writer(&'a self) -> Self::Writer {
        Tee { sink: &self.sink }
    }
}

impl std::io::Write for Tee<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        if let Ok(mut s) = self.sink.lock() {
            let _ = s.write(buf);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        if let Ok(mut s) = self.sink.lock() {
            let _ = s.flush();
        }
        Ok(())
    }
}

fn filter() -> EnvFilter {
    std::env::var("KICHI_LOG")
        .ok()
        .or_else(|| std::env::var("RUST_LOG").ok())
        .and_then(|s| EnvFilter::try_new(&s).ok())
        .unwrap_or_else(|| EnvFilter::new("info,kichi_core=debug,kichi_gui=debug"))
}

/// 初始化全局日志(只调用一次)。
pub fn init() {
    let path = log_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // 追加写; 写满按大小轮转(见 LogSink), 日志目录整体占用是有界的。
    let sink = LogSink::new(path.clone(), max_bytes(), keep_files());

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter())
        .with_ansi(false)
        .with_target(true)
        .with_level(true);

    if sink.file.is_some() {
        let writer = TeeWriter {
            sink: Arc::new(Mutex::new(sink)),
        };
        builder.with_writer(writer).init();
    } else {
        // 日志文件打不开(目录只读等): 退化为只写 stderr。
        builder.with_writer(std::io::stderr).init();
    }

    tracing::info!("日志已启动, 文件: {}", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个用例独立的临时目录(不引额外依赖, 用例内自行清理)。
    fn tmp_base(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kichi-log-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir.join("kichi.log")
    }

    fn clean(base: &Path) {
        if let Some(dir) = base.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn rotate_shifts_backups_and_drops_oldest() {
        let base = tmp_base("rotate");
        std::fs::write(&base, "cur").unwrap();
        std::fs::write(backup_path(&base, 1), "b1").unwrap();
        std::fs::write(backup_path(&base, 2), "b2").unwrap();

        rotate(&base, 2);

        // 原文件被挪走, 备份依次后移, 最旧的被丢弃。
        assert!(!base.exists());
        assert_eq!(
            std::fs::read_to_string(backup_path(&base, 1)).unwrap(),
            "cur"
        );
        assert_eq!(
            std::fs::read_to_string(backup_path(&base, 2)).unwrap(),
            "b1"
        );
        clean(&base);
    }

    #[test]
    fn sink_rotates_only_at_line_boundary() {
        let base = tmp_base("boundary");
        let mut sink = LogSink::new(base.clone(), 8, 1);

        sink.write(b"123456\n").unwrap();
        assert!(!backup_path(&base, 1).exists(), "未写满不轮转");

        // 已超限, 但这条不在行尾: 先攒着, 不切分日志行。
        sink.write(b"ab").unwrap();
        assert!(!backup_path(&base, 1).exists(), "行中间不轮转");

        // 行尾且超限: 先轮转, 再把这行写进新文件。
        sink.write(b"\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(backup_path(&base, 1)).unwrap(),
            "123456\nab"
        );
        assert_eq!(std::fs::read_to_string(&base).unwrap(), "\n");
        clean(&base);
    }

    #[test]
    fn new_rotates_oversized_previous_file() {
        let base = tmp_base("startup");
        // 上次运行留下的日志已经超过上限, 启动时应先轮转再续写。
        std::fs::write(&base, "0123456789").unwrap();

        let sink = LogSink::new(base.clone(), 8, 1);

        assert!(sink.file.is_some());
        assert_eq!(
            std::fs::read_to_string(backup_path(&base, 1)).unwrap(),
            "0123456789"
        );
        assert_eq!(std::fs::metadata(&base).unwrap().len(), 0);
        assert_eq!(sink.written, 0);
        clean(&base);
    }
}
