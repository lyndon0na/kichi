//! kichi-core：PikPak 非官方 API 的纯 Rust 封装。
//!
//! 本 crate 保持进程无关、无状态、不依赖 Qt；
//! GUI（以及将来可能的守护进程）都通过它访问云端。

pub mod auth;
pub mod captcha;
pub mod config;
pub mod drive;
pub mod error;
pub(crate) mod http;
pub mod media;
pub mod tasks;
pub mod types;

pub use auth::{LoginOutcome, REFRESH_MARGIN};
pub use config::Config;
pub use drive::{DriveClient, UserInfo};
pub use error::Error;
pub use media::FileInfo;
pub use types::{EntryKind, FileEntry, OfflineTask, PlayOption, Quota, TaskPhase, TokenPair};
