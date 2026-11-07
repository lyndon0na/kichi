//! 传输任务记录表：下载流水（跨重启保留）。
//!
//! 只有界面侧需要这些记录（排队顺序、失败原因、落盘路径），因此不进
//! `kichi-core`；落盘与读回规则拆在 [`super::transfers_history`]。
//!
//! 排队批次只登记一条「总账行」（文件夹名），轮到读目录时才用
//! [`expand_folder`] 在它后面挂上子行 —— 统一列表始终一个文件夹批次一行，
//! 几百个文件同时排队时不会把上限挤爆；子行只在父行展开时插入同一张表。
//!
//! 并发：下载线程直接写记录，主线程（取消 / 清空 / 登出）也写，统一走
//! [`with_records`] 的互斥锁；锁中毒时继续用内部数据，不 panic。
//! QML 模型由主线程通过 `sync_transfers()` 重建，下载线程只改记录 + 投递同步任务。
//!
//! **锁顺序**：先 `records`，再 `trees`（文件夹目录树）。任何地方都按这个顺序加锁。
//!
//! 按职责拆分：记录类型在 [`model`]，批次聚合在 [`batch`]，全局表与淘汰在
//! [`store`]，排队 / 展开 / 开始 / 暂停在 [`queue`] / [`pause`]，上传文件夹
//! 批次在 [`upload`]，进度与视图字段在 [`progress`]，取消与断点清理在 [`cancel`]。

mod batch;
mod cancel;
mod model;
mod pause;
mod progress;
mod queue;
mod store;
mod upload;

pub(crate) use batch::*;
pub(crate) use cancel::*;
pub(crate) use model::*;
pub(crate) use pause::*;
pub(crate) use progress::*;
pub(crate) use queue::*;
pub(crate) use store::*;
pub(crate) use upload::*;

#[cfg(test)]
#[path = "transfers_tests.rs"]
mod tests;
