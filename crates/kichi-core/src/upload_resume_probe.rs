//! 上传跨重启续传探针(手动回归工具, 保留在仓库; 结论见 `docs/UPLOAD_RESUME_NOTES.md`)。
//!
//! 平时不跑; 怀疑服务端凭证有效期 / 协议行为变化时, 用它复核「落盘凭证 + upload_id +
//! ETag 在另一进程续传」是否仍然成立(stage1 与 stage2 必须分别在两个进程里跑)。
//! 目的: 实测两个「社区有先例、本项目尚未复核」的关键事实(见 `docs/UPLOAD_RESUME_NOTES.md` §6):
//! 1. `POST /drive/v1/files` 响应里 `resumable.params` 有哪些字段、`expiration` 是什么值;
//! 2. 不重新出票, 用落盘的 STS 凭证 + upload_id + ETag 在**另一个进程**里续传能否成功、数据是否一致。
//!
//! 用法(两个 stage 必须分别在两个进程里跑; stage2 建议重启机器后再跑, 才叫「跨重启」):
//! ```text
//! KICHI_UPLOAD_PROBE=1 cargo test -p kichi-core --lib upload_resume_probe::stage1 -- --ignored --nocapture
//! # 完全退出(或重启机器)
//! KICHI_UPLOAD_PROBE=1 cargo test -p kichi-core --lib upload_resume_probe::stage2 -- --ignored --nocapture
//! # stage2 若卡在「回下载」, 用这条单独复跑校验(不重传; 状态文件不在时自动按云端条目校验):
//! KICHI_UPLOAD_PROBE=1 cargo test -p kichi-core --lib upload_resume_probe::verify -- --ignored --nocapture
//! # 清历次 stage1 攒下的不可见 PENDING 占位条目(stage1 跑过多次才需要; 只动探针自己的文件):
//! KICHI_UPLOAD_PROBE=1 cargo test -p kichi-core --lib upload_resume_probe::orphans -- --ignored --nocapture
//! KICHI_UPLOAD_PROBE=1 cargo test -p kichi-core --lib upload_resume_probe::cleanup -- --ignored --nocapture
//! ```
//!
//! 安全约定:
//! - 只用 `~/.config/kichi/session.json` 里已有的登录会话, 不接受明文密码;
//! - 终端输出不含任何凭证: access_key_id / access_key_secret / security_token 只报长度;
//! - stage1 会把 STS 凭证写进 `~/.cache/kichi/probe/upload_resume_session.json`(权限 0600),
//!   这是 stage2 续传所必需的; stage2 成功后自动删除, 失败则保留以便重试(凭证约 12 小时过期);
//! - 该文件含短期凭证, **不要提交 / 分享 / 贴进任何对话**。
//!
//! 注意: stage1 每次运行都会新建一个上传票据; 未完成的上传条目在 file_list(默认过滤)里
//! 不可见(phase=PENDING), 服务端会留占位条目 —— 所以不要反复跑 stage1。

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::KichiClient;
use crate::session::{load_session, save_session, Session};
use crate::upload::{file_gcid, OssContext, OssUploadState, UploadTicket};

const PROBE_FOLDER: &str = "kichi-resume-probe";
const PROBE_FILE: &str = "kichi-resume-probe.bin";
/// 12 MiB: 按 5 MiB 分片切成 3 片, stage1 只传第 1 片。
const PROBE_SIZE: u64 = 12 * 1024 * 1024;

fn probe_dir() -> PathBuf {
    dirs::cache_dir()
        .expect("无 cache 目录")
        .join("kichi")
        .join("probe")
}

fn local_file_path() -> PathBuf {
    probe_dir().join("upload_resume_local.bin")
}

fn downloaded_file_path() -> PathBuf {
    probe_dir().join("upload_resume_back.bin")
}

fn state_file_path() -> PathBuf {
    probe_dir().join("upload_resume_session.json")
}

fn probe_enabled() -> bool {
    std::env::var("KICHI_UPLOAD_PROBE").as_deref() == Ok("1")
}

/// 未设置开关时直接跳过(测试同时标了 `#[ignore]`, 双保险)。
fn require_probe(stage: &str) -> bool {
    if probe_enabled() {
        return true;
    }
    eprintln!("[probe] 未设置 KICHI_UPLOAD_PROBE=1, 跳过 {stage}");
    false
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn mask(s: &str) -> String {
    let head: String = s.chars().take(12).collect();
    format!("{head}…(len={})", s.len())
}

/// 写文件并强制 0600(目录 0700), 用于含短期凭证的探针状态。
fn write_private(path: &Path, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).expect("创建探针目录失败");
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .expect("写入探针文件失败");
    use std::io::Write as _;
    f.write_all(bytes).expect("写入探针文件失败");
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

/// 落盘的 STS 上传上下文(与 `OssContext` 同构, 只为 serde)。
#[derive(Serialize, Deserialize)]
struct OssRecord {
    endpoint: String,
    access_key_id: String,
    access_key_secret: String,
    security_token: String,
    bucket: String,
    key: String,
}

impl OssRecord {
    fn from_ctx(c: &OssContext) -> Self {
        Self {
            endpoint: c.endpoint.clone(),
            access_key_id: c.access_key_id.clone(),
            access_key_secret: c.access_key_secret.clone(),
            security_token: c.security_token.clone(),
            bucket: c.bucket.clone(),
            key: c.key.clone(),
        }
    }

    fn to_ctx(&self) -> OssContext {
        OssContext {
            endpoint: self.endpoint.clone(),
            access_key_id: self.access_key_id.clone(),
            access_key_secret: self.access_key_secret.clone(),
            security_token: self.security_token.clone(),
            bucket: self.bucket.clone(),
            key: self.key.clone(),
        }
    }
}

/// stage1 落盘、stage2 读取的续传状态。
#[derive(Serialize, Deserialize)]
struct ProbeState {
    local_path: String,
    size: u64,
    gcid: String,
    /// 本次本地文件用的随机种子(便于复现内容)。
    #[serde(default)]
    seed: u64,
    folder_id: String,
    file_id: String,
    upload_id: String,
    oss: OssRecord,
    /// part_number -> ETag(已成功上传的分片)。
    etags: BTreeMap<u64, String>,
    stage1_at_unix: u64,
}

/// 用应用已登录的会话构建客户端(不接受明文密码)。
/// 刷新出的 token 按 GUI 的做法回写会话文件(`worker/auth.rs::apply_session`),
/// 避免探针期间 token 轮换导致应用下次启动登录态失效。
async fn probe_client() -> KichiClient {
    let session = load_session()
        .expect("读取会话失败")
        .expect("未登录: 请先启动 kichi 登录一次, 使 ~/.config/kichi/session.json 存在");
    assert!(
        !session.device_id.is_empty(),
        "会话缺少 device_id, 请在 kichi 里重新登录"
    );
    let mut client = KichiClient::new(session.device_id.clone());
    client.set_token_saver(Arc::new(|s: &Session| {
        let _ = save_session(s);
    }));
    client.set_session(&session).await;
    client
}

/// 找到或创建探针目录, 返回其 id。
async fn find_or_create_folder(client: &KichiClient) -> String {
    let list = client
        .file_list(None, 200, None)
        .await
        .expect("列出根目录失败");
    if let Some(f) = list
        .files
        .iter()
        .find(|f| f.is_folder() && f.name == PROBE_FOLDER)
    {
        return f.id.clone();
    }
    client
        .create_folder_id(PROBE_FOLDER, None)
        .await
        .expect("创建探针目录失败")
}

/// 生成伪随机内容的本地文件(xorshift64)。
/// `seed` 每次运行都不同: 内容一旦与云端某条重复就会命中秒传(gcid 去重), 那样根本没有分片可续。
fn make_local_file(path: &Path, size: u64, seed: u64) -> std::io::Result<()> {
    let mut state = seed | 1;
    let mut data = vec![0u8; size as usize];
    for b in data.iter_mut() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *b = (state >> 32) as u8;
    }
    fs::write(path, &data)
}

/// 打印票据响应结构(不打印任何凭证值)。
fn print_ticket(resp: &serde_json::Value) {
    fn keys(v: &serde_json::Value) -> Vec<String> {
        v.as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    }
    println!("[stage1] 票据响应顶层键: {:?}", keys(resp));
    if let Some(file) = resp.get("file") {
        println!(
            "[stage1] file 键: {:?}, phase={:?}",
            keys(file),
            file.get("phase").and_then(|v| v.as_str())
        );
    }
    let Some(params) = resp
        .pointer("/resumable/params")
        .and_then(|v| v.as_object())
    else {
        println!("[stage1] 响应里没有 resumable.params");
        return;
    };
    let mut names: Vec<&str> = params.keys().map(String::as_str).collect();
    names.sort_unstable();
    println!(
        "[stage1] resumable.params 键名({} 个): {names:?}",
        names.len()
    );
    for k in [
        "expiration",
        "upload_id",
        "upload_url",
        "bucket",
        "key",
        "endpoint",
    ] {
        match params.get(k) {
            Some(v) => println!("[stage1]   {k} = {v}"),
            None => println!("[stage1]   {k} = <无此字段>"),
        }
    }
    for k in ["access_key_id", "access_key_secret", "security_token"] {
        if let Some(v) = params.get(k).and_then(|v| v.as_str()) {
            println!("[stage1]   {k} = <脱敏, 长度 {}>", v.len());
        }
    }
}

fn ensure_probe_dir() -> PathBuf {
    let dir = probe_dir();
    fs::create_dir_all(&dir).expect("创建探针目录失败");
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    dir
}

#[tokio::test]
#[ignore = "实机探针: 需 KICHI_UPLOAD_PROBE=1 且已在本机登录, 见文件头注释"]
async fn stage1() {
    if !require_probe("stage1") {
        return;
    }
    let dir = ensure_probe_dir();
    let local = local_file_path();
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    make_local_file(&local, PROBE_SIZE, seed).expect("生成本地文件失败");
    let gcid = file_gcid(&local).expect("计算 gcid 失败");
    let chunk = crate::upload::upload_chunk_size(PROBE_SIZE);
    println!(
        "[stage1] 本地文件: {} ({} 字节, gcid={gcid}, 随机种子 {seed})",
        local.display(),
        PROBE_SIZE
    );
    println!(
        "[stage1] 分片 {chunk} 字节, 共 {} 片(本次只传第 1 片); 探针目录 {}",
        PROBE_SIZE.div_ceil(chunk),
        dir.display()
    );

    let client = probe_client().await;
    let folder_id = find_or_create_folder(&client).await;
    println!("[stage1] 探针目录 id: {folder_id}");

    // 1) 出票: 打印 resumable.params 的结构与 expiration
    let resp = client
        .upload_create_raw(PROBE_FILE, Some(&folder_id), PROBE_SIZE, &gcid)
        .await
        .expect("upload_create 失败");
    print_ticket(&resp);
    let file_id = resp
        .pointer("/file/id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    assert!(!file_id.is_empty(), "票据响应缺少 file.id");
    let oss = UploadTicket::from_response(&resp).oss.expect(
        "命中秒传, 没有可续传的分片: 说明这次的内容服务端已有(同 gcid) —— 重跑会换随机内容",
    );
    println!("[stage1] key = {}", oss.key);

    // 2) initiate, 只传第 1 片后取消(剩余分片留给 stage2 用同一票据续传)
    let upload_id = client.oss_initiate(&oss).await.expect("OSS initiate 失败");
    println!("[stage1] upload_id: {}", mask(&upload_id));

    client.set_part_concurrency(1);
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_cb = cancel.clone();
    let mut state = OssUploadState::default();
    let mut progress = |done: u64, total: u64| {
        println!("[stage1] 分片完成: {done}/{total}");
        cancel_cb.store(true, Ordering::Relaxed);
    };
    let err = client
        .upload_oss(
            &oss,
            &upload_id,
            &local,
            Some(cancel),
            &mut state,
            &mut progress,
            &mut |_| {},
        )
        .await
        .expect_err("预期取消后中止, 但 upload_oss 正常返回了");
    println!("[stage1] 已按预期中止: {err}");
    println!(
        "[stage1] 已成功分片: {:?}",
        state.etags.keys().collect::<Vec<_>>()
    );
    assert!(!state.etags.is_empty(), "没有成功上传任何分片, 无法继续");

    // 3) 落盘续传状态(含短期 STS 凭证, 0600)
    let st = ProbeState {
        local_path: local.to_string_lossy().into_owned(),
        size: PROBE_SIZE,
        gcid,
        seed,
        folder_id,
        file_id,
        upload_id,
        oss: OssRecord::from_ctx(&oss),
        etags: state.etags.into_iter().collect(),
        stage1_at_unix: now_unix(),
    };
    let text = serde_json::to_string_pretty(&st).expect("序列化探针状态失败");
    write_private(&state_file_path(), text.as_bytes());
    println!(
        "[stage1] 续传状态已写入 {} (0600, 含短期凭证, 勿分享)",
        state_file_path().display()
    );
    println!("[stage1] 完成 —— 请完全退出(或重启)后运行 stage2。");
}

/// 单次请求/解析直链的超时(客户端本身没配超时, 连接被挂起会一直等)。
const LINK_TIMEOUT: Duration = Duration::from_secs(60);
/// 12 MiB 回下载的整体超时。实测这条线路很慢(12 MiB 用了 128.55s ≈ 98 KB/s),
/// 给足余量避免把「慢」误判成「挂起」; 有每 2 MiB 的进度打点, 真挂起也看得出来。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

fn load_state_opt() -> Option<ProbeState> {
    let text = fs::read_to_string(state_file_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn load_state() -> ProbeState {
    load_state_opt().expect("读取探针状态失败: 请先在另一个进程里跑 stage1")
}

/// 只取 URL 的 `scheme://host`, 用于在日志里确认直链落在哪个域名(不打印完整签名 URL)。
fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split(['/', '?']).next().unwrap_or(rest).to_string()
}

/// 回下载校验的目标。
struct VerifyTarget {
    folder_id: String,
    file_id: String,
    /// 期望字节数(以本地文件为准)。
    size: u64,
    /// 期望 gcid(以本地文件为准)。
    gcid: String,
    /// 来源说明, 仅用于日志。
    source: String,
}

impl ProbeState {
    fn target(&self) -> VerifyTarget {
        VerifyTarget {
            folder_id: self.folder_id.clone(),
            file_id: self.file_id.clone(),
            size: self.size,
            gcid: self.gcid.clone(),
            source: "探针状态(stage1 落盘)".to_string(),
        }
    }
}

/// 在根目录里找探针目录(不创建)。
async fn find_folder(client: &KichiClient) -> Result<Option<String>, String> {
    let list = match tokio::time::timeout(LINK_TIMEOUT, client.file_list(None, 200, None)).await {
        Ok(Ok(l)) => l,
        Ok(Err(e)) => return Err(format!("列出根目录失败: {e}")),
        Err(_) => return Err("列出根目录超时".to_string()),
    };
    Ok(list
        .files
        .iter()
        .find(|f| f.is_folder() && f.name == PROBE_FOLDER)
        .map(|f| f.id.clone()))
}

/// 状态文件不在时的兜底: 在云端按名字找条目, 用本地文件算期望的 gcid/大小。
/// 注意本地文件每次 stage1 都会换成新的随机内容, 所以这条路径只在「本地文件就是刚上传的那份」时才有意义。
async fn target_from_cloud(client: &KichiClient) -> Result<VerifyTarget, String> {
    let local = local_file_path();
    let size = fs::metadata(&local)
        .map_err(|e| format!("本地文件不可用({}): {e}", local.display()))?
        .len();
    let gcid = file_gcid(&local).map_err(|e| format!("计算本地 gcid 失败: {e}"))?;

    let folder_id = find_folder(client)
        .await?
        .ok_or_else(|| format!("根目录里没有 {PROBE_FOLDER} 目录"))?;
    let children =
        match tokio::time::timeout(LINK_TIMEOUT, client.file_list(Some(&folder_id), 200, None))
            .await
        {
            Ok(Ok(l)) => l,
            Ok(Err(e)) => return Err(format!("列出探针目录失败: {e}")),
            Err(_) => return Err("列出探针目录超时".to_string()),
        };
    let hits: Vec<&crate::types::File> = children
        .files
        .iter()
        .filter(|f| f.name == PROBE_FILE)
        .collect();
    let Some(f) = hits.first() else {
        return Err(format!("探针目录里没有 {PROBE_FILE}"));
    };
    if hits.len() > 1 {
        println!(
            "[verify] 注意: 目录里有 {} 个同名条目, 取第一个",
            hits.len()
        );
    }
    Ok(VerifyTarget {
        folder_id,
        file_id: f.id.clone(),
        size,
        gcid,
        source: format!("云端条目 + {}", local.display()),
    })
}

/// 校验云端条目: 出现在默认列表(phase=COMPLETE) → 解析直链 → 回下载 → 比对 gcid。
/// 每一步都打点 + 硬超时, 便于定位是「挂起」还是「慢」。
async fn verify_and_download(client: &KichiClient, t: &VerifyTarget) -> bool {
    println!(
        "[verify] 校验目标: {} (期望 {} 字节, gcid={})",
        t.source, t.size, t.gcid
    );
    println!("[verify] 等待条目出现在目录列表(默认过滤只看得到 phase=COMPLETE)…");
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut found: Option<(String, i64, Option<String>)> = None;
    while Instant::now() < deadline {
        match tokio::time::timeout(
            LINK_TIMEOUT,
            client.file_list(Some(&t.folder_id), 200, None),
        )
        .await
        {
            Ok(Ok(list)) => {
                if let Some(f) = list.files.iter().find(|f| f.id == t.file_id) {
                    found = Some((f.name.clone(), f.size, f.phase.clone()));
                    break;
                }
            }
            Ok(Err(e)) => {
                println!("[verify] FAIL: 列出探针目录失败: {e}");
                return false;
            }
            Err(_) => println!("[verify] 列目录超时({}s), 继续等", LINK_TIMEOUT.as_secs()),
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    let Some((name, csize, phase)) = found else {
        println!("[verify] FAIL: 90 秒内云端仍未出现该条目(可能还是 PENDING)");
        return false;
    };
    println!("[verify] 云端条目: name={name} size={csize} phase={phase:?}");
    if csize != t.size as i64 {
        println!("[verify] FAIL: 云端大小 {csize} 与本地 {} 不一致", t.size);
        return false;
    }

    println!("[verify] 解析直链…");
    let link = match tokio::time::timeout(LINK_TIMEOUT, client.file_download_link(&t.file_id)).await
    {
        Ok(Ok(l)) => l,
        Ok(Err(e)) => {
            println!("[verify] FAIL: 解析直链失败: {e}");
            return false;
        }
        Err(_) => {
            println!(
                "[verify] FAIL: 解析直链超时({}s) —— 请求被挂起",
                LINK_TIMEOUT.as_secs()
            );
            return false;
        }
    };
    println!(
        "[verify] 直链域名 {} / 声明大小 {} 字节",
        host_of(&link.url),
        link.size
    );

    let back = downloaded_file_path();
    let _ = fs::remove_file(&back);
    // 一并清掉下载残留的 .part, 避免上次的残片被当成续传起点。
    let _ = fs::remove_file(crate::download::part_path(&back));
    let mut last = 0u64;
    let mut progress = |total: u64, done: u64| {
        if done.saturating_sub(last) >= 2 * 1024 * 1024 || done == total {
            last = done;
            println!("[verify] 已下载 {done}/{total} 字节");
        }
    };
    println!(
        "[verify] 开始回下载(整体上限 {}s)…",
        DOWNLOAD_TIMEOUT.as_secs()
    );
    let got = match tokio::time::timeout(
        DOWNLOAD_TIMEOUT,
        client.download_to(&link, &back, None, &mut progress),
    )
    .await
    {
        Ok(Ok(n)) => n,
        Ok(Err(e)) => {
            println!("[verify] FAIL: 下载失败: {e}");
            return false;
        }
        Err(_) => {
            println!(
                "[verify] FAIL: 下载超时({}s) —— 连接被挂起",
                DOWNLOAD_TIMEOUT.as_secs()
            );
            return false;
        }
    };
    let back_gcid = file_gcid(&back).expect("计算下载文件 gcid 失败");
    println!("[verify] 下载 {got} 字节, gcid={back_gcid}");
    if back_gcid == t.gcid {
        println!("[verify] PASS: 回下载内容与本地文件一致(gcid 相同)");
        true
    } else {
        println!(
            "[verify] FAIL: gcid 不一致(云端 {back_gcid} vs 本地 {})",
            t.gcid
        );
        false
    }
}

#[tokio::test]
#[ignore = "实机探针: 需先跑 stage1, 且 KICHI_UPLOAD_PROBE=1, 见文件头注释"]
async fn stage2() {
    if !require_probe("stage2") {
        return;
    }
    let st = load_state();
    let elapsed_min = now_unix().saturating_sub(st.stage1_at_unix) / 60;
    println!("[stage2] 距 stage1 已过 {elapsed_min} 分钟(社区先例称凭证有效期约 12 小时)");
    println!(
        "[stage2] 复用 key={} upload_id={}",
        st.oss.key,
        mask(&st.upload_id)
    );
    println!(
        "[stage2] 已传分片: {:?}",
        st.etags.keys().collect::<Vec<_>>()
    );

    let local = PathBuf::from(&st.local_path);
    assert!(local.exists(), "本地文件不存在: {}", local.display());
    let client = probe_client().await;

    // 1) 不重新出票, 直接用落盘凭证 + upload_id 续传剩余分片并完成上传
    let mut oss_state = OssUploadState {
        etags: st
            .etags
            .iter()
            .map(|(p, e)| (*p, e.clone()))
            .collect::<HashMap<_, _>>(),
    };
    let mut progress = |done: u64, total: u64| println!("[stage2] 分片完成: {done}/{total}");
    let uploaded = client
        .upload_oss(
            &st.oss.to_ctx(),
            &st.upload_id,
            &local,
            None,
            &mut oss_state,
            &mut progress,
            &mut |_| {},
        )
        .await
        .expect("续传失败(凭证过期 / key 失配 / upload_id 失效?) —— 这正是要验证的点");
    println!("[stage2] upload_oss 返回 {uploaded} 字节(分片上传已完成)");

    let same = verify_and_download(&client, &st.target()).await;
    if same {
        let _ = fs::remove_file(state_file_path());
        println!("[stage2] 已删除含凭证的探针状态文件; 云端与本地文件请用 cleanup 清理");
    } else {
        println!("[stage2] 校验未通过, 保留探针状态文件; 可用 upload_resume_probe::verify 单独复跑回下载");
    }
    assert!(same, "跨进程续传结果与本地文件不一致");
}

/// 只做回下载校验(不重传): 对已完成的云端条目复跑, 用于定位卡点是「解析直链」还是「下载」。
/// 优先用 stage1 落盘的探针状态; 状态文件不在(被 stage2 成功删除或手动删除)时,
/// 退回「云端按名字找条目 + 本地文件当期望值」。
#[tokio::test]
#[ignore = "实机探针: 需 KICHI_UPLOAD_PROBE=1 且云端已有探针文件, 见文件头注释"]
async fn verify() {
    if !require_probe("verify") {
        return;
    }
    let client = probe_client().await;
    let target = match load_state_opt() {
        Some(st) => st.target(),
        None => {
            println!("[verify] 探针状态文件不在, 改为按云端条目 + 本地文件校验");
            match target_from_cloud(&client).await {
                Ok(t) => t,
                Err(e) => {
                    println!("[verify] FAIL: {e}");
                    panic!("无法确定校验目标: {e}");
                }
            }
        }
    };
    let same = verify_and_download(&client, &target).await;
    assert!(same, "回下载校验未通过");
}

/// 清掉历次 stage1 留下的**孤儿占位条目**(`phase=PENDING`), 顺便试出「列未完成条目的 filters 写法」——
/// 出票即产生占位条目, 而默认过滤(`phase=eq PHASE_TYPE_COMPLETE`)看不到它们, cleanup 只删得掉
/// 状态文件里记着的那一个 id, 所以多跑几次 stage1 就会攒下够不到的孤儿。
/// 安全边界: 只对名字含 `kichi-resume-probe` 的条目动手, 不碰其他文件。
#[tokio::test]
#[ignore = "实机探针: 需 KICHI_UPLOAD_PROBE=1, 见文件头注释"]
async fn orphans() {
    if !require_probe("orphans") {
        return;
    }
    let client = probe_client().await;
    let folder_id = match find_folder(&client).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            println!("[orphans] 根目录没有探针目录 —— 孤儿占位条目是它的子条目, 够不到, 只能等服务端回收");
            return;
        }
        Err(e) => {
            println!("[orphans] FAIL: {e}");
            return;
        }
    };

    // 逐个试 filters 写法, 看服务端认哪一种(这是将来实现里「列出/取消未完成上传」要用的能力)。
    let shapes = [
        (
            "phase in 三态",
            serde_json::json!({"trashed": {"eq": false}, "phase": {"in": ["PHASE_TYPE_PENDING", "PHASE_TYPE_RUNNING", "PHASE_TYPE_COMPLETE"]}}),
        ),
        (
            "phase eq PENDING",
            serde_json::json!({"trashed": {"eq": false}, "phase": {"eq": "PHASE_TYPE_PENDING"}}),
        ),
        ("空 filters", serde_json::json!({})),
    ];
    let mut working: Option<serde_json::Value> = None;
    for (label, filters) in &shapes {
        match tokio::time::timeout(
            LINK_TIMEOUT,
            client.file_list_filtered(filters.clone(), Some(&folder_id), 200, None),
        )
        .await
        {
            Ok(Ok(list)) => {
                let shown: Vec<String> = list
                    .files
                    .iter()
                    .map(|f| format!("{}[{}]", f.name, f.phase.clone().unwrap_or_default()))
                    .collect();
                println!(
                    "[orphans] filters {label}: {} 条 {shown:?}",
                    list.files.len()
                );
                if working.is_none() {
                    working = Some(filters.clone());
                }
            }
            Ok(Err(e)) => println!("[orphans] filters {label}: 被拒({e})"),
            Err(_) => println!("[orphans] filters {label}: 超时"),
        }
    }
    let Some(filters) = working else {
        println!("[orphans] 没试出能列出未完成条目的写法");
        return;
    };

    let list = client
        .file_list_filtered(filters, Some(&folder_id), 200, None)
        .await
        .expect("列出条目失败");
    let ids: Vec<String> = list
        .files
        .iter()
        .filter(|f| f.name.contains("kichi-resume-probe"))
        .map(|f| f.id.clone())
        .collect();
    if ids.is_empty() {
        println!("[orphans] 没有名字含 kichi-resume-probe 的条目, 无需清理");
        return;
    }
    println!("[orphans] 待清条目 {} 个(可见 + PENDING 占位)", ids.len());
    match client.batch_trash(&ids).await {
        Ok(_) => match client.batch_delete(&ids).await {
            Ok(_) => println!("[orphans] 已彻底删除 {} 个条目", ids.len()),
            Err(e) => println!("[orphans] 彻底删除失败({e}); 条目已在回收站"),
        },
        Err(e) => println!(
            "[orphans] 移入回收站失败({e}) —— PENDING 占位可能不吃 batchTrash, 需要 :cancelUpload"
        ),
    }
}

#[tokio::test]
#[ignore = "实机探针: 需 KICHI_UPLOAD_PROBE=1, 见文件头注释"]
async fn cleanup() {
    if !require_probe("cleanup") {
        return;
    }
    let state = load_state_opt();
    let client = probe_client().await;
    let list = client
        .file_list(None, 200, None)
        .await
        .expect("列出根目录失败");
    let Some(folder) = list
        .files
        .into_iter()
        .find(|f| f.is_folder() && f.name == PROBE_FOLDER)
    else {
        println!("[cleanup] 根目录没有探针目录, 云端无需清理");
        remove_local_files();
        return;
    };
    let mut ids: Vec<String> = client
        .file_list(Some(&folder.id), 200, None)
        .await
        .expect("列出探针目录失败")
        .files
        .iter()
        .map(|f| f.id.clone())
        .collect();
    // 默认过滤看不到 PENDING 占位条目, 用 state 里的 id 兜底。
    if let Some(st) = &state {
        if !ids.contains(&st.file_id) {
            ids.push(st.file_id.clone());
        }
    }
    ids.push(folder.id.clone());
    match client.batch_trash(&ids).await {
        Ok(_) => match client.batch_delete(&ids).await {
            Ok(_) => println!("[cleanup] 已彻底删除 {} 个云端条目(含探针目录)", ids.len()),
            Err(e) => {
                eprintln!("[cleanup] 彻底删除失败({e}); 条目已在回收站, 可在应用里清空回收站")
            }
        },
        Err(e) => eprintln!("[cleanup] 移入回收站失败({e}); 请手动删除云端的 {PROBE_FOLDER} 目录"),
    }
    remove_local_files();
    println!("[cleanup] 完成");
}

fn remove_local_files() {
    let back = downloaded_file_path();
    for p in [
        state_file_path(),
        local_file_path(),
        back.clone(),
        crate::download::part_path(&back),
    ] {
        match fs::remove_file(&p) {
            Ok(()) => println!("[cleanup] 已删除本地 {}", p.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("[cleanup] 删除 {} 失败: {e}", p.display()),
        }
    }
}
