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
