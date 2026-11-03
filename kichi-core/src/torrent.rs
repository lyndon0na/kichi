//! `.torrent` 文件的本地解析。
//!
//! PikPak 的离线任务接口不接收 torrent 文件本身；公开 PikPak Web 客户端的做法是
//! 在本地读取 `info` 字典、计算 BitTorrent v1 infohash，再提交
//! `magnet:?xt=urn:btih:...`。这里只实现这条离线提交链路需要的解析，不内置
//! tracker / DHT，也不把 `.torrent` 上传到云端。

use std::fs::File;
use std::io::Read;
use std::ops::Range;
use std::path::Path;

use sha1::{Digest, Sha1};

use crate::Error;

/// 单个 `.torrent` 文件的读取上限。正常种子只有几十 KiB；8 MiB 足以覆盖
/// 大量文件 / tracker 的极端种子，同时避免误选大文件时把内存打满。
pub const MAX_TORRENT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_BENCODE_DEPTH: usize = 64;

/// 从 `.torrent` 提取的离线任务信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentInfo {
    pub name: String,
    /// 40 位小写十六进制 BitTorrent v1 infohash。
    pub info_hash: String,
    pub total_size: u64,
    pub file_count: usize,
}

impl TorrentInfo {
    /// PikPak 创建离线任务时使用的磁力链接。
    pub fn magnet(&self) -> String {
        format!("magnet:?xt=urn:btih:{}", self.info_hash)
    }
}

/// 读取并解析一个本地 `.torrent` 文件。
///
/// 不跟随符号链接：和文件上传的拖放规则一致，避免把任意路径悄悄当成种子读取。
pub fn parse_torrent_file(path: &Path) -> Result<TorrentInfo, Error> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|err| Error::Other(format!("无法读取 .torrent 文件：{err}")))?;
    if metadata.file_type().is_symlink() {
        return Err(Error::Other("不跟随符号链接读取 .torrent".into()));
    }
    if !metadata.is_file() {
        return Err(Error::Other("只能解析普通 .torrent 文件".into()));
    }
    if metadata.len() == 0 {
        return Err(Error::Other("无法解析 .torrent：文件为空".into()));
    }
    if metadata.len() > MAX_TORRENT_BYTES {
        return Err(Error::Other(format!(
            "无法解析 .torrent：文件超过 {} MiB 上限",
            MAX_TORRENT_BYTES / (1024 * 1024)
        )));
    }

    let file =
        File::open(path).map_err(|err| Error::Other(format!("无法读取 .torrent 文件：{err}")))?;
    let mut bytes = Vec::new();
    file.take(MAX_TORRENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| Error::Other(format!("无法读取 .torrent 文件：{err}")))?;
    parse_torrent_bytes(&bytes)
}

/// 解析 `.torrent` 内容。只接受带 v1 `pieces` 的种子（v1 / 混合种子）。
pub fn parse_torrent_bytes(bytes: &[u8]) -> Result<TorrentInfo, Error> {
    if bytes.is_empty() {
        return Err(Error::Other("无法解析 .torrent：文件为空".into()));
    }
    if bytes.len() as u64 > MAX_TORRENT_BYTES {
        return Err(Error::Other(format!(
            "无法解析 .torrent：文件超过 {} MiB 上限",
            MAX_TORRENT_BYTES / (1024 * 1024)
        )));
    }

    // 有些下载来源会在文件末尾额外补换行或零字节；解析前忽略它们，但 info
    // 的内部字节保持原样，不能重新编码。
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace() && *byte != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    let data = &bytes[..end];
    if data.is_empty() {
        return Err(Error::Other("无法解析 .torrent：文件为空".into()));
    }

    let info_span = top_level_info_span(data)?;
    let fields = info_fields(&data[info_span.clone()])?;
    let info_hash = format!("{:x}", Sha1::digest(&data[info_span]));

    Ok(TorrentInfo {
        name: fields.name,
        info_hash,
        total_size: fields.total_size,
        file_count: fields.file_count,
    })
}

fn parse_error(reason: &str) -> Error {
    Error::Other(format!("无法解析 .torrent：{reason}"))
}

struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Integer,
    Bytes,
    List,
    Dictionary,
}

impl<'a> Parser<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn peek(&self) -> Result<u8, Error> {
        self.data
            .get(self.pos)
            .copied()
            .ok_or_else(|| parse_error("数据提前结束"))
    }

    fn expect(&mut self, expected: u8) -> Result<(), Error> {
        if self.peek()? != expected {
            return Err(parse_error("bencode 结构不完整"));
        }
        self.pos += 1;
        Ok(())
    }

    fn bytes(&mut self) -> Result<&'a [u8], Error> {
        let mut length = 0usize;
        let mut digits = 0usize;
        while let Some(byte) = self.data.get(self.pos).copied() {
            if !byte.is_ascii_digit() {
                break;
            }
            length = length
                .checked_mul(10)
                .and_then(|value| value.checked_add((byte - b'0') as usize))
                .ok_or_else(|| parse_error("字符串长度溢出"))?;
            digits += 1;
            self.pos += 1;
        }
        if digits == 0 || self.peek()? != b':' {
            return Err(parse_error("字符串长度无效"));
        }
        self.pos += 1;
        let end = self
            .pos
            .checked_add(length)
            .ok_or_else(|| parse_error("字符串长度溢出"))?;
        if end > self.data.len() {
            return Err(parse_error("字符串数据提前结束"));
        }
        let value = &self.data[self.pos..end];
        self.pos = end;
        Ok(value)
    }

    fn integer(&mut self) -> Result<i64, Error> {
        self.expect(b'i')?;
        let start = self.pos;
        if self.peek()? == b'-' {
            self.pos += 1;
        }
        let digits_start = self.pos;
        while self
            .data
            .get(self.pos)
            .copied()
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            self.pos += 1;
        }
        if self.pos == digits_start || self.peek()? != b'e' {
            return Err(parse_error("整数格式无效"));
        }
        let text = std::str::from_utf8(&self.data[start..self.pos])
            .map_err(|_| parse_error("整数格式无效"))?;
        let value = text
            .parse::<i64>()
            .map_err(|_| parse_error("整数超出范围"))?;
        self.pos += 1;
        Ok(value)
    }

    fn non_negative_integer(&mut self) -> Result<u64, Error> {
        let value = self.integer()?;
        u64::try_from(value).map_err(|_| parse_error("长度不能为负数"))
    }

    fn value(&mut self, depth: usize) -> Result<(Range<usize>, ValueKind), Error> {
        if depth > MAX_BENCODE_DEPTH {
            return Err(parse_error("bencode 嵌套过深"));
        }
        let start = self.pos;
        let kind = match self.peek()? {
            b'i' => {
                self.integer()?;
                ValueKind::Integer
            }
            b'0'..=b'9' => {
                self.bytes()?;
                ValueKind::Bytes
            }
            b'l' => {
                self.pos += 1;
                while self.peek()? != b'e' {
                    self.value(depth + 1)?;
                }
                self.pos += 1;
                ValueKind::List
            }
            b'd' => {
                self.pos += 1;
                while self.peek()? != b'e' {
                    self.bytes()?;
                    self.value(depth + 1)?;
                }
                self.pos += 1;
                ValueKind::Dictionary
            }
            _ => return Err(parse_error("未知的 bencode 值")),
        };
        Ok((start..self.pos, kind))
    }
}

fn top_level_info_span(data: &[u8]) -> Result<Range<usize>, Error> {
    let mut parser = Parser::new(data);
    parser.expect(b'd')?;
    let mut info = None;
    while parser.peek()? != b'e' {
        let key = parser.bytes()?;
        let (span, kind) = parser.value(1)?;
        if key == b"info" {
            if kind != ValueKind::Dictionary {
                return Err(parse_error("info 不是字典"));
            }
            if info.replace(span).is_some() {
                return Err(parse_error("info 字典重复"));
            }
        }
    }
    parser.expect(b'e')?;
    if parser.pos != data.len() {
        return Err(parse_error("文件末尾有多余数据"));
    }
    info.ok_or_else(|| parse_error("缺少 info 字典"))
}

struct InfoFields {
    name: String,
    total_size: u64,
    file_count: usize,
}

fn info_fields(info: &[u8]) -> Result<InfoFields, Error> {
    let mut parser = Parser::new(info);
    parser.expect(b'd')?;
    let mut name = None;
    let mut utf8_name = None;
    let mut single_length = None;
    let mut files = None;
    let mut pieces = false;
    let mut meta_version = None;

    while parser.peek()? != b'e' {
        let key = parser.bytes()?;
        match key {
            b"name" => name = Some(read_name(parser.bytes()?)?),
            b"name.utf-8" => utf8_name = Some(read_name(parser.bytes()?)?),
            b"length" => single_length = Some(parser.non_negative_integer()?),
            b"files" => files = Some(read_files_list(&mut parser)?),
            b"pieces" => {
                if parser.bytes()?.is_empty() {
                    return Err(parse_error("pieces 为空"));
                }
                pieces = true;
            }
            b"meta version" => meta_version = Some(parser.integer()?),
            _ => {
                parser.value(1)?;
            }
        }
    }
    parser.expect(b'e')?;
    if parser.pos != info.len() {
        return Err(parse_error("info 字典末尾有多余数据"));
    }

    if !pieces {
        return Err(if meta_version == Some(2) {
            parse_error("仅支持 BitTorrent v1 / 混合种子，暂不支持 v2-only")
        } else {
            parse_error("缺少 v1 pieces 字段")
        });
    }

    let name = utf8_name.or(name).unwrap_or_default();
    if name.is_empty() {
        return Err(parse_error("缺少种子名称"));
    }
    let (file_count, total_size) = match files {
        Some((count, total)) if count > 0 => (count, total),
        Some(_) => return Err(parse_error("文件列表为空")),
        None => (1, single_length.ok_or_else(|| parse_error("缺少文件长度"))?),
    };

    Ok(InfoFields {
        name,
        total_size,
        file_count,
    })
}

fn read_name(bytes: &[u8]) -> Result<String, Error> {
    let name = String::from_utf8_lossy(bytes).trim().to_owned();
    if name.is_empty() {
        Err(parse_error("种子名称为空"))
    } else {
        Ok(name)
    }
}

fn read_files_list(parser: &mut Parser<'_>) -> Result<(usize, u64), Error> {
    parser.expect(b'l')?;
    let mut count = 0usize;
    let mut total = 0u64;
    while parser.peek()? != b'e' {
        parser.expect(b'd')?;
        let mut length = None;
        while parser.peek()? != b'e' {
            let key = parser.bytes()?;
            if key == b"length" {
                length = Some(parser.non_negative_integer()?);
            } else {
                parser.value(1)?;
            }
        }
        parser.expect(b'e')?;
        let length = length.ok_or_else(|| parse_error("文件条目缺少 length"))?;
        count = count
            .checked_add(1)
            .ok_or_else(|| parse_error("文件数量溢出"))?;
        total = total
            .checked_add(length)
            .ok_or_else(|| parse_error("文件总大小溢出"))?;
    }
    parser.expect(b'e')?;
    Ok((count, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Vec<u8> {
        let mut out = format!("{}:", value.len()).into_bytes();
        out.extend_from_slice(value.as_bytes());
        out
    }

    fn integer(value: u64) -> Vec<u8> {
        format!("i{value}e").into_bytes()
    }

    fn dictionary(entries: &[(&[u8], Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![b'd'];
        for (key, value) in entries {
            out.extend_from_slice(&format!("{}:", key.len()).into_bytes());
            out.extend_from_slice(key);
            out.extend_from_slice(value);
        }
        out.push(b'e');
        out
    }

    fn files(entries: &[u64]) -> Vec<u8> {
        let mut out = vec![b'l'];
        for length in entries {
            out.extend(dictionary(&[(b"length", integer(*length))]));
        }
        out.push(b'e');
        out
    }

    fn single_file_info() -> Vec<u8> {
        dictionary(&[
            (b"length", integer(1024)),
            (b"name", text("示例电影")),
            (b"pieces", text("abc")),
        ])
    }

    fn wrap_torrent(info: &[u8]) -> Vec<u8> {
        let mut out = vec![b'd'];
        out.extend(text("info"));
        out.extend_from_slice(info);
        out.push(b'e');
        out
    }

    #[test]
    fn parses_single_file_and_uses_exact_info_bytes_for_hash() {
        let info = single_file_info();
        let torrent = wrap_torrent(&info);
        let parsed = parse_torrent_bytes(&torrent).expect("有效种子");
        assert_eq!(parsed.name, "示例电影");
        assert_eq!(parsed.total_size, 1024);
        assert_eq!(parsed.file_count, 1);
        assert_eq!(parsed.info_hash, format!("{:x}", Sha1::digest(&info)));
        assert_eq!(
            parsed.magnet(),
            format!("magnet:?xt=urn:btih:{}", parsed.info_hash)
        );
    }

    #[test]
    fn parses_multi_file_sizes_and_counts() {
        let info = dictionary(&[
            (b"files", files(&[10, 20, 30])),
            (b"name", text("合集")),
            (b"pieces", text("abc")),
        ]);
        let parsed = parse_torrent_bytes(&wrap_torrent(&info)).expect("多文件种子");
        assert_eq!(parsed.name, "合集");
        assert_eq!(parsed.file_count, 3);
        assert_eq!(parsed.total_size, 60);
    }

    #[test]
    fn accepts_trailing_whitespace_without_changing_hash() {
        let info = single_file_info();
        let mut torrent = wrap_torrent(&info);
        torrent.extend_from_slice(b"\r\n\0");
        let parsed = parse_torrent_bytes(&torrent).expect("允许尾部空白");
        assert_eq!(parsed.info_hash, format!("{:x}", Sha1::digest(&info)));
    }

    #[test]
    fn rejects_v2_only_torrents_with_a_clear_error() {
        let info = dictionary(&[
            (b"length", integer(1024)),
            (b"meta version", integer(2)),
            (b"name", text("v2")),
        ]);
        let error = parse_torrent_bytes(&wrap_torrent(&info)).expect_err("v2-only 不支持");
        assert!(error.to_string().contains("v2-only"));
    }

    #[test]
    fn rejects_missing_info_and_malformed_bencode() {
        assert!(parse_torrent_bytes(b"de").is_err());
        assert!(parse_torrent_bytes(b"d4:infoi1ee").is_err());
        assert!(parse_torrent_bytes(b"d4:info").is_err());
    }

    #[test]
    fn rejects_missing_name_and_negative_lengths() {
        let no_name = dictionary(&[(b"length", integer(1)), (b"pieces", text("abc"))]);
        assert!(parse_torrent_bytes(&wrap_torrent(&no_name)).is_err());

        let negative = dictionary(&[
            (b"length", b"i-1e".to_vec()),
            (b"name", text("bad")),
            (b"pieces", text("abc")),
        ]);
        assert!(parse_torrent_bytes(&wrap_torrent(&negative)).is_err());
    }
}
