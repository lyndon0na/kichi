//! 容量、会员、传输配额与当前用户。

use serde::Deserialize;

use crate::types::{Quota, TransferAllowance, TransferQuota};
use crate::Error;

use super::{DriveClient, UserInfo};

#[derive(Deserialize)]
pub(super) struct AboutResponse {
    #[serde(default)]
    pub(super) quota: Option<AboutQuota>,
}

/// 官方「传输配额」接口（`/vip/v1/quantity/list?type=transfer`）。
///
/// 字节数在这里是 JSON number（容量接口是字符串），用宽容解析两种情况都收。
#[derive(Deserialize)]
pub(super) struct TransferQuotaResponse {
    #[serde(default)]
    pub(super) base: Option<TransferBase>,
}

#[derive(Deserialize, Default)]
pub(super) struct TransferBase {
    #[serde(default)]
    pub(super) offline: TransferAmount,
    #[serde(default)]
    pub(super) download: TransferAmount,
    #[serde(default)]
    pub(super) upload: TransferAmount,
    #[serde(default)]
    pub(super) download_daily: TransferAmount,
    #[serde(default)]
    pub(super) vip_status: String,
}

#[derive(Deserialize, Default)]
pub(super) struct TransferAmount {
    #[serde(default, deserialize_with = "lenient_u64")]
    pub(super) total_assets: u64,
    #[serde(default, deserialize_with = "lenient_u64")]
    pub(super) assets: u64,
}

impl TransferAmount {
    pub(super) fn allowance(&self) -> TransferAllowance {
        TransferAllowance {
            limit: self.total_assets,
            used: self.assets,
        }
    }
}

#[derive(Deserialize)]
pub(super) struct AboutQuota {
    #[serde(default, deserialize_with = "lenient_u64")]
    pub(super) usage: u64,
    #[serde(default, deserialize_with = "lenient_signed")]
    pub(super) limit: i64,
    #[serde(default)]
    pub(super) is_unlimited: bool,
}

/// 官方把额度编码成字符串，且「不限量」用 `"-1"` 表示。
fn lenient_signed<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(number) => Ok(number.as_i64().unwrap_or(0)),
        serde_json::Value::String(text) => text.parse::<i64>().map_err(D::Error::custom),
        _ => Ok(0),
    }
}

/// 官方把大数字编码为字符串（`"123"`），也兼容直接给数字。
pub(super) fn lenient_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(n) => Ok(n.as_u64().unwrap_or(0)),
        serde_json::Value::String(s) => s.parse::<u64>().map_err(D::Error::custom),
        _ => Ok(0),
    }
}

impl DriveClient {
    /// 容量与用量。
    pub fn about(&self) -> Result<Quota, Error> {
        let response: AboutResponse = self.get(&self.config.about_url())?;
        let quota = response
            .quota
            .ok_or_else(|| Error::ApiChanged("about 响应缺少 quota".into()))?;
        // 会员状态只在成功时覆盖，失败按非会员处理（不让容量查询一起失败）
        let vip = self.is_vip().unwrap_or(false);
        Ok(Quota {
            usage: quota.usage,
            limit: (!quota.is_unlimited && quota.limit > 0).then_some(quota.limit as u64),
            vip,
        })
    }

    /// 月度传输配额（云下载 / 下行 / 上传）。
    ///
    /// 与 [`DriveClient::about`] 的网盘容量分开统计；官方 Web 客户端在
    /// 「传输配额」弹窗里用的就是同一个接口。响应没有重置时间字段，
    /// 界面按官方说明固定在每月 1 日 00:00 SGT 显示。
    pub fn transfer_quota(&self) -> Result<TransferQuota, Error> {
        let response: TransferQuotaResponse = self.get(&self.config.transfer_quota_url())?;
        let base = response
            .base
            .ok_or_else(|| Error::ApiChanged("quantity/list 响应缺少 base".into()))?;
        Ok(TransferQuota {
            cloud_download: base.offline.allowance(),
            download: base.download.allowance(),
            upload: base.upload.allowance(),
            download_daily: base.download_daily.allowance(),
            premium: base.vip_status == "ok",
        })
    }

    /// 只取配额用量（本地索引 canary 探针用）。
    ///
    /// 与 [`DriveClient::about`] 的区别是不顺带请求会员状态：探针每 60 秒
    /// 最多跑一次，没必要为一个字节数再发一次请求。
    pub fn usage(&self) -> Result<u64, Error> {
        let response: AboutResponse = self.get(&self.config.about_url())?;
        response
            .quota
            .map(|quota| quota.usage)
            .ok_or_else(|| Error::ApiChanged("about 响应缺少 quota".into()))
    }

    /// 排障用：`about` 的原始响应（只给集成测试打印字段名，不对外暴露）。
    #[doc(hidden)]
    pub fn about_raw_for_probe(&self) -> Result<serde_json::Value, Error> {
        self.get(&self.config.about_url())
    }

    /// 会员状态（失败不致命，`about` 里按 false 处理）。
    pub fn is_vip(&self) -> Result<bool, Error> {
        let vip: VipResponse = self.get(&self.config.vip_url())?;
        Ok(vip.data.is_some_and(|data| {
            data.status.eq_ignore_ascii_case("ok") && !data.vip_type.eq_ignore_ascii_case("novip")
        }))
    }

    /// 当前用户（昵称 / 头像 / 邮箱用来在界面上显示身份）。
    pub fn me(&self) -> Result<UserInfo, Error> {
        self.get(&self.config.user_me_url())
    }
}

#[derive(Deserialize)]
struct VipResponse {
    #[serde(default)]
    data: Option<VipData>,
}

#[derive(Deserialize)]
struct VipData {
    #[serde(default)]
    status: String,
    #[serde(default, rename = "type")]
    vip_type: String,
}
