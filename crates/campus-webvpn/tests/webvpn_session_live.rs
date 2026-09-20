//! WebVPN 会话全链路 live 探针——#[ignore]：涉及真实凭据与校园网。
//!
//! 仅主智能体验收时运行：
//! `CAMPUS_HUB_CREDS=<凭据文件> cargo test -p campus-webvpn -- --ignored --nocapture webvpn_session`
//!
//! 流程：CAS 登录（kaptcha 自动识别）拿 TGT → `WebVpnSession::login` 静默登 WebVPN
//! → `is_alive` 断言 → 用 `wrap_url` 包装 `http://10.3.100.110/charge/feeitem` 经
//! `wrapped_client` 请求，打印 HTTP 状态与响应前 200 字符——**这是验证「网关是否
//! 代理 10.3.100.110」的关键探针**。
//!
//! 纪律：真实账号/密码绝不写进代码常量或任何日志/断言消息；不打印 cookie / token /
//! 请求响应头。

use campus_auth::captcha::{solve, KaptchaTemplates};
use campus_auth::cas::CasClient;
use campus_auth::rsa::rsa_encrypt_hex;
use campus_webvpn::{wrap_url, GATEWAY, WebVpnSession};

/// 从 CAMPUS_HUB_CREDS 指向的凭据文件读取 (账号, 密码)（cas_live.rs 同款解析，
/// 兼容逐行「账号:xxx」与单行「账号xxx，密码yyy」两种格式）。
fn read_creds() -> (String, String) {
    let path = std::env::var("CAMPUS_HUB_CREDS")
        .expect("缺少 CAMPUS_HUB_CREDS 环境变量（指向凭据文件）");
    let text = std::fs::read_to_string(&path).expect("读取凭据文件失败");

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

/// CAS 登录拿 TGT（kaptcha → solve 自动识别，识别失败换一张，≤3 张）。
/// 返回 (client, tgt)：后续 WebVPN 换票复用同一 client（其 jar 与 CAS 会话无耦合，
/// 但复用避免重复建连）。
async fn cas_login_for_tgt() -> (CasClient, String) {
    let (username, password) = read_creds();
    let cas = CasClient::new().expect("创建 CasClient 失败");
    let password_rsa = rsa_encrypt_hex(&password).expect("RSA 加密密码失败");
    let templates = KaptchaTemplates::load();
    let mut ok = None;
    for attempt in 1..=3 {
        let captcha = cas.kaptcha().await.expect("获取验证码失败");
        let png = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &captcha.png_base64,
        )
        .expect("验证码 base64 解码失败");
        let Some(answer) = solve(&png, &templates) else {
            println!("[CAS 第 {attempt} 张] 识别失败，换一张");
            continue;
        };
        let r = cas
            .login(&username, &password_rsa, &captcha.uid, &answer)
            .await
            .expect("CAS 登录请求失败（网络层）");
        ok = Some(r);
        break;
    }
    let ok = ok.expect("3 张验证码均未能识别（或登录均失败）");
    assert!(ok.tgt.starts_with("TGT-"), "tgt 应为 TGT- 前缀");
    (cas, ok.tgt)
}

#[tokio::test]
#[ignore = "live：需校园网与真实凭据，仅主智能体验收时 -- --ignored 运行"]
async fn webvpn_session_live_probe() {
    let (cas, tgt) = cas_login_for_tgt().await;
    println!("[1] CAS TGT 获取成功（前 12 字符={}…）", &tgt[..12.min(tgt.len())]);

    // TGT 静默登 WebVPN（换票 → ticket 回跳 302 链）
    let session = WebVpnSession::login(&cas, &tgt)
        .await
        .expect("WebVPN 登录失败（换票或 302 链）");
    let cookie_names: Vec<String> = session
        .jar()
        .snapshot()
        .iter()
        .map(|(n, _)| n.clone())
        .collect();
    println!("[2] WebVPN 登录链走完，jar cookie 名: {cookie_names:?}");

    // 探活：GET 网关 / 不落 /login
    assert!(session.is_alive().await, "WebVPN 会话应存活（GET / 不应弹回 /login）");
    println!("[3] is_alive = true");

    // 关键探针：网关是否代理内网计费服务器 10.3.100.110
    let target = "http://10.3.100.110/charge/feeitem";
    let wrapped = wrap_url(target, GATEWAY).expect("wrap_url 失败");
    println!("[4] 探针 {target} → {wrapped}");
    let resp = session
        .wrapped_client()
        .get(&wrapped)
        .send()
        .await
        .expect("探针请求失败（网关不可达或未代理该目标）");
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let head: String = body.chars().take(200).collect();
    println!("[4] HTTP {status}，响应前 200 字符：");
    println!("{head}");
    println!("[4] body 总长度: {} 字节", body.len());
}
