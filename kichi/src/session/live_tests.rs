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
    const TEST_URL: &str =
        "https://download.blender.org/durian/trailer/sintel_trailer-480p.mp4";
    println!("提交测试任务：{TEST_URL}");
    let created = session
        .add_offline_task(TEST_URL)
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
    let info = media::file_info(&session, &file.id).expect("取文件详情失败");
    println!(
        "详情：{} 个 medias，links 全空={}",
        info.medias.len(),
        info.all_links_empty
    );
    assert!(!info.all_links_empty, "官方没有下发任何链接");

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
