//! kichi-core：PikPak 非官方 API 的纯 Rust 封装。
//!
//! 本 crate 保持进程无关、无状态、不依赖 Qt；
//! GUI（以及将来可能的守护进程）都通过它访问云端。

pub mod auth;
pub mod captcha;
pub mod config;
pub mod dl;
pub mod drive;
pub mod error;
pub mod events;
pub(crate) mod http;
pub mod library;
pub mod manage;
pub mod media;
mod naming;
pub mod search;
pub mod share;
pub mod subtitles;
pub mod tasks;
pub mod types;
pub mod upload;
mod upload_rate;
mod upload_state;

pub use auth::{LoginOutcome, REFRESH_MARGIN};
pub use config::Config;
pub use dl::DownloadStream;
pub use drive::{DriveClient, UserInfo};
pub use error::Error;
pub use events::{PlaybackHistoryEntry, PlaybackProgress, RecentSavedEvent};
pub use library::{LibraryEntry, LibraryKind};
pub use media::{FileInfo, NoLinkReason};
pub use search::FileHit;
pub use share::{CreateShareOptions, CreatedShare, MyShare};
pub use tasks::TaskStatus;
pub use types::{
    EntryKind, FileEntry, OfflineTask, PlayOption, Quota, TaskPhase, TokenPair, TrashEntry,
};
pub use upload::{UploadProgress, UploadResult, UploadStage};
pub use upload_rate::UploadRateLimiter;
pub use upload_state::{
    UploadCredentials, UploadError, UploadPart, UploadResume, UPLOAD_RESUME_VERSION,
};
