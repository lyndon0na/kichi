//! 「缩略图」域: 纹理缓存(按字节 / 张数设上限的 LRU)与在途 / 失败登记。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use eframe::egui;

/// 纹理缓存上限(解码后的 RGBA 字节数)。
pub(crate) const TEXTURE_CAP_BYTES: usize = 64 * 1024 * 1024;
/// 纹理缓存上限(张数)。与字节上限**同时生效**, 同 `cache.rs` 的双上限口径:
/// 正常尺寸的纹理远达不到字节上限, 张数上限让浏览过的图片数量也被封顶。
pub(crate) const TEXTURE_CAP_ENTRIES: usize = 512;
/// 刚用过的纹理在此宽限期内免疫淘汰: 网格每帧点亮「可见 ± 一屏」, 因此滚动中
/// 来回经过的图不会被淘汰后立刻重新解码。
pub(crate) const TEXTURE_GRACE: Duration = Duration::from_secs(2);

struct Entry<V> {
    value: V,
    bytes: usize,
    last_used: Instant,
}

/// 已加载的缩略图纹理(file_id -> 纹理)。
///
/// 上限是**软上限**: 超限时只淘汰「距上次使用已过宽限期」的最久未用项, 全部
/// 都在宽限期内(正是当前视野里要用的)则允许暂时超出 —— 宁可多占几帧显存,
/// 也不把正在看的图淘汰掉再解码一遍。与 `cache.rs` 的磁盘缓存同一套语义。
/// 淘汰掉的图滚回视野时会重新请求, 命中磁盘缓存, 不走网络。
pub(crate) struct ThumbTextures<V> {
    entries: HashMap<String, Entry<V>>,
    bytes: usize,
    cap_bytes: usize,
    cap_entries: usize,
    grace: Duration,
}

impl<V> ThumbTextures<V> {
    pub(crate) fn new() -> Self {
        Self::with_limits(TEXTURE_CAP_BYTES, TEXTURE_CAP_ENTRIES, TEXTURE_GRACE)
    }

    fn with_limits(cap_bytes: usize, cap_entries: usize, grace: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            cap_bytes,
            cap_entries,
            grace,
        }
    }

    fn over_cap(&self) -> bool {
        self.bytes > self.cap_bytes || self.entries.len() > self.cap_entries
    }

    pub(crate) fn contains(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    pub(crate) fn get(&self, id: &str) -> Option<&V> {
        self.entries.get(id).map(|e| &e.value)
    }

    /// 当前缓存的纹理张数。
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// 当前缓存的纹理总字节数(解码后 RGBA 口径)。
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// 记录「刚被用到」, 使其在宽限期内免于淘汰。
    pub(crate) fn mark_used(&mut self, id: &str, now: Instant) {
        if let Some(e) = self.entries.get_mut(id) {
            e.last_used = now;
        }
    }

    /// 插入(或替换)一张纹理; `bytes` 为解码后 RGBA 的字节数。
    pub(crate) fn insert(&mut self, id: String, value: V, bytes: usize, now: Instant) {
        if let Some(old) = self.entries.insert(
            id,
            Entry {
                value,
                bytes,
                last_used: now,
            },
        ) {
            self.bytes -= old.bytes;
        }
        self.bytes += bytes;
    }

    /// 淘汰最久未用且已过宽限期的条目, 直到回到字节 / 张数上限内; 返回释放的字节数。
    pub(crate) fn evict(&mut self, now: Instant) -> usize {
        if !self.over_cap() {
            return 0;
        }
        let mut expired: Vec<(String, Instant)> = self
            .entries
            .iter()
            .filter(|(_, e)| now.saturating_duration_since(e.last_used) >= self.grace)
            .map(|(id, e)| (id.clone(), e.last_used))
            .collect();
        expired.sort_by_key(|(_, last)| *last);
        let mut freed = 0;
        for (id, _) in expired {
            if !self.over_cap() {
                break;
            }
            if let Some(e) = self.entries.remove(&id) {
                self.bytes -= e.bytes;
                freed += e.bytes;
            }
        }
        if freed > 0 {
            tracing::debug!(
                "缩略图纹理超限淘汰: 释放 {freed} B, 现 {} 张 / {} B",
                self.entries.len(),
                self.bytes
            );
        }
        freed
    }
}

/// 「缩略图」域: 纹理缓存 + 在途 / 失败登记。
pub(crate) struct ThumbsPage {
    /// 已加载的缩略图纹理(file_id -> TextureHandle), 带按字节上限的 LRU 淘汰。
    pub(crate) textures: ThumbTextures<egui::TextureHandle>,
    /// 正在加载缩略图的文件 id。
    inflight: HashSet<String>,
    /// 缩略图重试到底仍失败的文件 id(本次会话不再重复请求; 刷新目录 /
    /// 重新搜索时清空以再试一次)。
    failed: HashSet<String>,
}

impl Default for ThumbsPage {
    fn default() -> Self {
        Self {
            textures: ThumbTextures::new(),
            inflight: HashSet::new(),
            failed: HashSet::new(),
        }
    }
}

impl ThumbsPage {
    /// 该文件是否还需要请求缩略图(在途或已失败都不必再发)。
    pub(crate) fn needs_request(&self, id: &str) -> bool {
        !self.inflight.contains(id) && !self.failed.contains(id)
    }

    /// 登记一次在途请求。
    pub(crate) fn mark_inflight(&mut self, id: String) {
        self.inflight.insert(id);
    }

    /// 清空在途与失败登记, 让重新可见的文件有机会再请求一次(换目录 / 刷新 /
    /// 新一次搜索时调用; worker 侧也已作废旧任务)。
    pub(crate) fn reset(&mut self) {
        self.inflight.clear();
        self.failed.clear();
    }

    /// 缩略图就绪: 上传为纹理并按上限淘汰。
    pub(crate) fn on_ready(
        &mut self,
        ctx: &egui::Context,
        file_id: String,
        width: u32,
        height: u32,
        pixels: Vec<egui::Color32>,
    ) {
        self.inflight.remove(&file_id);
        let color_image = egui::ColorImage {
            size: [width as usize, height as usize],
            pixels,
        };
        let texture = ctx.load_texture(
            format!("thumb_{file_id}"),
            color_image,
            egui::TextureOptions::LINEAR,
        );
        // Color32 即 4 字节 RGBA; 纹理缓存按它记账并做上限淘汰。
        let bytes = width as usize * height as usize * 4;
        let now = Instant::now();
        self.textures.insert(file_id, texture, bytes, now);
        tracing::trace!(
            "缩略图纹理 {width}×{height} ({bytes} B), 缓存 {} 张 / {} B",
            self.textures.len(),
            self.textures.bytes()
        );
        self.textures.evict(now);
    }

    /// 缩略图失败: 结束在途登记并记住失败 —— 既不留下永不结束的 inflight,
    /// 也不在下一帧立刻重新请求同一个失败的图。
    pub(crate) fn on_failed(&mut self, file_id: String) {
        self.inflight.remove(&file_id);
        self.failed.insert(file_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用缓存: 宽限期 2s, 张数上限 1000, 每张纹理按调用方给的字节数记账。
    fn cache(cap_bytes: usize) -> ThumbTextures<u32> {
        ThumbTextures::with_limits(cap_bytes, 1000, Duration::from_secs(2))
    }

    #[test]
    fn keeps_everything_under_cap() {
        let t0 = Instant::now();
        let mut c = cache(100);
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            c.insert((*id).into(), i as u32, 10, t0);
        }
        assert_eq!(c.evict(t0 + Duration::from_secs(60)), 0);
        assert_eq!(c.len(), 3);
        assert_eq!(c.bytes(), 30);
    }

    #[test]
    fn evicts_oldest_first_when_over_cap() {
        let t0 = Instant::now();
        let mut c = cache(25);
        c.insert("a".into(), 1, 10, t0);
        c.insert("b".into(), 2, 10, t0 + Duration::from_secs(1));
        c.insert("c".into(), 3, 10, t0 + Duration::from_secs(2));
        // 已超上限(30 > 25), 且三张都过了宽限期: 淘汰最久未用的 a 即回到上限内。
        let freed = c.evict(t0 + Duration::from_secs(12));
        assert_eq!(freed, 10);
        assert_eq!(c.len(), 2);
        assert_eq!(c.bytes(), 20);
        assert!(c.get("a").is_none());
        assert!(c.get("b").is_some() && c.get("c").is_some());
    }

    #[test]
    fn evicts_exactly_to_cap_when_far_over() {
        let t0 = Instant::now();
        let mut c = cache(20);
        for (i, id) in ["a", "b", "c", "d"].iter().enumerate() {
            c.insert(
                (*id).into(),
                i as u32,
                10,
                t0 + Duration::from_secs(i as u64),
            );
        }
        // 40 字节超到 2 倍上限: 一次淘汰到刚好不超(保留最新的两张)。
        let freed = c.evict(t0 + Duration::from_secs(12));
        assert_eq!(freed, 20);
        assert_eq!(c.bytes(), 20);
        assert!(c.get("a").is_none() && c.get("b").is_none());
        assert!(c.get("c").is_some() && c.get("d").is_some());
    }

    #[test]
    fn soft_cap_keeps_entries_within_grace() {
        let t0 = Instant::now();
        let mut c = cache(10);
        c.insert("a".into(), 1, 10, t0);
        c.insert("b".into(), 2, 10, t0 + Duration::from_millis(100));
        // 两张都还在宽限期内: 允许暂时超限, 一个都不删。
        assert_eq!(c.evict(t0 + Duration::from_secs(1)), 0);
        assert_eq!(c.len(), 2);
        assert_eq!(c.bytes(), 20);
    }

    #[test]
    fn mark_used_gives_grace_to_recently_visible() {
        let t0 = Instant::now();
        let mut c = cache(20);
        c.insert("a".into(), 1, 10, t0);
        c.insert("b".into(), 2, 10, t0);
        // a 刚被点亮(仍在视野里), b 已 5s 没人看: 淘汰 b。
        c.mark_used("a", t0 + Duration::from_secs(5));
        c.insert("c".into(), 3, 10, t0 + Duration::from_secs(5));
        let freed = c.evict(t0 + Duration::from_secs(5));
        assert_eq!(freed, 10);
        assert!(c.get("b").is_none());
        assert!(c.get("a").is_some() && c.get("c").is_some());
    }

    #[test]
    fn reinsert_same_id_replaces_without_double_counting() {
        let t0 = Instant::now();
        let mut c = cache(10);
        c.insert("a".into(), 1, 10, t0);
        c.insert("a".into(), 2, 10, t0);
        assert_eq!(c.len(), 1);
        assert_eq!(c.bytes(), 10);
        assert_eq!(c.get("a"), Some(&2));
    }

    #[test]
    fn evicts_by_entry_cap_even_when_bytes_are_small() {
        let t0 = Instant::now();
        // 字节上限很宽、张数上限 2: 小图也只留最新的两张。
        let mut c = ThumbTextures::with_limits(1_000_000, 2, Duration::from_secs(2));
        c.insert("a".into(), 1, 10, t0);
        c.insert("b".into(), 2, 10, t0 + Duration::from_secs(1));
        c.insert("c".into(), 3, 10, t0 + Duration::from_secs(2));
        c.evict(t0 + Duration::from_secs(12));
        assert_eq!(c.len(), 2);
        assert!(c.get("a").is_none());
        assert!(c.get("b").is_some() && c.get("c").is_some());
    }

    #[test]
    fn mark_used_ignores_unknown_id() {
        let t0 = Instant::now();
        let mut c = cache(10);
        c.mark_used("none", t0);
        assert_eq!(c.len(), 0);
    }
}
