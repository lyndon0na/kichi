//! 运行日志初始化: 同时输出到 stderr 与缓存目录下的日志文件。
//!
//! 级别由环境变量 `KICHI_LOG`(优先)或 `RUST_LOG` 控制, 例如
//! `KICHI_LOG=debug` 或 `KICHI_LOG=kichi_core=trace,warn`。
//! 未设置时默认 `info`, 并对本项目 crate 打开 `debug`。

use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

/// 日志文件路径; 取不到缓存目录时回退到临时目录。
fn log_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("kichi")
        .join("kichi.log")
}

/// 同时写 stderr 与日志文件的 `MakeWriter`。
#[derive(Clone)]
struct TeeWriter {
    file: Arc<Mutex<std::fs::File>>,
}

struct Tee<'a> {
    file: &'a Arc<Mutex<std::fs::File>>,
}

impl<'a> MakeWriter<'a> for TeeWriter {
    type Writer = Tee<'a>;
    fn make_writer(&'a self) -> Self::Writer {
        Tee { file: &self.file }
    }
}

impl std::io::Write for Tee<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(buf);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
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
    // 追加写, 不做轮转(日志量小)。
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter())
        .with_ansi(false)
        .with_target(true)
        .with_level(true);

    match file {
        Some(f) => {
            let writer = TeeWriter {
                file: Arc::new(Mutex::new(f)),
            };
            builder.with_writer(writer).init();
        }
        None => {
            builder.with_writer(std::io::stderr).init();
        }
    }

    tracing::info!("日志已启动, 文件: {}", path.display());
}
