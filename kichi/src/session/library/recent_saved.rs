//! “最近保存”查询落点。公共骨架阶段先返回空列表。

use kichi_core::LibraryEntry;

use super::super::{Failure, Session};

pub(super) fn load(_session: &Session) -> Result<Vec<LibraryEntry>, Failure> {
    Ok(Vec::new())
}
