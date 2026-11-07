//! S3 XML 响应解析与小工具。

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::upload_state::UploadPart;
use crate::Error;

pub(super) struct ListPartsPage {
    pub(super) parts: Vec<UploadPart>,
    pub(super) truncated: bool,
    pub(super) next_marker: Option<u64>,
}

pub(super) fn parse_list_parts(xml: &str) -> Result<ListPartsPage, Error> {
    let mut reader = Reader::from_str(xml);
    let mut parts = Vec::new();
    let mut current: Option<UploadPart> = None;
    let mut field: Vec<u8> = Vec::new();
    let mut text = String::new();
    let mut truncated = false;
    let mut next_marker = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let name = event.local_name().as_ref().to_vec();
                if name == b"Part" {
                    current = Some(UploadPart {
                        number: 0,
                        etag: String::new(),
                        size: 0,
                    });
                    field.clear();
                    continue;
                }
                if matches!(
                    name.as_slice(),
                    b"PartNumber" | b"ETag" | b"Size" | b"IsTruncated" | b"NextPartNumberMarker"
                ) {
                    field = name;
                    text.clear();
                }
            }
            Ok(Event::Text(event)) if !field.is_empty() => {
                text.push_str(&String::from_utf8_lossy(event.as_ref()));
            }
            Ok(Event::GeneralRef(event)) if !field.is_empty() => {
                push_xml_ref(&mut text, event.as_ref());
            }
            Ok(Event::End(event)) => {
                let name = event.local_name().as_ref().to_vec();
                if name == b"Part" {
                    let Some(part) = current.take() else {
                        return Err(Error::ApiChanged("分片列表缺少分片内容".to_owned()));
                    };
                    if part.number == 0 || part.etag.trim().is_empty() || part.size == 0 {
                        return Err(Error::ApiChanged("分片列表包含不完整分片".to_owned()));
                    }
                    parts.push(part);
                    field.clear();
                    text.clear();
                    continue;
                }
                if name != field {
                    continue;
                }
                match name.as_slice() {
                    b"PartNumber" if current.is_some() => {
                        let number = parse_xml_u64(&text, "分片号")?;
                        if let Some(part) = current.as_mut() {
                            part.number = number;
                        }
                    }
                    b"ETag" if current.is_some() => {
                        if let Some(part) = current.as_mut() {
                            part.etag = text.trim().to_owned();
                        }
                    }
                    b"Size" if current.is_some() => {
                        let size = parse_xml_u64(&text, "分片大小")?;
                        if let Some(part) = current.as_mut() {
                            part.size = size;
                        }
                    }
                    b"IsTruncated" => {
                        truncated = matches!(text.trim(), "true" | "1");
                    }
                    b"NextPartNumberMarker" => {
                        next_marker = Some(parse_xml_u64(&text, "下一分片标记")?);
                    }
                    _ => {}
                }
                field.clear();
                text.clear();
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(Error::ApiChanged(format!("无法解析分片列表：{error}")));
            }
            _ => {}
        }
    }
    Ok(ListPartsPage {
        parts,
        truncated,
        next_marker,
    })
}

pub(super) fn parse_xml_u64(text: &str, what: &str) -> Result<u64, Error> {
    text.trim()
        .parse::<u64>()
        .map_err(|_| Error::ApiChanged(format!("分片列表的{what}无效")))
}

pub(super) fn push_xml_ref(out: &mut String, reference: &[u8]) {
    match reference {
        b"amp" => out.push('&'),
        b"lt" => out.push('<'),
        b"gt" => out.push('>'),
        b"quot" => out.push('"'),
        b"apos" => out.push('\''),
        _ => {
            out.push('&');
            out.push_str(&String::from_utf8_lossy(reference));
            out.push(';');
        }
    }
}

pub(super) fn xml_text(xml: &[u8], tag: &[u8]) -> Option<String> {
    let mut reader = Reader::from_reader(xml);
    let mut in_wanted = false;
    let mut out = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                in_wanted = event.name().as_ref() == tag;
                if in_wanted {
                    out.clear();
                }
            }
            Ok(Event::Text(event)) if in_wanted => {
                out.push_str(&String::from_utf8_lossy(event.as_ref()));
            }
            Ok(Event::GeneralRef(event)) if in_wanted => {
                let reference = event.as_ref();
                match reference {
                    b"amp" => out.push('&'),
                    b"lt" => out.push('<'),
                    b"gt" => out.push('>'),
                    b"quot" => out.push('"'),
                    b"apos" => out.push('\''),
                    _ => {
                        let name = String::from_utf8_lossy(reference);
                        out.push('&');
                        out.push_str(&name);
                        out.push(';');
                    }
                }
            }
            Ok(Event::End(event)) if event.name().as_ref() == tag && in_wanted => {
                return Some(out);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    None
}

pub(super) fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(super) fn parse_s3_error_body(body: &str) -> Option<(String, String)> {
    let code = xml_text(body.as_bytes(), b"Code")?;
    let message = xml_text(body.as_bytes(), b"Message").unwrap_or_default();
    Some((code, message))
}
