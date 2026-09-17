//! CAS 全流程集成冒烟（live）——#[ignore]：涉及真实凭据与校园网。
//!
//! 仅主智能体验收时运行：
//! `CAMPUS_HUB_CREDS=<凭据文件> cargo test -p campus-auth -- --ignored cas_live`（验证码自动识别）
//!
//! 凭据文件格式（逐行，冒号支持全/半角）：
//! ```text
//! 账号:xxx
//! 密码:yyy
//! ```
//! 纪律：真实账号/密码绝不写进代码常量或任何日志/断言消息。

use campus_auth::captcha::{solve, KaptchaTemplates};
use campus_auth::cas::{CasClient, SessionState, PORTAL_SERVICE};
use campus_auth::rsa::rsa_encrypt_hex;

/// 从 CAMPUS_HUB_CREDS 指向的凭据文件读取 (账号, 密码)。解析写在测试内（计划 Task 7 Step 3）。
/// 兼容两种格式：逐行「账号:xxx」/「密码:yyy」，或单行「账号xxx，密码yyy」（probe.js 实测格式）。
fn read_creds() -> (String, String) {
    let path = std::env::var("CAMPUS_HUB_CREDS")
        .expect("缺少 CAMPUS_HUB_CREDS 环境变量（指向凭据文件）");
    let text = std::fs::read_to_string(&path).expect("读取凭据文件失败");

    /// 取 key 之后、首个分隔符（，,/空白/。）之前的值；全/半角冒号均跳过。
    fn field(line: &str, key: &str) -> Option<String> {
        let i = line.find(key)?;
        let rest = line[i + key.len()..]
            .trim_start_matches([':', '：'])
            .trim_start();
        let v: String = rest
            .chars()
            .take_while(|c| !matches!(c, '，' | ',' | '。' | ' ' | '\t' | '\r'))
            .collect();
        (!v.trim().is_empty()).then(|| v.trim().to_string())
    }

    let (mut user, mut pass) = (None, None);
    for line in text.lines() {
        if user.is_none() {
            user = field(line, "账号");
        }
        if pass.is_none() {
            pass = field(line, "密码");
        }
    }
    (
        user.expect("凭据文件缺少账号（支持「账号:xxx」或「账号xxx，密码yyy」）"),
        pass.expect("凭据文件缺少密码"),
    )
}

/// kaptcha → `captcha::solve` 自动识别（识别失败自动换一张，≤3 张）→ login →
/// sso_follow(PORTAL_SERVICE) → 断言 portal_probe()==Alive 且 jar has("customsid")。
#[tokio::test]
#[ignore = "live：需校园网与真实凭据，仅主智能体验收时 -- --ignored 运行"]
async fn cas_live_login_flow() {
    let (username, password) = read_creds();
    let templates = KaptchaTemplates::load();

    let client = CasClient::new().expect("创建 CasClient 失败");
    let password_rsa = rsa_encrypt_hex(&password).expect("RSA 加密密码失败");

    // 自动识别验证码（识别不出→换一张重来，不提交 CAS，不消耗连续错误计数）
    let mut ok = None;
    for attempt in 1..=3 {
        let captcha = client.kaptcha().await.expect("获取验证码失败");
        assert!(!captcha.uid.is_empty(), "kaptcha 应返回 uid");
        assert!(!captcha.png_base64.is_empty(), "kaptcha 应返回非空 base64 PNG");
        let png = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &captcha.png_base64,
        )
        .expect("验证码 base64 解码失败");
        let Some(answer) = solve(&png, &templates) else {
            println!("[第 {attempt} 张] 识别失败，换一张");
            continue;
        };
        let r = client
            .login(&username, &password_rsa, &captcha.uid, &answer)
            .await
            .expect("CAS 登录请求失败（网络层）");
        ok = Some(r);
        break;
    }
    let ok = ok.expect("3 张验证码均未能识别（或登录均失败）");
    assert!(ok.ticket.starts_with("ST-"), "ticket 应为 ST- 前缀");
    assert!(ok.tgt.starts_with("TGT-"), "tgt 应为 TGT- 前缀");

    client
        .sso_follow(PORTAL_SERVICE, &ok.ticket)
        .await
        .expect("SSO 回跳门户失败");

    assert!(client.jar().has("customsid"), "门户会话应种下 customsid");

    // 会话 cookie 名单（只打名字，绝不打值）：门户会话应为 customsid/rememberMe/Authorization
    let names: Vec<String> = client
        .jar()
        .snapshot()
        .iter()
        .map(|(n, _)| n.clone())
        .collect();
    println!("[会话] jar cookie 名: {names:?}");
    assert!(names.iter().any(|n| n == "customsid"));

    assert_eq!(client.portal_probe().await, SessionState::Alive);
}
