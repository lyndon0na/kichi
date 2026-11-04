//! 从文件夹 id 反查它在网盘里的完整路径。
//!
//! 离线任务响应只有产物 id，不带面包屑；`parent_of` 能拿到自身名字与父目录，
//! 再逐级向上即可还原包含目标文件夹自身的完整路径。每条路径最多回溯
//! [`MAX_DEPTH`] 层。

use super::{Failure, Session};

const MAX_DEPTH: usize = 64;

impl Session {
    /// 返回条目自身的名字与父目录 id；文件和文件夹都可以查询。
    pub fn parent_of(&self, entry_id: &str) -> Result<(String, String), Failure> {
        let entry_id = entry_id.trim();
        if entry_id.is_empty() {
            return Err(Failure::Other("产物 id 为空".to_owned()));
        }
        if let Some(index) = self.tree_index() {
            match index.parent_location(entry_id) {
                Ok(Some(location)) => return Ok(location),
                Ok(None) => {}
                Err(err) => self.record_tree_index_error(format!("本地索引读取失败：{err}")),
            }
        }
        let client = self.drive()?;
        client.parent_of(entry_id).map_err(Failure::from)
    }

    /// 返回从根目录下一级到目标文件夹自身的 `(id, 名称)` 列表。
    ///
    /// `folder_id` 必须指向文件夹；返回列表的最后一项就是该文件夹。
    pub fn folder_trail(&self, folder_id: &str) -> Result<Vec<(String, String)>, Failure> {
        let folder_id = folder_id.trim();
        if folder_id.is_empty() {
            return Err(Failure::Other("产物文件夹 id 为空".to_owned()));
        }
        // 本地索引已有完整父链就不必逐级请求；索引缺链时再走网络。
        if let Some(index) = self.tree_index() {
            match index.path_to(folder_id) {
                Ok(Some(trail)) if !trail.is_empty() => return Ok(trail),
                Ok(_) => {}
                Err(err) => {
                    self.record_tree_index_error(format!("本地索引读取失败：{err}"));
                }
            }
        }
        let client = self.drive()?;
        let trail = resolve_trail(folder_id.to_owned(), |id| {
            client.parent_of(id).map_err(Failure::from)
        })?;
        // 回填失败不影响本次路径结果；错误留给界面提示一次。
        if let Some(index) = self.tree_index() {
            if let Err(err) = index.backfill_trail(&trail) {
                self.record_tree_index_error(format!("本地索引回填失败：{err}"));
            }
        }
        Ok(trail)
    }
}

fn resolve_trail<F>(start_id: String, mut lookup: F) -> Result<Vec<(String, String)>, Failure>
where
    F: FnMut(&str) -> Result<(String, String), Failure>,
{
    let mut trail = Vec::new();
    let mut current = start_id;
    while !current.is_empty() {
        if trail.len() >= MAX_DEPTH {
            return Err(Failure::Other("目录层级异常，无法定位完整路径".to_owned()));
        }
        let (name, parent_id) = lookup(&current)?;
        trail.push((current, name));
        current = parent_id;
    }
    trail.reverse();
    Ok(trail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn folder_chain_includes_target_and_goes_from_root_to_leaf() {
        let folders = HashMap::from([
            ("season", ("第 1 季", "anime")),
            ("anime", ("动画", "bangumi")),
            ("bangumi", ("追番", "")),
        ]);
        let trail = resolve_trail("season".to_owned(), |id| {
            folders
                .get(id)
                .map(|(name, parent)| ((*name).to_owned(), (*parent).to_owned()))
                .ok_or_else(|| Failure::Other(format!("missing {id}")))
        })
        .expect("路径应可解析");

        assert_eq!(
            trail,
            vec![
                ("bangumi".to_owned(), "追番".to_owned()),
                ("anime".to_owned(), "动画".to_owned()),
                ("season".to_owned(), "第 1 季".to_owned()),
            ]
        );
    }

    #[test]
    fn root_parent_needs_no_lookup() {
        let trail = resolve_trail(String::new(), |_| {
            panic!("根目录不应请求父级");
        })
        .expect("根目录路径为空");
        assert!(trail.is_empty());
    }

    #[test]
    fn cycles_are_bounded() {
        let result = resolve_trail("a".to_owned(), |_| {
            Ok(("循环目录".to_owned(), "a".to_owned()))
        });
        assert!(result.is_err(), "循环层级必须报错而不是无限请求");
    }
}
