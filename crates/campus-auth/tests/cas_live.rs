//! CAS 全流程集成冒烟（live）——#[ignore]：涉及真实凭据与校园网。
//!
//! 仅主智能体验收时运行：
//! `CAMPUS_HUB_CREDS=<凭据文件> CAMPUS_HUB_CAPTCHA_ANSWER=<答案> cargo test -p campus-auth -- --ignored cas_live`
//!
//! 凭据文件格式（逐行，冒号支持全/半角）：
//! ```text
//! 账号:xxx
//! 密码:yyy
//! ```
//! 纪律：真实账号/密码绝不写进代码常量或任何日志/断言消息。

use campus_auth::cas::{CasClient, SessionState, PORTAL_SERVICE};
use campus_auth::rsa::rsa_encrypt_hex;

/// 从 CAMPUS_HUB_CREDS 指向的凭据文件读取 (账号, 密码)。解析写在测试内（计划 Task 7 Step 3）。
fn read_creds() -> (String, String) {
    let path = std::env::var("CAMPUS_HUB_CREDS")
        .expect("缺少 CAMPUS_HUB_CREDS 环境变量（指向凭据文件）");
    let text = std::fs::read_to_string(&path).expect("读取凭据文件失败");
    let mut user: Option<String> = None;
    let mut pass: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line
            .strip_prefix("账号:")
            .or_else(|| line.strip_prefix("账号："))
        {
            user = Some(v.trim().to_string());
        }
        if let Some(v) = line
            .strip_prefix("密码:")
            .or_else(|| line.strip_prefix("密码："))
        {
            pass = Some(v.trim().to_string());
        }
    }
    (
        user.expect("凭据文件缺少「账号:」行"),
        pass.expect("凭据文件缺少「密码:」行"),
    )
}

/// kaptcha → 手动占位答案（Task 8 完成后换 captcha::solve 自动识别）→ login →
/// sso_follow(PORTAL_SERVICE) → 断言 portal_probe()==Alive 且 jar has("customsid")。
#[tokio::test]
#[ignore = "live：需校园网与真实凭据，仅主智能体验收时 -- --ignored 运行"]
async fn cas_live_login_flow() {
    let (username, password) = read_creds();
    let answer = std::env::var("CAMPUS_HUB_CAPTCHA_ANSWER")
        .expect("缺少 CAMPUS_HUB_CAPTCHA_ANSWER（看图手动输入的验证码答案）");

    let client = CasClient::new().expect("创建 CasClient 失败");

    let captcha = client.kaptcha().await.expect("获取验证码失败");
    assert!(!captcha.uid.is_empty(), "kaptcha 应返回 uid");
    assert!(!captcha.png_base64.is_empty(), "kaptcha 应返回非空 base64 PNG");

    let password_rsa = rsa_encrypt_hex(&password).expect("RSA 加密密码失败");
    let ok = client
        .login(&username, &password_rsa, &captcha.uid, &answer)
        .await
        .expect("CAS 登录应成功（Err 多为验证码答案过期，重跑换新答案）");
    assert!(ok.ticket.starts_with("ST-"), "ticket 应为 ST- 前缀");
    assert!(ok.tgt.starts_with("TGT-"), "tgt 应为 TGT- 前缀");

    client
        .sso_follow(PORTAL_SERVICE, &ok.ticket)
        .await
        .expect("SSO 回跳门户失败");

    assert!(client.jar().has("customsid"), "门户会话应种下 customsid");
    assert_eq!(client.portal_probe().await, SessionState::Alive);
}
