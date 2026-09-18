//! PikPak 非官方 API 的 Rust 客户端核心库。
//! 端点与加签算法参考社区逆向成果(Quan666/PikPakAPI 等)。

pub mod captcha;
pub mod client;
pub mod consts;
pub mod download;
pub mod error;
pub mod session;
pub mod types;
pub mod upload;

pub use client::KichiClient;
pub use error::Error;
pub use session::Session;
