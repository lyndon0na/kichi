//! kichi-core：PikPak 非官方 API 的纯 Rust 封装。
//!
//! 本 crate 保持进程无关、无状态、不依赖 Qt；
//! GUI（以及将来可能的守护进程）都通过它访问云端。

pub mod config;
pub mod error;
pub mod types;

pub use config::Config;
pub use error::Error;
