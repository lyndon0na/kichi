//! `queue_collect.rs` 的单元测试：用假目录树验证边界规则，不碰网络。
//!
//! 覆盖空目录、只有空子目录、过深、读失败（可跳过 / 必须中止）、防环、
//! 取消，以及「所有子目录都读不到」与空文件夹的分界。

use std::collections::HashMap;

use super::*;

fn folder(id: &str, name: &str) -> FileEntry {
    entry(id, name, EntryKind::Folder)
}

fn file(id: &str, name: &str) -> FileEntry {
    entry(id, name, EntryKind::File)
}

fn entry(id: &str, name: &str, kind: EntryKind) -> FileEntry {
    FileEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        kind,
        size: 0,
        created_at: None,
        modified_at: None,
        parent_id: String::new(),
        md5: None,
        audit_flagged: false,
        starred: false,
        media: false,
        video: false,
        thumbnail_url: String::new(),
    }
}

/// 假目录树：id → 这一层的条目（或读取失败）。
///
/// 失败按类别存，调用时才造 `Failure`（它不实现 Clone）。
#[derive(Default)]
struct Tree {
    nodes: HashMap<String, Node>,
}

enum Node {
    Entries(Vec<FileEntry>),
    Failure(FailureKind),
}

/// `Failure` 不可 Clone，存一份可重复生成的类别。
enum FailureKind {
    Other(String),
    Auth(String),
    RateLimited(String),
    Captcha,
}

impl FailureKind {
    fn build(&self) -> Failure {
        match self {
            Self::Other(reason) => Failure::Other(reason.clone()),
            Self::Auth(reason) => Failure::Auth(reason.clone()),
            Self::RateLimited(detail) => Failure::RateLimited {
                detail: detail.clone(),
            },
            Self::Captcha => Failure::Captcha { url: None },
        }
    }
}

impl Tree {
    fn with(mut self, id: &str, entries: Vec<FileEntry>) -> Self {
        self.nodes.insert(id.to_owned(), Node::Entries(entries));
        self
    }

    fn failing(mut self, id: &str, failure: Failure) -> Self {
        self.nodes
            .insert(id.to_owned(), Node::Failure(FailureKind::from(failure)));
        self
    }

    fn get(&self, id: &str) -> Result<Vec<FileEntry>, Failure> {
        match self.nodes.get(id) {
            Some(Node::Entries(entries)) => Ok(entries.clone()),
            Some(Node::Failure(kind)) => Err(kind.build()),
            // 假树漏配的目录按「不存在」处理，避免误判为空目录
            None => Err(Failure::Other(format!("没有这个目录：{id}"))),
        }
    }

    fn collect(
        &mut self,
        root: &FileEntry,
        max_depth: usize,
        cancel: &Cancel,
    ) -> Result<Collected, Failure> {
        let mut list = |id: &str| self.get(id);
        collect_tree(&mut list, root, max_depth, cancel)
    }
}

impl From<Failure> for FailureKind {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Other(reason) => Self::Other(reason),
            Failure::Auth(reason) => Self::Auth(reason),
            Failure::RateLimited { detail } => Self::RateLimited(detail),
            Failure::Captcha { .. } => Self::Captcha,
            // 这个测试替身只关心失败归类；分享类错误按普通失败处理。
            Failure::ShareUnavailable { message, .. } => Self::Other(message),
        }
    }
}

fn failure_text(result: Result<Collected, Failure>) -> String {
    match result {
        Ok(_) => panic!("这里应该失败"),
        Err(err) => err.to_string(),
    }
}

#[test]
fn empty_root_is_an_empty_folder_not_a_failure() {
    let root = folder("root", "空文件夹");
    let mut tree = Tree::default().with("root", Vec::new());
    let collected = tree
        .collect(&root, DEFAULT_MAX_DEPTH, &Cancel::default())
        .expect("空文件夹要算成功");

    assert!(collected.files.is_empty());
    assert_eq!(collected.dirs, vec![vec!["空文件夹".to_owned()]]);
    assert_eq!(collected.skipped, 0);
}

#[test]
fn nested_empty_folders_are_all_kept() {
    let root = folder("root", "剧集");
    let mut tree = Tree::default()
        .with("root", vec![folder("s1", "第一季"), folder("s2", "空")])
        .with("s1", vec![folder("s1a", "花絮")])
        .with("s1a", Vec::new())
        .with("s2", Vec::new());
    let collected = tree
        .collect(&root, DEFAULT_MAX_DEPTH, &Cancel::default())
        .expect("只有空子目录也是成功");

    assert!(collected.files.is_empty(), "一个文件都没有");
    assert_eq!(
        collected.dirs,
        vec![
            vec!["剧集".to_owned()],
            vec!["剧集".to_owned(), "第一季".to_owned()],
            vec!["剧集".to_owned(), "第一季".to_owned(), "花絮".to_owned()],
            vec!["剧集".to_owned(), "空".to_owned()],
        ],
        "所有成功列出的目录（含嵌套空目录）都保留，按先序"
    );
    assert_eq!(collected.skipped, 0);
}

#[test]
fn files_and_empty_folders_coexist() {
    let root = folder("root", "剧集");
    let mut tree = Tree::default()
        .with(
            "root",
            vec![file("f1", "第 1 集.mkv"), folder("s1", "空子目录")],
        )
        .with("s1", Vec::new());
    let collected = tree
        .collect(&root, DEFAULT_MAX_DEPTH, &Cancel::default())
        .expect("有文件也有空目录");

    assert_eq!(collected.files.len(), 1);
    assert_eq!(collected.files[0].sub_dir, vec!["剧集".to_owned()]);
    assert_eq!(collected.files[0].name, "第 1 集.mkv");
    assert_eq!(
        collected.dirs,
        vec![
            vec!["剧集".to_owned()],
            vec!["剧集".to_owned(), "空子目录".to_owned()],
        ]
    );
    assert_eq!(collected.skipped, 0);
}

#[test]
fn too_deep_branches_are_skipped_without_a_request() {
    let root = folder("root", "根");
    // max_depth=1：根 = 0、子目录 = 1、孙目录 = 2 → 跳过
    let mut tree = Tree::default()
        .with("root", vec![file("f0", "根文件.mkv"), folder("a", "一层")])
        .with("a", vec![file("f1", "浅层文件.mkv"), folder("b", "二层")])
        // 给 b 配上内容：一旦它被请求就会出现在 dirs 里，断言当场失败
        .with("b", vec![file("f2", "深层文件.mkv")]);
    let collected = tree
        .collect(&root, 1, &Cancel::default())
        .expect("过深只跳过分支，不算整批失败");

    assert_eq!(collected.skipped, 1, "被跳过的是二层目录");
    let names: Vec<&str> = collected
        .files
        .iter()
        .map(|job| job.name.as_str())
        .collect();
    assert_eq!(names, vec!["根文件.mkv", "浅层文件.mkv"], "浅层文件照下");
    assert!(
        !collected
            .dirs
            .contains(&vec!["根".to_owned(), "一层".to_owned(), "二层".to_owned()]),
        "被跳过的那一层不建目录"
    );
    assert!(
        collected
            .dirs
            .contains(&vec!["根".to_owned(), "一层".to_owned()]),
        "父级已知目录仍然建"
    );
}

#[test]
fn max_depth_parsing_falls_back_for_zero_and_garbage() {
    assert_eq!(parse_max_depth(None), DEFAULT_MAX_DEPTH);
    assert_eq!(parse_max_depth(Some("")), DEFAULT_MAX_DEPTH);
    assert_eq!(parse_max_depth(Some("  ")), DEFAULT_MAX_DEPTH);
    assert_eq!(parse_max_depth(Some("abc")), DEFAULT_MAX_DEPTH);
    assert_eq!(parse_max_depth(Some("-1")), DEFAULT_MAX_DEPTH);
    assert_eq!(parse_max_depth(Some("0")), DEFAULT_MAX_DEPTH, "0 回默认");
    assert_eq!(parse_max_depth(Some("8")), 8);
    assert_eq!(parse_max_depth(Some(" 8 ")), 8, "允许前后空白");
}

#[test]
fn unreadable_subfolder_is_skipped_and_siblings_survive() {
    let root = folder("root", "剧集");
    let mut tree = Tree::default()
        .with("root", vec![folder("gone", "已删除"), folder("ok", "还在")])
        .failing("gone", Failure::Other("item_not_found".to_owned()))
        .with("ok", vec![file("f1", "第 1 集.mkv")]);
    let collected = tree
        .collect(&root, DEFAULT_MAX_DEPTH, &Cancel::default())
        .expect("子目录读失败只跳过这个分支");

    assert_eq!(collected.skipped, 1);
    assert_eq!(collected.files.len(), 1);
    assert!(
        !collected
            .dirs
            .contains(&vec!["剧集".to_owned(), "已删除".to_owned()]),
        "读失败的目录不建"
    );
    assert!(collected
        .dirs
        .contains(&vec!["剧集".to_owned(), "还在".to_owned()]));
}

#[test]
fn auth_rate_limit_and_captcha_abort_the_whole_batch() {
    let failures = [
        Failure::Auth("令牌过期".to_owned()),
        Failure::RateLimited {
            detail: "too many requests".to_owned(),
        },
        Failure::Captcha { url: None },
    ];
    for failure in failures {
        let label = failure.to_string();
        let root = folder("root", "剧集");
        let mut tree = Tree::default()
            .with("root", vec![folder("a", "子目录")])
            .failing("a", failure);
        let text = failure_text(tree.collect(&root, DEFAULT_MAX_DEPTH, &Cancel::default()));
        assert!(
            !text.contains("所有子目录都无法读取"),
            "这类错误不能被当作跳过：{label} → {text}"
        );
    }
}

#[test]
fn cancel_stops_the_walk_without_counting_a_skip() {
    let root = folder("root", "剧集");
    let cancel = Cancel::default();
    cancel.cancel();
    let mut tree = Tree::default().with("root", vec![folder("a", "子目录")]);
    let text = failure_text(tree.collect(&root, DEFAULT_MAX_DEPTH, &cancel));
    assert_eq!(text, CANCELLED);
}

#[test]
fn a_cancel_midway_is_not_mistaken_for_a_skipped_branch() {
    let root = folder("root", "剧集");
    let cancel = Cancel::default();
    let tree = Tree::default()
        .with("root", vec![folder("a", "子目录")])
        .failing("a", Failure::Other("连接中断".to_owned()));
    // 读子目录时用户按了取消，随后错误才返回：按取消收尾，不记跳过
    let mut list = |id: &str| {
        if id == "a" {
            cancel.cancel();
        }
        tree.get(id)
    };
    let text = failure_text(collect_tree(&mut list, &root, DEFAULT_MAX_DEPTH, &cancel));
    assert_eq!(text, CANCELLED, "取消按取消收尾，不记跳过");
}

#[test]
fn cycles_do_not_loop_forever_and_count_one_skip() {
    let root = folder("root", "A");
    let mut tree = Tree::default()
        .with("root", vec![folder("b", "B")])
        // B 里又指回 A：visited 里已有，直接跳过
        .with("b", vec![folder("root", "A"), file("f1", "B 里的文件")]);
    let collected = tree
        .collect(&root, DEFAULT_MAX_DEPTH, &Cancel::default())
        .expect("环只跳过重复分支");

    assert_eq!(collected.skipped, 1);
    assert_eq!(collected.files.len(), 1);
    assert_eq!(
        collected.dirs,
        vec![vec!["A".to_owned()], vec!["A".to_owned(), "B".to_owned()]]
    );
}

#[test]
fn root_read_failure_fails_the_whole_batch() {
    let root = folder("root", "剧集");
    let mut tree = Tree::default().failing("root", Failure::Other("条目不在了".to_owned()));
    let text = failure_text(tree.collect(&root, DEFAULT_MAX_DEPTH, &Cancel::default()));
    assert_eq!(text, "条目不在了");
}

#[test]
fn all_subfolders_failing_is_not_an_empty_folder() {
    let root = folder("root", "剧集");
    let mut tree = Tree::default()
        .with("root", vec![folder("a", "甲"), folder("b", "乙")])
        .failing("a", Failure::Other("删掉了".to_owned()))
        .failing("b", Failure::Other("删掉了".to_owned()));
    let text = failure_text(tree.collect(&root, DEFAULT_MAX_DEPTH, &Cancel::default()));
    assert!(
        text.contains("所有子目录都无法读取") && text.contains("跳过 2 个"),
        "要明确说清不是空文件夹：{text}"
    );
}

#[test]
fn one_readable_empty_subfolder_is_enough_to_succeed() {
    let root = folder("root", "剧集");
    let mut tree = Tree::default()
        .with("root", vec![folder("a", "甲"), folder("b", "乙")])
        .with("a", Vec::new())
        .failing("b", Failure::Other("删掉了".to_owned()));
    let collected = tree
        .collect(&root, DEFAULT_MAX_DEPTH, &Cancel::default())
        .expect("至少有一个子目录读到了，就算成功");

    assert_eq!(collected.skipped, 1);
    assert_eq!(collected.dirs.len(), 2, "根 + 读到的那个空目录");
}

#[test]
fn only_skipped_branches_with_no_readable_child_is_a_failure() {
    // 根下只有一个目录，它过深 / 读失败：dirs 只有根，files 为空 → 失败
    let root = folder("root", "剧集");
    let mut tree = Tree::default().with("root", vec![folder("a", "甲")]);
    tree.nodes.insert(
        "a".to_owned(),
        Node::Failure(FailureKind::Other("删掉了".to_owned())),
    );
    let text = failure_text(tree.collect(&root, DEFAULT_MAX_DEPTH, &Cancel::default()));
    assert!(text.contains("所有子目录都无法读取"), "{text}");
}
