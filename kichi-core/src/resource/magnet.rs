//! `resource/list` 响应 → 磁链文件清单的纯解析。

use serde_json::Value;

use crate::Error;

use super::{MagnetFile, MagnetListing, MAX_MAGNET_DEPTH, MAX_MAGNET_FILES};

pub(super) fn parse_magnet_listing(payload: &Value, link: &str) -> Result<MagnetListing, Error> {
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
pub(super) fn percent_decode(text: &str) -> String {
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
