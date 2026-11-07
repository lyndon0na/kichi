//! S3 请求签名（SigV4）与发送。

use std::collections::BTreeMap;
use std::io::Read;

use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use reqwest::blocking::Body;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, ETAG, HOST};
use reqwest::{Method, StatusCode, Url};
use sha2::{Digest as Sha256Digest, Sha256};

use crate::Error;

use super::io::{hex_lower, percent_encode};
use super::s3::S3Target;
use super::util::ensure_not_cancelled;
use super::MAX_ERROR_BODY;

#[derive(Default)]
pub(super) struct S3RequestBody<'a> {
    pub(super) content_length: Option<u64>,
    pub(super) content_type: Option<&'a str>,
    pub(super) body: Option<Body>,
}

pub(super) struct S3Response {
    pub(super) status: StatusCode,
    pub(super) etag: Option<String>,
    pub(super) content_length: Option<u64>,
    pub(super) body: String,
}

impl S3Target {
    pub(super) fn send<C>(
        &self,
        method: Method,
        query: &[(&str, String)],
        payload_hash: &str,
        body: S3RequestBody<'_>,
        cancelled: &C,
    ) -> Result<S3Response, Error>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
    {
        ensure_not_cancelled(cancelled)?;
        let mut url = Url::parse(&format!("https://{}{}", self.host, self.path))
            .map_err(|_| Error::InvalidInput("上传对象地址无效".to_owned()))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }

        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let short_date = now.format("%Y%m%d").to_string();
        let signed = sign_request(
            method.as_str(),
            url.path(),
            query,
            &self.host,
            body.content_length,
            body.content_type,
            &self.credentials.security_token,
            payload_hash,
            &amz_date,
            &short_date,
            "pikpak",
            "s3",
            &self.credentials.access_key_id,
            &self.credentials.access_key_secret,
        )?;

        let mut request = self
            .client
            .request(method, url)
            .header(HOST, &self.host)
            .header("x-amz-content-sha256", payload_hash)
            .header("x-amz-date", &amz_date)
            .header("Authorization", signed);
        if let Some(length) = body.content_length {
            request = request.header(reqwest::header::CONTENT_LENGTH, length.to_string());
        }
        if let Some(content_type) = body.content_type {
            request = request.header(CONTENT_TYPE, content_type);
        }
        if !self.credentials.security_token.trim().is_empty() {
            request = request.header(
                "x-amz-security-token",
                self.credentials.security_token.trim(),
            );
        }
        if let Some(body) = body.body {
            request = request.body(body);
        }
        let response = request.send().map_err(s3_network_error)?;
        let status = response.status();
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_length = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        let body = read_limited(response, MAX_ERROR_BODY)?;
        Ok(S3Response {
            status,
            etag,
            content_length,
            body,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn sign_request(
    method: &str,
    canonical_uri: &str,
    query: &[(&str, String)],
    host: &str,
    content_length: Option<u64>,
    content_type: Option<&str>,
    security_token: &str,
    payload_hash: &str,
    amz_date: &str,
    short_date: &str,
    region: &str,
    service: &str,
    access_key: &str,
    secret_key: &str,
) -> Result<String, Error> {
    let mut headers = BTreeMap::<String, String>::new();
    headers.insert("host".to_owned(), normalize_header(host));
    if let Some(length) = content_length {
        headers.insert("content-length".to_owned(), length.to_string());
    }
    if let Some(content_type) = content_type {
        headers.insert("content-type".to_owned(), normalize_header(content_type));
    }
    headers.insert("x-amz-content-sha256".to_owned(), payload_hash.to_owned());
    headers.insert("x-amz-date".to_owned(), amz_date.to_owned());
    if !security_token.trim().is_empty() {
        headers.insert(
            "x-amz-security-token".to_owned(),
            normalize_header(security_token),
        );
    }
    let signed_headers = headers.keys().cloned().collect::<Vec<_>>().join(";");
    let canonical_headers = headers
        .iter()
        .map(|(key, value)| format!("{key}:{value}\n"))
        .collect::<String>();
    let canonical_query = canonical_query(query);
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );
    let scope = format!("{short_date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex_lower(&Sha256::digest(canonical_request.as_bytes()))
    );
    let date_key = hmac_sha256(
        format!("AWS4{secret_key}").as_bytes(),
        short_date.as_bytes(),
    )?;
    let region_key = hmac_sha256(&date_key, region.as_bytes())?;
    let service_key = hmac_sha256(&region_key, service.as_bytes())?;
    let signing_key = hmac_sha256(&service_key, b"aws4_request")?;
    let signature = hex_lower(&hmac_sha256(&signing_key, string_to_sign.as_bytes())?);
    Ok(format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    ))
}

pub(super) fn canonical_query(query: &[(&str, String)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(key, value)| {
            (
                percent_encode(key.as_bytes()),
                percent_encode(value.as_bytes()),
            )
        })
        .collect();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

pub(super) fn normalize_header(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| Error::Other("无法计算上传签名".to_owned()))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

pub(super) fn read_limited(
    mut response: reqwest::blocking::Response,
    limit: u64,
) -> Result<String, Error> {
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Network("读取上传服务响应失败".to_owned()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(super) fn s3_network_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Network("上传请求超时".to_owned())
    } else if error.is_connect() {
        Error::Network("无法连接上传服务器".to_owned())
    } else {
        Error::Network("上传连接中断".to_owned())
    }
}
