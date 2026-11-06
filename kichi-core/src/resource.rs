//! 磁链资源列表与秒传。
//!
//! `POST /drive/v1/resource/list` 让服务端解析磁力链接并返回文件清单；
//! 带 GCID 的文件可以用 `POST /drive/v1/files` 直接秒传，不必先跑完整的
//! 云下载任务。端点与请求形状参照公开 Web 客户端的实际请求（只看行为，
//! 未复制代码）。

use serde_json::{json, Value};

use crate::http::Body;
use crate::naming::MAX_NAME_BYTES;
use crate::{Config, DriveClient, Error};

/// 单个磁链最多接受的文件数；再多既不好选也不好逐一秒传。
pub const MAX_MAGNET_FILES: usize = 10_000;

/// 目录嵌套上限，防止异常响应把递归打穿。
const MAX_MAGNET_DEPTH: usize = 64;

/// 磁链里解析出的单个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagnetFile {
    pub name: String,
    /// 相对保存容器根目录的文件夹链；空 = 直接放在目标目录。
    pub folders: Vec<String>,
    pub size: u64,
    /// 内容哈希；官方只在已缓存该内容时给出，缺失则无法秒传。
    pub gcid: Option<String>,
}

impl MagnetFile {
    pub fn available(&self) -> bool {
        self.gcid.is_some()
    }
}

/// 一次磁链解析的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagnetListing {
    /// 单根目录时取目录名，否则取磁链 `dn=`，都没有时用「磁力任务」。
    pub root_name: String,
    pub files: Vec<MagnetFile>,
    pub total_size: u64,
}

impl MagnetListing {
    /// 已缓存内容、可以秒传的文件数。
    pub fn available_count(&self) -> usize {
        self.files.iter().filter(|file| file.available()).count()
    }
}

/// 秒传结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantSave {
    /// 云端已有同样内容，条目已经直接建好。
    Saved,
    /// 内容不在云端缓存里，秒传不可用；只能改走普通云下载。
    Unavailable,
}

impl DriveClient {
    /// 解析磁力链接里的文件清单。
    ///
    /// `link` 必须是磁力链接或纯 BTIH；HTTP / ed2k 仍走普通离线任务，不在这里解析。
    pub fn list_magnet_resources(&self, link: &str) -> Result<MagnetListing, Error> {
        let link = crate::normalize_task_url(link);
        if link.is_empty() {
            return Err(Error::InvalidInput("磁力链接为空".into()));
        }
        if !crate::is_supported_task_url(&link)
            || !link.to_ascii_lowercase().starts_with("magnet:?")
        {
            return Err(Error::InvalidInput("不是可解析的磁力链接".into()));
        }
        let body = json!({ "page_size": 2000, "urls": link });
        let payload: Value = self.request(
            reqwest::Method::POST,
            &self.config().resource_list_url(),
            &[],
            Body::Json(&body),
        )?;
        parse_magnet_listing(&payload, &link)
    }

    /// 把一个已解析的文件秒传到 `parent_id` 下（空串 = 根目录）。
    ///
    /// 响应里若没有直接建好文件，官方可能已经插入了一个等待上传的空壳；
    /// 这里尽力把它移入回收站，避免文件列表里留下永远不动的条目。
    pub fn instant_save_file(
        &self,
        parent_id: &str,
        file: &MagnetFile,
    ) -> Result<InstantSave, Error> {
        let Some(gcid) = file
            .gcid
            .as_deref()
            .map(str::trim)
            .filter(|hash| !hash.is_empty())
        else {
            return Ok(InstantSave::Unavailable);
        };
        let name = sanitize_drive_name(&file.name, "未命名文件");
        let body = instant_save_body(parent_id, &name, file.size, gcid);
        let payload: Value = self.request(
            reqwest::Method::POST,
            &self.config().files_url(),
            &[],
            Body::Json(&body),
        )?;
        if instant_saved(&payload) {
            return Ok(InstantSave::Saved);
        }
        if let Some(id) = payload
            .get("file")
            .and_then(|file| file.get("id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            // 清理失败不影响「不可秒传」这个结论，也不该盖住真正的失败原因。
            let trash = json!({ "ids": [id] });
            let _ = self.request_no_response_body(
                reqwest::Method::POST,
                &Config::file_action_url(self.config(), "batchTrash"),
                &[],
                &trash,
            );
        }
        Ok(InstantSave::Unavailable)
    }
}

/// 秒传请求体。hash 必须大写，size 是字符串。
fn instant_save_body(parent_id: &str, name: &str, size: u64, gcid: &str) -> Value {
    json!({
        "hash": gcid.trim().to_ascii_uppercase(),
        "name": name,
        "size": size.to_string(),
        "kind": "drive#file",
        "parent_id": parent_id.trim(),
        "upload_type": "UPLOAD_TYPE_RESUMABLE",
        "folder_type": "NORMAL",
        "objProvider": { "provider": "UPLOAD_TYPE_UNKNOWN" }
    })
}

/// 响应是否表示「内容已在云端，文件直接建好」。
///
/// 官方若给回 `resumable` 票据，说明它准备让你上传，内容其实没缓存；
/// 票据阶段是 PENDING / RUNNING 之类时同样不能当作成功。
fn instant_saved(payload: &Value) -> bool {
    let file = payload.get("file").filter(|value| value.is_object());
    let upload_type = payload
        .get("upload_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_uppercase();
    let phase = file
        .and_then(|file| file.get("phase"))
        .or_else(|| payload.get("phase"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_uppercase();
    let has_resumable = payload
        .get("resumable")
        .is_some_and(|value| value.is_object());
    let pending = ["PENDING", "RUNNING", "PAUSED", "QUEUED", "PROCESSING"]
        .iter()
        .any(|word| phase.contains(word));
    let failed = ["FAILED", "ERROR", "CANCELLED", "CANCELED", "ABORTED"]
        .iter()
        .any(|word| phase.contains(word));
    let has_file_id = file
        .and_then(|file| file.get("id"))
        .and_then(Value::as_str)
        .is_some_and(|id| !id.trim().is_empty());

    !has_resumable
        && !pending
        && !failed
        && (upload_type == "UPLOAD_TYPE_UNKNOWN"
            || phase == "PHASE_TYPE_COMPLETE"
            || (file.is_some() && has_file_id))
}

/// 从 `resource/list` 响应里整理出可保存的文件清单。
fn parse_magnet_listing(payload: &Value, link: &str) -> Result<MagnetListing, Error> {
    let list = payload
        .get("list")
        .filter(|value| value.is_object())
        .unwrap_or(payload);
    let resources = list
        .get("resources")
        .and_then(Value::as_array)
        .or_else(|| payload.get("resources").and_then(Value::as_array))
        .ok_or_else(|| Error::ApiChanged("磁链解析响应缺少 resources".into()))?;

    let mut root_name = magnet_display_name(link).unwrap_or_else(|| "磁力任务".to_owned());
    let mut files = Vec::new();

    if resources.len() == 1 && is_directory(&resources[0]) {
        // 单根目录直接展开：根目录名当保存容器的名字，子项从容器根算起。
        let root = &resources[0];
        if let Some(name) = resource_name(root) {
            root_name = name;
        }
        for child in children_of(root) {
            walk_resource(child, &[], 1, &mut files)?;
        }
    } else {
        if resources.len() == 1 {
            if let Some(name) = resource_name(&resources[0]) {
                root_name = name;
            }
        }
        for resource in resources {
            if is_directory(resource) {
                let name = resource_name(resource).unwrap_or_else(|| "未命名文件夹".to_owned());
                for child in children_of(resource) {
                    walk_resource(child, std::slice::from_ref(&name), 1, &mut files)?;
                }
            } else {
                walk_resource(resource, &[], 1, &mut files)?;
            }
        }
    }

    if files.is_empty() {
        return Err(Error::Other(
            "磁链里没有可保存的文件（官方可能还在解析，稍后再试）".into(),
        ));
    }
    let total_size = files.iter().map(|file| file.size).sum();
    Ok(MagnetListing {
        root_name,
        files,
        total_size,
    })
}

fn walk_resource(
    resource: &Value,
    folders: &[String],
    depth: usize,
    out: &mut Vec<MagnetFile>,
) -> Result<(), Error> {
    if depth > MAX_MAGNET_DEPTH {
        return Err(Error::ApiChanged("磁链目录嵌套过深".into()));
    }
    if out.len() >= MAX_MAGNET_FILES {
        return Err(Error::Other(format!(
            "磁链里的文件超过 {MAX_MAGNET_FILES} 个，无法逐个选择"
        )));
    }
    if is_directory(resource) {
        let name = resource_name(resource).unwrap_or_else(|| "未命名文件夹".to_owned());
        let mut chain = folders.to_vec();
        chain.push(name);
        for child in children_of(resource) {
            walk_resource(child, &chain, depth + 1, out)?;
        }
    } else {
        out.push(MagnetFile {
            name: resource_name(resource).unwrap_or_else(|| "未命名文件".to_owned()),
            folders: folders.to_vec(),
            size: resource_size(resource),
            gcid: resource_gcid(resource),
        });
    }
    Ok(())
}

fn is_directory(resource: &Value) -> bool {
    let is_dir = match resource.get("is_dir") {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => value.eq_ignore_ascii_case("true"),
        _ => false,
    };
    is_dir
        || resource
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind == "drive#folder")
        || resource
            .get("dir")
            .and_then(|dir| dir.get("resources"))
            .is_some_and(Value::is_array)
        || resource.get("resources").is_some_and(Value::is_array)
        || resource.get("children").is_some_and(Value::is_array)
}

fn children_of(resource: &Value) -> &[Value] {
    resource
        .get("dir")
        .and_then(|dir| dir.get("resources"))
        .and_then(Value::as_array)
        .or_else(|| resource.get("resources").and_then(Value::as_array))
        .or_else(|| resource.get("children").and_then(Value::as_array))
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn resource_name(resource: &Value) -> Option<String> {
    ["name", "file_name", "title"]
        .iter()
        .find_map(|key| resource.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn resource_size(resource: &Value) -> u64 {
    let size = |value: Option<&Value>| match value {
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0),
        Some(Value::String(text)) => text.trim().parse::<u64>().unwrap_or(0),
        _ => 0,
    };
    size(resource.get("file_size")).max(size(resource.get("size")))
}

fn resource_gcid(resource: &Value) -> Option<String> {
    resource
        .get("meta")
        .and_then(|meta| meta.get("hash"))
        .and_then(Value::as_str)
        .or_else(|| resource.get("gcid").and_then(Value::as_str))
        .or_else(|| resource.get("hash").and_then(Value::as_str))
        .map(str::trim)
        .filter(|hash| !hash.is_empty())
        .map(str::to_owned)
}

/// 从磁链的 `dn=` 参数取显示名。
fn magnet_display_name(link: &str) -> Option<String> {
    let query = link.split_once('?')?.1;
    let query = query.split('#').next().unwrap_or(query);
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key == "dn" {
            let name = percent_decode(value).trim().to_owned();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// 最小百分号解码（`%XX` 与 `+`；非法序列原样保留）。
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 磁链里的文件名 / 目录名转成云端可接受的名字。
///
/// 去掉路径分隔符、Windows 保留字符与控制字符；首尾空白和结尾的点也去掉。
/// 超过官方 255 字节上限时按字符边界截断。
pub fn sanitize_drive_name(raw: &str, fallback: &str) -> String {
    fn clean(value: &str) -> String {
        value
            .chars()
            .filter(|ch| {
                !ch.is_control()
                    && !matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            })
            .collect::<String>()
            .trim()
            .trim_end_matches('.')
            .trim()
            .to_owned()
    }
    let mut name = clean(raw);
    if name.is_empty() {
        name = clean(fallback);
    }
    if name.is_empty() {
        name = "未命名".to_owned();
    }
    if name.len() > MAX_NAME_BYTES {
        let mut boundary = MAX_NAME_BYTES;
        while boundary > 0 && !name.is_char_boundary(boundary) {
            boundary -= 1;
        }
        name = name[..boundary].trim_end().to_owned();
        if name.is_empty() {
            name = "未命名".to_owned();
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAGNET: &str = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=%E7%A4%BA%E4%BE%8B%E7%94%B5%E5%BD%B1";

    fn listing(payload: Value) -> MagnetListing {
        parse_magnet_listing(&payload, MAGNET).expect("响应应可解析")
    }

    #[test]
    fn parses_single_root_folder_and_unwraps_it() {
        let payload = json!({
            "list": {
                "resources": [{
                    "name": "剧集",
                    "is_dir": true,
                    "dir": { "resources": [
                        { "name": "第 1 集.mkv", "file_size": "1024", "meta": { "hash": "abc" } },
                        { "name": "字幕", "is_dir": true, "dir": { "resources": [
                            { "name": "第 1 集.ass", "size": 12 }
                        ]}}
                    ]}
                }]
            }
        });
        let parsed = listing(payload);
        assert_eq!(parsed.root_name, "剧集");
        assert_eq!(parsed.files.len(), 2);
        assert_eq!(parsed.files[0].folders, Vec::<String>::new());
        assert_eq!(parsed.files[0].size, 1024);
        assert_eq!(parsed.files[0].gcid.as_deref(), Some("abc"));
        assert_eq!(parsed.files[1].folders, vec!["字幕".to_owned()]);
        assert!(parsed.files[1].gcid.is_none());
        assert_eq!(parsed.total_size, 1036);
        assert_eq!(parsed.available_count(), 1);
    }

    #[test]
    fn parses_multiple_roots_and_nested_folders() {
        let payload = json!({
            "resources": [
                { "name": "电影.mkv", "size": 10, "gcid": "HASH" },
                { "name": "花絮", "kind": "drive#folder", "resources": [
                    { "name": "片段.mp4", "size": 5 }
                ]}
            ]
        });
        let parsed = listing(payload);
        assert_eq!(parsed.root_name, "示例电影", "取磁链 dn=");
        assert_eq!(parsed.files.len(), 2);
        assert_eq!(parsed.files[0].folders, Vec::<String>::new());
        assert_eq!(parsed.files[1].folders, vec!["花絮".to_owned()]);
    }

    #[test]
    fn treats_children_array_and_string_flags_as_folders() {
        let payload = json!({
            "list": {
                "resources": [{
                    "name": "根",
                    "is_dir": "true",
                    "children": [
                        { "name": "子目录", "children": [
                            { "name": "文件.bin", "file_size": 3 }
                        ]}
                    ]
                }]
            }
        });
        let parsed = listing(payload);
        assert_eq!(parsed.files[0].folders, vec!["子目录".to_owned()]);
        assert_eq!(parsed.files[0].size, 3);
    }

    #[test]
    fn empty_listing_is_a_clear_error() {
        let error = parse_magnet_listing(&json!({ "list": { "resources": [] } }), MAGNET)
            .expect_err("空清单要报错");
        assert!(error.to_string().contains("没有可保存的文件"));
    }

    #[test]
    fn missing_resources_is_an_api_change() {
        assert!(matches!(
            parse_magnet_listing(&json!({ "list": {} }), MAGNET),
            Err(Error::ApiChanged(_))
        ));
    }

    #[test]
    fn nesting_limit_is_enforced() {
        let mut node = json!({ "name": "叶子", "size": 1 });
        for index in 0..(MAX_MAGNET_DEPTH + 2) {
            node = json!({
                "name": format!("目录{index}"),
                "is_dir": true,
                "resources": [node],
            });
        }
        let payload = json!({ "resources": [node] });
        assert!(parse_magnet_listing(&payload, MAGNET).is_err());
    }

    #[test]
    fn percent_decoding_handles_spaces_and_invalid_sequences() {
        assert_eq!(percent_decode("a+b%20c"), "a b c");
        assert_eq!(percent_decode("%E7%A4%BA"), "示");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn name_sanitizing_keeps_cloud_names_legal() {
        assert_eq!(sanitize_drive_name(" a/b:c*? ", "fallback"), "abc");
        assert_eq!(sanitize_drive_name("   ", "备用"), "备用");
        assert_eq!(sanitize_drive_name("", ""), "未命名");
        assert_eq!(sanitize_drive_name("名字...", "x"), "名字");
        let long = "字".repeat(100);
        let trimmed = sanitize_drive_name(&long, "x");
        assert!(trimmed.len() <= MAX_NAME_BYTES);
        assert!(trimmed.chars().all(|ch| ch == '字'));
    }

    #[test]
    fn instant_save_body_matches_the_official_shape() {
        let body = instant_save_body(" parent ", "电影.mkv", 1024, "abc");
        assert_eq!(body["hash"], "ABC");
        assert_eq!(body["name"], "电影.mkv");
        assert_eq!(body["size"], "1024");
        assert_eq!(body["kind"], "drive#file");
        assert_eq!(body["parent_id"], "parent");
        assert_eq!(body["upload_type"], "UPLOAD_TYPE_RESUMABLE");
        assert_eq!(body["folder_type"], "NORMAL");
        assert_eq!(body["objProvider"]["provider"], "UPLOAD_TYPE_UNKNOWN");
    }

    #[test]
    fn instant_success_detection_matches_observed_responses() {
        assert!(instant_saved(&json!({
            "upload_type": "UPLOAD_TYPE_UNKNOWN",
            "file": { "id": "f1", "phase": "PHASE_TYPE_COMPLETE" }
        })));
        assert!(instant_saved(&json!({
            "file": { "id": "f1", "phase": "PHASE_TYPE_COMPLETE" }
        })));
        assert!(!instant_saved(&json!({
            "upload_type": "UPLOAD_TYPE_RESUMABLE",
            "resumable": { "provider": "PROVIDER_ALIYUN" },
            "file": { "id": "ghost", "phase": "PHASE_TYPE_PENDING" }
        })));
        assert!(!instant_saved(&json!({
            "file": { "id": "ghost", "phase": "PHASE_TYPE_ERROR" }
        })));
    }
}
