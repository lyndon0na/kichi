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

use kichi_core::{auth, captcha, Config, DriveClient};

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
/// 结论写回 `docs/api.md`。
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
