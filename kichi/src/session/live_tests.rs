//! 真实账号的集成测试（默认 `#[ignore]`）。
//!
//! 前置条件：先在本机用 GUI 登录一次，令牌已存进 KWallet。
//! 运行：`cargo test -p kichi --bin kichi -- --ignored`

use super::*;
use kichi_core::TaskPhase;
/// 真实会话的自动刷新验证（默认忽略）。
///
/// 前置条件：先在本机用 GUI 登录一次，令牌已存进 KWallet。
/// 做法：把内存里的过期时间压到 1 分钟后，`ensure_tokens` 应判定
/// 「5 分钟内到期」而主动刷新，并用新令牌成功查询容量。
#[test]
#[ignore = "需要先在 GUI 登录一次（会话在 KWallet 里）"]
fn live_auto_refresh_with_stored_session() {
    let path = store_path();
    let loaded = store::load(&path).expect("读取会话存储失败");
    let Some((mut saved, backend)) = loaded else {
        panic!("KWallet 里没有会话，请先在 GUI 登录一次再跑这个测试");
    };
    println!("会话来源：{backend:?}");

    let old_access = saved.access_token.clone();
    // 压到 1 分钟后过期 -> 命中「提前 5 分钟刷新」
    saved.expires_unix = Utc::now().timestamp() + 60;
    let device_id = saved.device_id.clone();
    let config = Config::from_env();
    let session = Session::new(config, path.clone(), Some(saved), device_id);

    session.ensure_tokens().expect("自动刷新失败");

    let refreshed = session.with_inner(|inner| {
        inner
            .current
            .as_ref()
            .map(|s| (s.access_token.clone(), s.expires_unix))
    });
    let Some((new_access, expires_unix)) = refreshed else {
        panic!("刷新后会话丢失");
    };
    assert_ne!(
        old_access, new_access,
        "access_token 没有变化，可能没真正刷新"
    );

    let remaining = expires_unix - Utc::now().timestamp();
    println!("刷新成功，新令牌剩余 {remaining} 秒");
    assert!(remaining > 300, "新令牌有效期异常：{remaining} 秒");

    let quota = session.quota().expect("用新令牌查询容量失败");
    println!(
        "新令牌可用：已用 {} GB，上限 {:?}",
        quota.usage / 1024 / 1024 / 1024,
        quota.limit.map(|l| l / 1024 / 1024 / 1024)
    );

    // 刷新结果应已写回存储：重新加载一次确认
    let reloaded = store::load(&path).expect("重新读取失败");
    let Some((again, _)) = reloaded else {
        panic!("刷新后存储里没有会话");
    };
    assert_eq!(again.access_token, new_access, "刷新结果没有落盘");
    println!("刷新结果已写回存储");
}

/// 真实会话的离线任务验证（默认忽略）。
///
/// 前置条件：先在本机用 GUI 登录一次。流程：
/// 列出任务 → 提交一个公开测试磁力 → 轮询到完成（最多 ~2 分钟）→ 删除任务记录。
/// 删除时 `delete_files=false`，云端产物保留。
#[test]
#[ignore = "需要先在 GUI 登录一次（会话在 KWallet 里）"]
fn live_offline_task_roundtrip() {
    let path = store_path();
    let loaded = store::load(&path).expect("读取会话存储失败");
    let Some((saved, backend)) = loaded else {
        panic!("KWallet 里没有会话，请先在 GUI 登录一次再跑这个测试");
    };
    println!("会话来源：{backend:?}");

    let device_id = saved.device_id.clone();
    let session = Session::new(Config::from_env(), path, Some(saved), device_id);

    let before = session.offline_tasks().expect("读取离线任务失败");
    println!("现有离线任务 {} 条", before.len());
    for task in before.iter().take(5) {
        println!("- [{:?}] {} ({}%)", task.phase, task.name, task.progress);
    }

    // Blender 官方的 4.4MB 预告片：稳定、体积小，适合验证链路
    const TEST_URL: &str = "https://download.blender.org/durian/trailer/sintel_trailer-480p.mp4";
    println!("提交测试任务：{TEST_URL}");
    let created = session
        .add_offline_task(TEST_URL, None)
        .expect("提交离线任务失败");
    println!(
        "任务已创建：id={} 名称={:?} 阶段={:?}",
        created.id, created.name, created.phase
    );
    assert!(!created.id.is_empty(), "新建任务没有 id");

    // 轮询到该任务不再变化或超时
    let mut final_phase = None;
    for round in 1..=24 {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let tasks = session.offline_tasks().expect("轮询任务列表失败");
        let Some(task) = tasks.iter().find(|t| t.id == created.id) else {
            println!("第 {round} 轮：任务已不在列表中");
            break;
        };
        println!(
            "第 {round} 轮：[{:?}] {} ({}%) {}",
            task.phase,
            task.name,
            task.progress,
            task.message.as_deref().unwrap_or("")
        );
        if matches!(task.phase, TaskPhase::Completed | TaskPhase::Error) {
            final_phase = Some(task.phase);
            break;
        }
    }
    println!("最终阶段：{final_phase:?}");
    assert!(
        final_phase.is_some(),
        "轮询 2 分钟内任务没有进入完成/失败状态"
    );

    session
        .delete_offline_task(&created.id)
        .expect("删除任务失败");
    println!("任务已删除（云端文件保留）");

    let after = session.offline_tasks().expect("删除后再读取失败");
    assert!(
        !after.iter().any(|t| t.id == created.id),
        "删除后任务仍在列表中"
    );
}

/// 真实会话的目录列表验证（默认忽略）。
///
/// 前置条件：先在本机用 GUI 登录一次。会读取根目录，并在存在子文件夹时
/// 进入第一个文件夹再列一次，验证 parent_id 与分页路径。
#[test]
#[ignore = "需要先在 GUI 登录一次（会话在 KWallet 里）"]
fn live_list_root_with_stored_session() {
    let path = store_path();
    let loaded = store::load(&path).expect("读取会话存储失败");
    let Some((saved, backend)) = loaded else {
        panic!("KWallet 里没有会话，请先在 GUI 登录一次再跑这个测试");
    };
    println!("会话来源：{backend:?}");

    let device_id = saved.device_id.clone();
    let session = Session::new(Config::from_env(), path, Some(saved), device_id);
    let entries = session.list_dir(None).expect("读取根目录失败");
    println!("根目录共 {} 条", entries.len());
    for entry in entries.iter().take(10) {
        println!(
            "- [{}] {}（{} 字节，修改于 {:?}）",
            match entry.kind {
                kichi_core::EntryKind::Folder => "目录",
                kichi_core::EntryKind::File => "文件",
            },
            entry.name,
            entry.size,
            entry.modified_at
        );
    }
    assert!(!entries.is_empty(), "根目录列表为空，实际使用时应有条目");

    if let Some(folder) = entries
        .iter()
        .find(|e| e.kind == kichi_core::EntryKind::Folder)
    {
        let children = session
            .list_dir(Some(folder.id.as_str()))
            .expect("读取子目录失败");
        println!("子目录「{}」共 {} 条", folder.name, children.len());
    } else {
        println!("根目录没有子文件夹，跳过子目录验证");
    }
}

/// 真实会话的播放档位验证（默认忽略）。
///
/// 前置条件：先在本机用 GUI 登录一次。流程：在根目录里找第一个视频/音频
/// 文件 → 取详情 → 断言至少有一个可播档位 → 对选中的直链发一次探测请求
/// （只发 Range 头，下载 1 字节），确认官网真的在服务这个链接。
/// 同时打印 medias 的原始 JSON，核对官方是否下发了字幕字段。
#[test]
#[ignore = "需要先在 GUI 登录一次（会话在 KWallet 里）"]
fn live_play_options_for_a_media_file() {
    let path = store_path();
    let loaded = store::load(&path).expect("读取会话存储失败");
    let Some((saved, backend)) = loaded else {
        panic!("KWallet 里没有会话，请先在 GUI 登录一次再跑这个测试");
    };
    println!("会话来源：{backend:?}");

    let device_id = saved.device_id.clone();
    let session = Session::new(Config::from_env(), path, Some(saved), device_id);

    // 找一个媒体文件：优先根目录，没有再下探一级子目录（最多 3 个）
    let mut found = None;
    let roots = session.list_dir(None).expect("读取根目录失败");
    let mut folders = Vec::new();
    for entry in roots {
        println!(
            "根目录：{} [{}] media={}",
            entry.name,
            if entry.kind == kichi_core::EntryKind::Folder {
                "目录"
            } else {
                "文件"
            },
            entry.media
        );
        if entry.media {
            found = Some(entry.clone());
            break;
        }
        if entry.kind == kichi_core::EntryKind::Folder {
            folders.push(entry);
        }
    }
    if found.is_none() {
        for folder in folders.iter().take(3) {
            let Ok(children) = session.list_dir(Some(folder.id.as_str())) else {
                continue;
            };
            println!("—— 子目录「{}」{} 条 ——", folder.name, children.len());
            for entry in children.iter().take(8) {
                println!("   {} media={}", entry.name, entry.media);
            }
            if let Some(entry) = children.into_iter().find(|entry| entry.media) {
                found = Some(entry);
                break;
            }
        }
    }
    let Some(file) = found else {
        println!("—— 诊断：根目录与子目录都没有 media 标记的条目 ——");
        panic!("没找到媒体文件（video/audio），请先往网盘里放一个视频再跑");
    };
    println!("选中媒体文件：{}（id={}）", file.name, file.id);
    let info =
        media::file_info(&session, &file.id, None, file.audit_flagged).expect("取文件详情失败");
    println!(
        "详情：{} 个 medias，links 全空={}，size={:?}，audit={:?}",
        info.medias.len(),
        info.all_links_empty(),
        info.size,
        info.audit_status
    );
    assert!(!info.all_links_empty(), "官方没有下发任何链接");

    let options = info.play_options();
    for option in &options {
        println!(
            "- {:<8} 原画={:<5} 高度={:?} 可用={} 原因={:?} 主机={}",
            option.label,
            option.original,
            option.height,
            option.available,
            option.reason,
            crate::player::mask_url(&option.url)
        );
    }
    let playable: Vec<_> = options.iter().filter(|o| o.playable()).collect();
    assert!(!playable.is_empty(), "没有可播放的档位，界面上会无从播起");
    assert!(
        playable.iter().any(|o| o.original),
        "连原画档都不可用：{}",
        playable.len()
    );

    // 直链真实可用性：只取 1 字节，不下载整个文件（用系统 curl，不给 kichi 加依赖）
    let target = playable
        .iter()
        .find(|o| o.original)
        .copied()
        .or_else(|| playable.first().copied())
        .expect("上面的断言已保证非空");
    let status = std::process::Command::new("curl")
        .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "-r", "0-0"])
        .arg("--max-time")
        .arg("30")
        .arg(&target.url)
        .output()
        .expect("调用 curl 探测直链失败");
    let code = String::from_utf8_lossy(&status.stdout).trim().to_owned();
    println!(
        "直链探测：HTTP {code}（{}，{}）",
        target.label,
        crate::player::mask_url(&target.url)
    );
    assert!(code == "200" || code == "206", "直链不可用：HTTP {code}");
}

/// 真实会话的容量接口原始响应探测（默认忽略）。
///
/// 目的：核实官方 `about` 到底下发哪些字段——尤其 rclone 里存在的
/// `play_times_limit` / `play_times_usage`（播放时长配额）。确认后才决定
/// 界面上要不要显示；拿不到就只留宽容解析，不臆造数值。
#[test]
#[ignore = "需要先在 GUI 登录一次（会话在 KWallet 里）"]
fn live_about_raw_fields() {
    let path = store_path();
    let loaded = store::load(&path).expect("读取会话存储失败");
    let Some((saved, _)) = loaded else {
        panic!("KWallet 里没有会话，请先在 GUI 登录一次再跑这个测试");
    };
    let device_id = saved.device_id.clone();
    let session = Session::new(Config::from_env(), path, Some(saved), device_id);
    let quota = session.quota().expect("查询容量失败");
    println!(
        "解析结果：usage={} limit={:?} vip={}",
        quota.usage, quota.limit, quota.vip
    );

    // 原始 body：走 kichi-core 的客户端拿一遍，只打印键名与标量值（无凭据）
    let client = session.drive().expect("构造客户端失败");
    match client.about_raw_for_probe() {
        Ok(value) => println!("原始字段：{}", describe_keys(&value)),
        Err(err) => println!("原始请求失败：{err}"),
    }
}

/// 递归列出 JSON 的键路径（只给键名，不给可能是凭据的值）。
fn describe_keys(value: &serde_json::Value) -> String {
    fn walk(value: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    walk(child, &path, out);
                }
            }
            serde_json::Value::Array(items) => {
                for (index, child) in items.iter().take(3).enumerate() {
                    walk(child, &format!("{prefix}[{index}]"), out);
                }
            }
            other => out.push(format!("{prefix}={other}")),
        }
    }
    let mut out = Vec::new();
    walk(value, "", &mut out);
    out.join(", ")
}

/// 排障：列出某目录下前几个视频文件各自的档位构成（默认忽略）。
#[test]
#[ignore = "需要先在 GUI 登录一次（会话在 KWallet 里）"]
fn live_probe_play_options_shapes() {
    let path = store_path();
    let loaded = store::load(&path).expect("读取会话存储失败");
    let Some((saved, _)) = loaded else {
        panic!("KWallet 里没有会话");
    };
    let device_id = saved.device_id.clone();
    let session = Session::new(Config::from_env(), path, Some(saved), device_id);

    let mut checked = 0;
    let mut folders = std::collections::VecDeque::new();
    for entry in session.list_dir(None).expect("读根目录失败") {
        if entry.media {
            report_tiers(&session, &entry);
            checked += 1;
        } else if entry.kind == kichi_core::EntryKind::Folder {
            folders.push_back(entry);
        }
    }
    while checked < 4 {
        let Some(folder) = folders.pop_front() else {
            break;
        };
        let Ok(children) = session.list_dir(Some(folder.id.as_str())) else {
            continue;
        };
        for entry in children {
            if checked >= 4 {
                break;
            }
            if entry.media {
                report_tiers(&session, &entry);
                checked += 1;
            } else if entry.kind == kichi_core::EntryKind::Folder {
                folders.push_back(entry);
            }
        }
    }
    println!("共检查 {checked} 个媒体文件");
}

fn report_tiers(session: &Session, entry: &kichi_core::FileEntry) {
    let Ok(info) = media::file_info(session, &entry.id, None, entry.audit_flagged) else {
        println!("{}: 取详情失败", entry.name);
        return;
    };
    let options = info.play_options();
    let playable = options.iter().filter(|o| o.playable()).count();
    println!("{}", entry.name);
    println!("  档位 {} 个，可用 {playable} 个", options.len());
    for option in &options {
        println!(
            "    {:<8} original={:<5} height={:?} 可用={} 原因={:?}",
            option.label, option.original, option.height, option.available, option.reason
        );
    }
}

/// 真实会话的详情链接字段探针（默认忽略）。
///
/// 目的：核实详情到底下发不下发 `size` / `audit` / `phase`，以及 0 字节
/// 文件是否真的没有任何链接。**只打印字段形状，不打印任何 URL / token。**
///
/// 优先用 `KICHI_TEST_FILE_ID` 指定文件；否则在根目录找第一个
/// `size == 0` 的文件。跑前提醒用户：会发真实请求。
#[test]
#[ignore = "需要先在 GUI 登录一次（会话在 KWallet 里）"]
fn live_probe_detail_link_fields() {
    let path = store_path();
    let loaded = store::load(&path).expect("读取会话存储失败");
    let Some((saved, _)) = loaded else {
        panic!("KWallet 里没有会话，请先在 GUI 登录一次再跑这个测试");
    };
    let device_id = saved.device_id.clone();
    let session = Session::new(Config::from_env(), path, Some(saved), device_id);

    let id = match std::env::var("KICHI_TEST_FILE_ID") {
        Ok(id) if !id.trim().is_empty() => id.trim().to_owned(),
        _ => {
            let entries = session.list_dir(None).expect("读取根目录失败");
            entries
                .into_iter()
                .find(|entry| entry.kind == kichi_core::EntryKind::File && entry.size == 0)
                .map(|entry| entry.id)
                .expect("根目录没有 0 字节文件；请用 KICHI_TEST_FILE_ID 指定一个")
        }
    };

    let client = session.drive().expect("构造客户端失败");
    let raw = client
        .file_detail_raw_for_probe(&id)
        .expect("取文件详情原始响应失败");

    println!(
        "size 原始值：{}",
        raw.get("size").unwrap_or(&serde_json::Value::Null)
    );
    println!(
        "audit.status：{:?}",
        raw.pointer("/audit/status")
            .and_then(|value| value.as_str())
    );
    println!(
        "phase：{:?}",
        raw.get("phase").and_then(|value| value.as_str())
    );

    let link_empty = |value: Option<&serde_json::Value>| match value.and_then(|v| v.as_str()) {
        Some(url) => url.trim().is_empty(),
        None => true,
    };
    println!(
        "octet 链接为空：{}",
        link_empty(raw.pointer("/links/application~1octet-stream/url"))
    );
    println!(
        "web_content_link 为空：{}",
        link_empty(raw.get("web_content_link"))
    );

    let medias = raw
        .get("medias")
        .and_then(|value| value.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let with_url = medias
        .iter()
        .filter(|media| !link_empty(media.pointer("/link/url")))
        .count();
    println!("medias：{} 个，非空链接 {with_url} 个", medias.len());
}

/// 真实 CDN 的 Range 探针（默认忽略）。
///
/// 目的：确认 `Range: bytes=100-` 真的返回 206 + `Content-Range`，而不是
/// 200 全量。若 CDN 忽略 Range，跨重启续传会退化成「从头重下」，本计划
/// 的前提不成立。
///
/// 优先用 `KICHI_TEST_FILE_ID` 指定文件；否则在根目录 / 前几层子目录里找
/// 一个大于 1 MiB 的文件。跑前提醒用户：会发真实下载请求（只读 116 字节）。
#[test]
#[ignore = "需要先在 GUI 登录一次（会话在 KWallet 里）"]
fn live_probe_download_range() {
    let path = store_path();
    let loaded = store::load(&path).expect("读取会话存储失败");
    let Some((saved, _)) = loaded else {
        panic!("KWallet 里没有会话，请先在 GUI 登录一次再跑这个测试");
    };
    let device_id = saved.device_id.clone();
    let session = Session::new(Config::from_env(), path, Some(saved), device_id);

    let mut chosen = None;
    if let Ok(id) = std::env::var("KICHI_TEST_FILE_ID") {
        if !id.trim().is_empty() {
            chosen = Some(id.trim().to_owned());
        }
    }
    if chosen.is_none() {
        let mut folders = std::collections::VecDeque::new();
        folders.push_back(None);
        for _ in 0..4 {
            let Some(parent) = folders.pop_front() else {
                break;
            };
            let Ok(entries) = session.list_dir(parent.as_deref()) else {
                continue;
            };
            for entry in entries {
                match entry.kind {
                    kichi_core::EntryKind::File if entry.size > 1024 * 1024 => {
                        chosen = Some(entry.id.clone());
                    }
                    kichi_core::EntryKind::Folder if folders.len() < 8 => {
                        folders.push_back(Some(entry.id.clone()));
                    }
                    _ => {}
                }
            }
            if chosen.is_some() {
                break;
            }
        }
    }
    let Some(id) = chosen else {
        panic!("没找到大于 1 MiB 的文件；请用 KICHI_TEST_FILE_ID 指定一个");
    };

    let info = media::file_info(&session, &id, None, false).expect("取文件详情失败");
    println!(
        "文件：{}（id={}，详情 size={:?}）",
        info.name, info.id, info.size
    );
    let client = session.drive().expect("构造客户端失败");
    let mut stream = match client.open_download_from(&info, 100) {
        Ok(stream) => stream,
        Err(err) => panic!("从偏移 100 打开下载流失败：{}", probe_error(&err)),
    };
    println!(
        "Range bytes=100- → start_offset={}，total_bytes={:?}",
        stream.start_offset(),
        stream.total_bytes()
    );
    assert_eq!(
        stream.start_offset(),
        100,
        "CDN 忽略了 Range（200 全量）；跨重启续传会退化成从头重下"
    );
    if let (Some(full), Some(detail)) = (stream.total_bytes(), info.size) {
        assert_eq!(full, detail, "Content-Range 的总大小与详情 size 不一致");
    }
    let mut buffer = vec![0u8; 16];
    let read = std::io::Read::read(&mut stream, &mut buffer).expect("读取续传流失败");
    println!("从偏移 100 读到 {read} 字节（内容不打印）");
    assert!(read > 0, "206 响应没有 body");
}

/// 探针：单连接 vs 4 分片的瞬时吞吐对比（只读，不落盘）。
///
/// 为什么要有：ADR 0013 的分片默认值得拿「网速正常时」的数据说话。
/// 2026-10-28 那次采样（1 连接 29.5 / 4 片 27.6 MB/s）正好赶上用户自家网慢，
/// 全网都堵在上限附近，测不出 CDN 有没有单连接限速。
///
/// 用法（先在本机 GUI 登录过一次）：
///   KICHI_TEST_FILE_ID=<≥32 MiB 的文件 id> \
///     cargo test -p kichi --bin kichi -- \
///     --ignored live_probe_download_throughput --nocapture
///
/// `KICHI_TEST_SECONDS` 控制每段采样时长（默认 15 秒）。
/// 会真下载「时长 × 吞吐 × 2 段」的流量，跑前提醒用户。
#[test]
#[ignore = "需要先在本机 GUI 登录一次；会真下载几十到几百 MB"]
fn live_probe_download_throughput() {
    use std::time::Duration;

    let path = store_path();
    let loaded = store::load(&path).expect("读取会话存储失败");
    let Some((saved, _)) = loaded else {
        panic!("KWallet 里没有会话，请先在 GUI 登录一次再跑这个测试");
    };
    let device_id = saved.device_id.clone();
    let session = Session::new(Config::from_env(), path, Some(saved), device_id);

    let seconds: u64 = std::env::var("KICHI_TEST_SECONDS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(15);
    let span = Duration::from_secs(seconds);

    let id = match std::env::var("KICHI_TEST_FILE_ID") {
        Ok(id) if !id.trim().is_empty() => id.trim().to_owned(),
        _ => find_big_file(&session, 32 * 1024 * 1024)
            .expect("没找到 ≥ 32 MiB 的文件；请用 KICHI_TEST_FILE_ID 指定一个"),
    };
    let info = media::file_info(&session, &id, None, false).expect("取文件详情失败");
    let Some(size) = info.size else {
        panic!("详情没给大小，没法按区间采样；换一个文件或用 KICHI_TEST_FILE_ID");
    };
    println!(
        "文件：{}（{:.1} MB，每段采样 {seconds} 秒）",
        info.name,
        size as f64 / 1e6
    );

    // both（默认）/ single / shards / serial：
    // 分两段跑一遍要下两倍流量，排查时只跑一段；
    // serial = 同样 4 个区间但**依次**读，用来分清「CDN 限并发」与「真的没收益」。
    let mode = std::env::var("KICHI_TEST_MODE").unwrap_or_else(|_| "both".to_owned());
    let client = session.drive().expect("构造客户端失败");

    let mut single = 0.0;
    if mode == "both" || mode == "single" {
        single = sample_single(client.as_ref(), &info, span);
        println!("1 连接：{single:.1} MB/s");
    }

    let mut sharded = 0.0;
    if mode == "both" || mode == "shards" {
        sharded = sample_shards(&client, &info, size, 4, span);
        println!("4 分片（并发）：{sharded:.1} MB/s");
    }

    if mode == "serial" {
        let serial = sample_shards_serial(client.as_ref(), &info, size, 4, span);
        println!("4 分片（依次）：{serial:.1} MB/s");
    }

    if mode == "open" {
        // 开放式 Range（bytes=N-）：与真正的分片一样带 Range 头，但不设结束位置
        let middle = size / 2;
        let label = "开放式 Range 从中间";
        let Some(mut stream) = open_with_retry(|| client.open_download_from(&info, middle), label)
        else {
            return;
        };
        let bytes = drain(&mut stream, label, span);
        println!(
            "{label}：{:.1} MB/s",
            bytes as f64 / 1e6 / span.as_secs_f64()
        );
    }

    if single > 0.0 && sharded > 0.0 {
        println!("分片收益：{:.2}×", sharded / single);
    }
}

/// 同样按区间切成 4 段，但一条读完再读下一条：总字节 ÷ (4 × 每段时长)。
fn sample_shards_serial(
    client: &kichi_core::DriveClient,
    info: &kichi_core::FileInfo,
    size: u64,
    shards: u64,
    span: std::time::Duration,
) -> f64 {
    let step = size / shards;
    let mut total = 0u64;
    for index in 0..shards {
        let start = index * step;
        let end = if index + 1 == shards {
            size
        } else {
            start + step
        };
        let label = format!("第 {} 片（依次）", index + 1);
        let Some(mut stream) = open_with_retry(
            || client.open_download_range(info, start, end.saturating_sub(1)),
            &label,
        ) else {
            continue;
        };
        let bytes = drain(&mut stream, &label, span);
        println!("{label}：{:.1} MB", bytes as f64 / 1e6);
        total += bytes;
    }
    total as f64 / 1e6 / (span.as_secs_f64() * shards as f64)
}

/// 从根目录往下找第一个 ≥ `min_bytes` 的文件（只列目录，不下载）。
fn find_big_file(session: &Session, min_bytes: u64) -> Option<String> {
    let mut folders = std::collections::VecDeque::new();
    folders.push_back(None);
    for _ in 0..4 {
        let parent = folders.pop_front()?;
        let Ok(entries) = session.list_dir(parent.as_deref()) else {
            continue;
        };
        for entry in entries {
            match entry.kind {
                kichi_core::EntryKind::File if entry.size >= min_bytes => {
                    return Some(entry.id.clone())
                }
                kichi_core::EntryKind::Folder if folders.len() < 8 => {
                    folders.push_back(Some(entry.id.clone()))
                }
                _ => {}
            }
        }
    }
    None
}

/// 单连接读 `span` 秒，返回 MB/s。打开失败只记 0（不打断另一段采样）。
fn sample_single(
    client: &kichi_core::DriveClient,
    info: &kichi_core::FileInfo,
    span: std::time::Duration,
) -> f64 {
    let Some(mut stream) = open_with_retry(|| client.open_download_from(info, 0), "单连接")
    else {
        return 0.0;
    };
    drain(&mut stream, "单连接", span) as f64 / 1e6 / span.as_secs_f64()
}

/// 打开下载流：失败等 1 秒重试一次（真机偶发连接被拒，一次就放弃会误判）。
fn open_with_retry<F>(mut open: F, label: &str) -> Option<kichi_core::DownloadStream>
where
    F: FnMut() -> Result<kichi_core::DownloadStream, kichi_core::Error>,
{
    match open() {
        Ok(stream) => return Some(stream),
        Err(err) => println!("{label}打开失败（{}），1 秒后重试一次", probe_error(&err)),
    }
    std::thread::sleep(std::time::Duration::from_secs(1));
    match open() {
        Ok(stream) => Some(stream),
        Err(err) => {
            println!("{label}重试仍失败：{}", probe_error(&err));
            None
        }
    }
}

/// 同时开 `shards` 条区间连接读 `span` 秒，返回合计 MB/s。
fn sample_shards(
    client: &std::sync::Arc<kichi_core::DriveClient>,
    info: &kichi_core::FileInfo,
    size: u64,
    shards: u64,
    span: std::time::Duration,
) -> f64 {
    let step = size / shards;
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..shards)
            .map(|index| {
                let client = std::sync::Arc::clone(client);
                let info = info.clone();
                let start = index * step;
                let end = if index + 1 == shards {
                    size
                } else {
                    start + step
                };
                scope.spawn(move || {
                    let label = format!("第 {} 片", index + 1);
                    let Some(mut stream) = open_with_retry(
                        || client.open_download_range(&info, start, end.saturating_sub(1)),
                        &label,
                    ) else {
                        return 0.0;
                    };
                    drain(&mut stream, &label, span) as f64
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or(0.0))
            .sum::<f64>()
    }) / 1e6
        / span.as_secs_f64()
}

/// 一直读到 `span` 用尽或流结束/出错；返回读到的字节数（出错原因打到 stdout）。
fn drain(stream: &mut kichi_core::DownloadStream, label: &str, span: std::time::Duration) -> u64 {
    let started = std::time::Instant::now();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    while started.elapsed() < span {
        match std::io::Read::read(stream, &mut buffer) {
            Ok(0) => {
                println!("{label}：读到文件末尾");
                break;
            }
            Ok(read) => total += read as u64,
            Err(err) => {
                // 只报错误类别：io::Error 的文案里可能带签名 URL
                println!(
                    "{label}：{:.1} 秒读出错（已收 {:.1} MB，{:?}）",
                    started.elapsed().as_secs_f64(),
                    total as f64 / 1e6,
                    err.kind()
                );
                break;
            }
        }
    }
    total
}

/// 探针打印错误时只报类别 / 状态码，绝不带签名 URL。
fn probe_error(err: &kichi_core::Error) -> String {
    match err {
        kichi_core::Error::Api { code, .. } => format!("HTTP {code}"),
        kichi_core::Error::Network(_) => "网络错误".to_owned(),
        kichi_core::Error::DownloadTimeout(_) => "下载超时".to_owned(),
        kichi_core::Error::RangeNotSatisfiable { total } => {
            format!("Range 不可用（total={total:?}）")
        }
        kichi_core::Error::Auth { .. } => "认证失败".to_owned(),
        kichi_core::Error::RateLimited { .. } => "限流 / 配额".to_owned(),
        other => other.to_string(),
    }
}
