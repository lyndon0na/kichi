//! 本地上传相关模型与算法。
//!
//! 机制(参考社区逆向 pikpaktui / pikpakcli / rclone):
//! 1. 先算 gcid(秒传哈希), 用 `POST /drive/v1/files` 创建上传票据;
//! 2. `file.phase == PHASE_TYPE_COMPLETE` 表示秒传命中, 无需上传;
//! 3. 否则按返回的 OSS 上下文做阿里云 OSS 分片上传(签名用 HMAC-SHA1)。

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::SystemTime;

use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha1::{Digest, Sha1};

use crate::error::Error;

/// gcid 分块的初始大小(256KB)。
const MIN_BLOCK: u64 = 0x40000;
/// gcid 分块的上限(2MB)。
const MAX_BLOCK: u64 = 0x200000;
/// OSS 分片的最小块(5MB), 保证分片数不超过上限。
const MIN_CHUNK: u64 = 5 * 1024 * 1024;
/// OSS 单个分片数上限。
const MAX_PARTS: u64 = 10_000;

/// 计算某文件大小下的 gcid 分块大小。
///
/// 与 rclone / pikpakhash 一致: 从 256KB 起, 当 `size/block > 512` 且未达 2MB 时翻倍。
pub fn gcid_block_size(size: u64) -> u64 {
    let mut psize = MIN_BLOCK;
    while psize < MAX_BLOCK && size / psize > 0x200 {
        psize <<= 1;
    }
    psize
}

/// 计算文件的 gcid(小写 hex): 分块 sha1, 再对各块「原始摘要」的拼接取 sha1。
/// 空文件为零块, 结果为 `sha1("")`。
pub fn file_gcid(path: &Path) -> Result<String, Error> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let block = gcid_block_size(size).max(1) as usize;
    let mut buf = vec![0u8; block];
    let mut total = Sha1::new();
    let mut remaining = size;
    while remaining > 0 {
        let n = remaining.min(block as u64) as usize;
        file.read_exact(&mut buf[..n])?;
        let mut h = Sha1::new();
        h.update(&buf[..n]);
        total.update(h.finalize());
        remaining -= n as u64;
    }
    Ok(hex_lower(total.finalize()))
}

/// OSS 分片上传的块大小: 至少 5MB, 且保证分片数 ≤ 10000。
pub fn upload_chunk_size(size: u64) -> u64 {
    size.div_ceil(MAX_PARTS).max(MIN_CHUNK)
}

/// 某个分片负责的字节数(最后一片可能不足)。
pub fn part_len(part: u64, chunk: u64, size: u64) -> u64 {
    let offset = (part - 1) * chunk;
    size.saturating_sub(offset).min(chunk)
}

/// OSS 分片上传的跨次重试状态: 保存已成功分片的 ETag (part_number -> etag)。
/// 同一个 `upload_id` 内续传时用它跳过已上传的分片。
#[derive(Debug, Default)]
pub struct OssUploadState {
    pub etags: std::collections::HashMap<u64, String>,
}

impl OssUploadState {
    /// 已上传分片对应的字节数合计。
    pub fn uploaded_bytes(&self, chunk: u64, size: u64) -> u64 {
        self.etags.keys().map(|p| part_len(*p, chunk, size)).sum()
    }
}

/// 创建上传票据后从响应解析出的上传信息。
#[derive(Debug, Clone)]
pub struct UploadTicket {
    /// 秒传: 服务端已有相同内容, 无需上传。
    pub completed: bool,
    /// 需要实际上传时的 OSS 上下文。
    pub oss: Option<OssContext>,
}

impl UploadTicket {
    /// 从 `POST /drive/v1/files` 的响应中防御式解析。
    pub fn from_response(v: &Value) -> UploadTicket {
        let phase = v
            .get("file")
            .and_then(|f| f.get("phase"))
            .and_then(|p| p.as_str())
            .unwrap_or_default();
        let completed = phase == "PHASE_TYPE_COMPLETE";
        UploadTicket {
            completed,
            oss: if completed { None } else { parse_oss(v) },
        }
    }
}

/// 阿里云 OSS 临时凭证与目标对象。
#[derive(Debug, Clone)]
pub struct OssContext {
    pub endpoint: String,
    pub access_key_id: String,
    pub access_key_secret: String,
    pub security_token: String,
    pub bucket: String,
    pub key: String,
}

fn parse_oss(v: &Value) -> Option<OssContext> {
    let p = v.get("resumable")?.get("params")?;
    let s = |k: &str| {
        p.get(k)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Some(OssContext {
        endpoint: s("endpoint")?,
        access_key_id: s("access_key_id").unwrap_or_default(),
        access_key_secret: s("access_key_secret").unwrap_or_default(),
        security_token: s("security_token").unwrap_or_default(),
        bucket: s("bucket")?,
        key: s("key")?,
    })
}

/// 构造 OSS 请求的 `Authorization` 头。
///
/// `query` 为 URL 中 `?` 之后的原始串(不含 `?`), 必须与实际请求完全一致;
/// `date` 为 RFC1123 GMT 时间(如 `Mon, 16 Sep 2026 12:00:00 GMT`)。
pub fn oss_authorization(method: &str, date: &str, oss: &OssContext, query: &str) -> String {
    type HmacSha1 = Hmac<Sha1>;

    let canonicalized_headers = format!("x-oss-security-token:{}\n", oss.security_token);
    let resource = if query.is_empty() {
        format!("/{}/{}", oss.bucket, oss.key)
    } else {
        format!("/{}/{}?{query}", oss.bucket, oss.key)
    };
    // string_to_sign: METHOD\n<Content-MD5>\n<Content-Type>\n<Date>\n<canonicalized headers><resource>
    let string_to_sign = format!(
        "{method}\n\napplication/octet-stream\n{date}\n{canonicalized_headers}{resource}"
    );

    let mut mac = HmacSha1::new_from_slice(oss.access_key_secret.as_bytes())
        .expect("HMAC 接受任意长度密钥");
    mac.update(string_to_sign.as_bytes());
    let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    format!("OSS {}:{}", oss.access_key_id, sig)
}

/// 当前的 RFC1123 GMT 时间, 用于 OSS 请求的 `Date` 头。
pub fn http_date_now() -> String {
    httpdate::fmt_http_date(SystemTime::now())
}

/// 构造 OSS 完成分片上传的 XML 请求体。
pub fn complete_multipart_body(etags: &[String]) -> String {
    let mut xml = String::from("<CompleteMultipartUpload>");
    for (i, etag) in etags.iter().enumerate() {
        xml.push_str(&format!(
            "<Part><PartNumber>{}</PartNumber><ETag>{etag}</ETag></Part>",
            i + 1
        ));
    }
    xml.push_str("</CompleteMultipartUpload>");
    xml
}

/// 从形如 `<Tag>value</Tag>` 的 XML 中取出 `value`(简单、非通用解析)。
pub fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}

fn hex_lower(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write as _;
    let bytes = bytes.as_ref();
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(label: &str, data: &[u8]) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("kichi-upload-{label}-{nanos}"));
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn block_size_thresholds() {
        assert_eq!(gcid_block_size(1), 256 * 1024);
        assert_eq!(gcid_block_size(200 * 1024 * 1024), 512 * 1024);
        assert_eq!(gcid_block_size(500 * 1024 * 1024), 1024 * 1024);
        assert_eq!(gcid_block_size(1024 * 1024 * 1024), 2 * 1024 * 1024);
    }

    #[test]
    fn gcid_golden_vectors() {
        let empty = temp_file("empty", b"");
        assert_eq!(
            file_gcid(&empty).unwrap(),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        std::fs::remove_file(&empty).unwrap();

        let hello = temp_file("hello", b"hello");
        assert_eq!(
            file_gcid(&hello).unwrap(),
            "6b4f89a54e2d27ecd7e8da05b4ab8fd9d1d8b119"
        );
        std::fs::remove_file(&hello).unwrap();

        // 300KB -> 256KB 块, 第二块 44KB。
        let big = temp_file("300k", &vec![b'A'; 300 * 1024]);
        assert_eq!(
            file_gcid(&big).unwrap(),
            "6764b9b8d20625682ec6ae5115f6d72386ae6294"
        );
        std::fs::remove_file(&big).unwrap();
    }

    #[test]
    fn upload_chunk_size_at_least_5mb_and_bounded_parts() {
        assert_eq!(upload_chunk_size(1), MIN_CHUNK);
        // 100GB: 块大小略大于 10MB, 且分片数不超过 10000。
        let size = 100u64 * 1024 * 1024 * 1024;
        let chunk = upload_chunk_size(size);
        assert!(chunk >= 10 * 1024 * 1024);
        assert!(size.div_ceil(chunk) <= MAX_PARTS);
    }

    #[test]
    fn oss_authorization_golden_vector() {
        let oss = OssContext {
            endpoint: "oss.example.com".into(),
            access_key_id: "AKID".into(),
            access_key_secret: "SECRET".into(),
            security_token: "TOKEN".into(),
            bucket: "bkt".into(),
            key: "dir/file.bin".into(),
        };
        let auth = oss_authorization(
            "PUT",
            "Mon, 16 Sep 2026 12:00:00 GMT",
            &oss,
            "partNumber=1&uploadId=UP1",
        );
        assert_eq!(auth, "OSS AKID:aLOyIHR+8CB/WM/SlWjX2reBF/c=");
    }

    #[test]
    fn parse_ticket_complete_and_resumable() {
        let done = UploadTicket::from_response(&serde_json::json!({
            "file": { "phase": "PHASE_TYPE_COMPLETE" }
        }));
        assert!(done.completed);
        assert!(done.oss.is_none());

        let pending = UploadTicket::from_response(&serde_json::json!({
            "file": { "phase": "PHASE_TYPE_PENDING" },
            "resumable": { "params": {
                "endpoint": "oss.example.com", "access_key_id": "a",
                "access_key_secret": "s", "security_token": "t",
                "bucket": "b", "key": "k"
            }}
        }));
        assert!(!pending.completed);
        let oss = pending.oss.unwrap();
        assert_eq!(oss.endpoint, "oss.example.com");
        assert_eq!(oss.key, "k");
    }

    #[test]
    fn part_len_and_uploaded_bytes() {
        assert_eq!(part_len(1, 5, 12), 5);
        assert_eq!(part_len(2, 5, 12), 5);
        assert_eq!(part_len(3, 5, 12), 2);
        assert_eq!(part_len(1, 5, 0), 0);

        let mut st = OssUploadState::default();
        st.etags.insert(1, "e1".into());
        st.etags.insert(2, "e2".into());
        assert_eq!(st.uploaded_bytes(5, 12), 10);
    }

    #[test]
    fn complete_body_and_xml_tag() {
        let body = complete_multipart_body(&["e1".into(), "e2".into()]);
        assert!(body.contains("<PartNumber>1</PartNumber><ETag>e1</ETag>"));
        assert!(body.contains("<PartNumber>2</PartNumber><ETag>e2</ETag>"));
        assert_eq!(
            extract_xml_tag("<X><UploadId>UP</UploadId></X>", "UploadId").as_deref(),
            Some("UP")
        );
    }
}
