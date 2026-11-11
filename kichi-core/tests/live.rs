//! 真实账号集成测试（默认 `#[ignore]`，需要环境变量）。
//!
//! 运行方式：
//! ```text
//! KICHI_TEST_USER=账号 KICHI_TEST_PASS=密码 cargo test -p kichi-core --test live -- --ignored --nocapture
//! ```
//! 首次运行常会触发人机验证：把官方给出的验证 URL 在浏览器里完成，
//! 再把页面上的 captcha token 放进 `KICHI_TEST_CAPTCHA_TOKEN` 重跑。
//! 测试输出只打印非敏感信息（不打印任何令牌）。

use std::env;

use kichi_core::{auth, captcha, Config, DriveClient, EntryKind, RestoreShareRequest, TaskPhase};

fn test_config() -> Option<(Config, String, String)> {
    let user = env::var("KICHI_TEST_USER").ok()?.trim().to_owned();
    let pass = env::var("KICHI_TEST_PASS").ok()?.to_owned();
    Some((Config::from_env(), user, pass))
}

/// 登录一次并建好网盘客户端（探针共用）。失败时直接 panic，与下面的用例一致。
fn signed_in_client() -> DriveClient {
    let Some((config, user, pass)) = test_config() else {
        panic!("未设置 KICHI_TEST_USER / KICHI_TEST_PASS");
    };
    let device_id = captcha::generate_device_id();
    let manual = env::var("KICHI_TEST_CAPTCHA_TOKEN").ok();
    let outcome = auth::sign_in(&config, &device_id, &user, &pass, manual.as_deref());
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(kichi_core::Error::Captcha { url, .. }) => {
            panic!("需要人机验证，完成后用 KICHI_TEST_CAPTCHA_TOKEN 重跑。验证地址：{url:?}")
        }
        Err(err) => panic!("登录失败：{err}"),
    };
    DriveClient::new(
        &config,
        &device_id,
        &outcome.tokens.access_token,
        &outcome.user_id,
    )
    .expect("创建客户端失败")
}

#[test]
#[ignore = "需要 KICHI_TEST_USER / KICHI_TEST_PASS"]
fn real_login_refresh_and_about() {
    let Some((config, user, pass)) = test_config() else {
        panic!("未设置 KICHI_TEST_USER / KICHI_TEST_PASS");
    };
    let device_id = captcha::generate_device_id();
    let manual = env::var("KICHI_TEST_CAPTCHA_TOKEN").ok();

    let outcome = auth::sign_in(&config, &device_id, &user, &pass, manual.as_deref());
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(kichi_core::Error::Captcha { url, .. }) => {
            panic!("需要人机验证，完成后用 KICHI_TEST_CAPTCHA_TOKEN 重跑。验证地址：{url:?}")
        }
        Err(err) => panic!("登录失败：{err}"),
    };

    assert!(!outcome.tokens.access_token.is_empty(), "access token 为空");
    assert!(!outcome.user_id.is_empty(), "未能解析出用户 id");
    println!("登录成功，令牌有效期至 {}", outcome.tokens.expires_at);

    let refreshed =
        auth::refresh(&config, &device_id, &outcome.tokens.refresh_token).expect("刷新令牌失败");
    assert!(!refreshed.access_token.is_empty());
    println!("刷新成功，新令牌有效期至 {}", refreshed.expires_at);

    let client = DriveClient::new(
        &config,
        &device_id,
        &refreshed.access_token,
        &outcome.user_id,
    )
    .expect("创建客户端失败");

    let quota = client.about().expect("获取容量失败");
    println!(
        "容量：已用 {} GB / 上限 {:?}，VIP={}",
        quota.usage / 1024 / 1024 / 1024,
        quota.limit.map(|l| l / 1024 / 1024 / 1024),
        quota.vip
    );

    let me = client.me().expect("获取用户信息失败");
    assert!(!me.id.is_empty() || !me.name.is_empty(), "用户信息为空");
    println!("用户：{}（{}）", me.name, me.email);
}

/// 移动 / 复制探针的收尾：无论用例怎么退出，都把建出来的临时目录移入回收站。
///
/// 记录的都是本次自己建的 id（源目录、目标目录、子目录、复制品），
/// 不会碰用户已有的条目。
struct ProbeCleanup<'a> {
    client: &'a DriveClient,
    ids: Vec<String>,
}

impl Drop for ProbeCleanup<'_> {
    fn drop(&mut self) {
        if self.ids.is_empty() {
            return;
        }
        let refs: Vec<&str> = self.ids.iter().map(String::as_str).collect();
        match self.client.trash(&refs) {
            Ok(()) => println!("收尾：临时目录已移入回收站（{} 个）", refs.len()),
            Err(err) => println!(
                "收尾失败（{}）：临时目录还在云端，名字以 kichi-probe- 开头，请手动删除",
                err
            ),
        }
    }
}

/// 在前几页里按名字找条目（刚建好的目录偶尔要等一下才出现在列表里）。
fn find_id(client: &DriveClient, parent: Option<&str>, name: &str) -> Option<String> {
    let mut token: Option<String> = None;
    for _page in 0..5 {
        let (entries, next) = client.list(parent, token.as_deref()).ok()?;
        if let Some(found) = entries.into_iter().find(|entry| entry.name == name) {
            return Some(found.id);
        }
        token = Some(next?);
    }
    None
}

/// 等列表跟上（探针里新建目录后立刻查会偶尔查不到）。
fn wait_for_id(client: &DriveClient, parent: Option<&str>, name: &str) -> Option<String> {
    for attempt in 0..5 {
        if let Some(id) = find_id(client, parent, name) {
            return Some(id);
        }
        if attempt < 4 {
            std::thread::sleep(std::time::Duration::from_millis(600));
        }
    }
    None
}

/// 真实端点探针：`batchMove` / `batchCopy` 的请求形状与错误码。
///
/// ⚠ 会真实改动云端数据（建三个临时目录 + 一次复制），跑之前先提醒用户。
#[test]
#[ignore = "会真实改动云端数据，跑前提醒用户（需要 KICHI_TEST_USER / KICHI_TEST_PASS）"]
fn real_move_copy_probe() {
    let client = signed_in_client();

    // 名字带秒级时间戳：重复跑不会和上一次的残留撞名
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("系统时间早于 1970")
        .as_secs();
    let src_name = format!("kichi-probe-src-{stamp}");
    let dst_name = format!("kichi-probe-dst-{stamp}");
    let child_name = format!("kichi-probe-child-{stamp}");

    let mut cleanup = ProbeCleanup {
        client: &client,
        ids: Vec::new(),
    };

    client.create_folder(None, &src_name).expect("建源目录失败");
    let src_id = wait_for_id(&client, None, &src_name).expect("列表里找不到刚建的源目录");
    cleanup.ids.push(src_id.clone());

    client
        .create_folder(None, &dst_name)
        .expect("建目标目录失败");
    let dst_id = wait_for_id(&client, None, &dst_name).expect("列表里找不到刚建的目标目录");
    cleanup.ids.push(dst_id.clone());

    client
        .create_folder(Some(&src_id), &child_name)
        .expect("在源目录下建子目录失败");
    let child_id =
        wait_for_id(&client, Some(&src_id), &child_name).expect("列表里找不到刚建的子目录");
    cleanup.ids.push(child_id.clone());
    println!("临时目录：src={src_id} dst={dst_id} child={child_id}");

    // 1) 同目录移动：期望官方回 file_move_or_copy_to_cur（code 9）
    match client.move_to(&[&child_id], &src_id) {
        Ok(()) => println!("⚠ 同目录移动居然成功了（官方不再挡）"),
        Err(err) => println!("同目录移动被拒（符合预期）：{err}"),
    }

    // 2) 移到目标目录
    match client.move_to(&[&child_id], &dst_id) {
        Ok(()) => {
            let moved = wait_for_id(&client, Some(&dst_id), &child_name);
            println!(
                "移动到目标目录：HTTP 2xx；目标目录里{}",
                if moved.as_deref() == Some(child_id.as_str()) {
                    "已能看到它"
                } else {
                    "暂时没看到（服务端列表可能滞后）"
                }
            );
        }
        Err(err) => println!("移动到目标目录失败：{err:?}"),
    }

    // 3) 移到根目录
    match client.move_to(&[&child_id], "") {
        Ok(()) => {
            let moved = wait_for_id(&client, None, &child_name);
            println!(
                "移动到根目录：HTTP 2xx；根目录里{}",
                if moved.as_deref() == Some(child_id.as_str()) {
                    "已能看到它"
                } else {
                    "暂时没看到（服务端列表可能滞后）"
                }
            );
        }
        Err(err) => println!("移动到根目录失败：{err:?}"),
    }

    // 4) 从根目录复制到目标目录（失败不 panic：VIP 限制等要看真实 code）
    match client.copy_to(&[&child_id], &dst_id) {
        Ok(()) => {
            let copied = wait_for_id(&client, Some(&dst_id), &child_name);
            match copied {
                Some(id) if id != child_id => {
                    println!("复制成功，目标目录里的新条目 id={id}（与源不同，符合预期）");
                    cleanup.ids.push(id);
                }
                Some(_) => println!("⚠ 复制成功但目标目录里还是源条目本身"),
                None => println!("复制返回 2xx，但目标目录里暂时没看到副本（可能被自动改名）"),
            }
            let original = find_id(&client, None, &child_name);
            println!(
                "复制后源条目仍在根目录：{}",
                if original.as_deref() == Some(child_id.as_str()) {
                    "是"
                } else {
                    "否（⚠ 需要人工确认）"
                }
            );
        }
        Err(err) => println!("复制失败（完整错误，未做任何映射）：{err:?}"),
    }
}

/// 打开别人分享的只读探针：链接解析、`/share` 元数据、顶层与一层子目录。
///
/// 需要 `KICHI_TEST_SHARE_URL`（分享链接或整段分享文本）；
/// 文本里没有密码时可用 `KICHI_TEST_SHARE_PASS` 单独给。
/// 只读，不改动云端数据；输出不含令牌。
#[test]
#[ignore = "需要 KICHI_TEST_USER / KICHI_TEST_PASS / KICHI_TEST_SHARE_URL"]
fn real_share_open_probe() {
    let Some(raw) = env::var("KICHI_TEST_SHARE_URL").ok() else {
        panic!("未设置 KICHI_TEST_SHARE_URL（分享链接或整段分享文本）");
    };
    let input = kichi_core::parse_share_input(&raw).expect("分享链接解析失败");
    println!(
        "解析成功：share_id 长度 {}，文本内密码={}",
        input.share_id.len(),
        input.pass_code.is_some()
    );
    let pass = env::var("KICHI_TEST_SHARE_PASS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| input.pass_code.clone());

    let client = signed_in_client();
    let info = match client.open_share(&input.share_id, pass.as_deref()) {
        Ok(info) => info,
        Err(kichi_core::Error::ShareUnavailable { status, message }) => {
            println!("分享状态 {status}：{message}（预期内的「打不开」，说明请求形状已通）");
            return;
        }
        Err(err) => panic!("打开分享失败：{err}"),
    };
    println!(
        "已打开：标题={:?} 分享者={:?} 条目数={} 永久={} 到期={:?}",
        info.title, info.owner, info.file_count, info.never_expires, info.expiration_at
    );

    let mut entries = info.entries.clone();
    let mut token = info.next_page_token.clone();
    let mut pages = 1;
    while let Some(page_token) = token {
        if pages >= 20 {
            println!("⚠ 顶层分页到 20 页仍未结束，停止");
            break;
        }
        let page = client
            .list_share_files(
                &info.share_id,
                &info.pass_code_token,
                None,
                Some(&page_token),
            )
            .expect("顶层分页失败");
        entries.extend(page.entries);
        token = page.next_page_token;
        pages += 1;
    }
    println!("顶层条目 {} 个（{pages} 页）", entries.len());

    if let Some(folder) = entries.iter().find(|entry| entry.kind == EntryKind::Folder) {
        match client.list_share_files(
            &info.share_id,
            &info.pass_code_token,
            Some(&folder.id),
            None,
        ) {
            Ok(page) => println!(
                "子目录「{}」第一页 {} 个条目，还有下一页={}",
                folder.name,
                page.entries.len(),
                page.next_page_token.is_some()
            ),
            Err(err) => println!("读取子目录失败：{err}"),
        }
    }
}

/// 转存探针：会真实改动云端数据（建临时目录 + 转存 + 回收站）。
///
/// 必须同时设置 `KICHI_TEST_SHARE_RESTORE=1` 才会执行，避免误跑。
/// 只转存分享顶层的第一项，目标是自己根目录下新建的 `kichi-probe-share-*`。
#[test]
#[ignore = "会真实改动云端数据；设置 KICHI_TEST_SHARE_RESTORE=1 才会执行"]
fn real_share_restore_probe() {
    if env::var("KICHI_TEST_SHARE_RESTORE").ok().as_deref() != Some("1") {
        println!("跳过：设置 KICHI_TEST_SHARE_RESTORE=1 才会真的转存");
        return;
    }
    let Some(raw) = env::var("KICHI_TEST_SHARE_URL").ok() else {
        panic!("未设置 KICHI_TEST_SHARE_URL（分享链接或整段分享文本）");
    };
    let input = kichi_core::parse_share_input(&raw).expect("分享链接解析失败");
    let pass = env::var("KICHI_TEST_SHARE_PASS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| input.pass_code.clone());
    let client = signed_in_client();
    let info = client
        .open_share(&input.share_id, pass.as_deref())
        .expect("打开分享失败");
    let Some(target) = info.entries.first() else {
        println!("分享顶层没有条目，无法转存");
        return;
    };

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("系统时间早于 1970")
        .as_secs();
    let dir_name = format!("kichi-probe-share-{stamp}");
    client
        .create_folder(None, &dir_name)
        .expect("创建临时目标目录失败");
    let dest = wait_for_id(&client, None, &dir_name).expect("找不到刚建的临时目标目录");
    let mut cleanup = ProbeCleanup {
        client: &client,
        ids: vec![dest.clone()],
    };

    let request = RestoreShareRequest {
        share_id: info.share_id.clone(),
        pass_code_token: info.pass_code_token.clone(),
        file_ids: vec![target.id.clone()],
        parent_id: dest.clone(),
        // 顶层条目的祖先链为空；子目录里的条目要带上文件夹 id。
        ancestor_ids: Vec::new(),
    };
    let result = match client.restore_share(&request) {
        Ok(result) => result,
        Err(err) => {
            println!("转存请求失败（完整错误，未做映射）：{err:?}");
            return;
        }
    };
    println!(
        "转存响应：status={} 有任务 id={}",
        result.status,
        result.task_id.is_some()
    );
    if result.is_complete() {
        println!("转存同步完成");
    } else if let Some(task_id) = result.task_id {
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let state = match client.share_restore_task(&task_id) {
                Ok(state) => state,
                Err(err) => {
                    println!("查询转存任务失败：{err}");
                    break;
                }
            };
            if state.phase == TaskPhase::Completed {
                println!("转存任务完成");
                break;
            }
            if state.phase == TaskPhase::Error {
                println!("转存任务失败：{:?}", state.message);
                break;
            }
        }
    }

    match find_id(&client, Some(&dest), &target.name) {
        Some(id) => {
            println!("目标目录里已出现「{}」，id 长度 {}", target.name, id.len());
            cleanup.ids.push(id);
        }
        None => println!("目标目录里暂时没看到「{}」，稍后到文件页确认", target.name),
    }
}
