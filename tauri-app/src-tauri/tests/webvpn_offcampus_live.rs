//! M4 校外（OffCampus）完整链路 live 探针——#[ignore]：真实凭据 + WebVPN 网关。
//!
//! 补上此前「berserker API 带 synjones-auth 头经网关透传未验证」的缺口：
//! CAS 登录 → `WebVpnSession::login`（深澜）→ `sso_token_via` **经网关**跑 SSO 桥换
//! 慧新E校 token → `wrapped_client` GET `queryCurrentCard`（带 synjones-auth 头）
//! → 断言业务信封并打印电子账户余额。
//!
//! 本探针在**校园网内**即可运行（深澜网关是公网入口，包装链路不依赖本机在校外；
//! 校外真机验收仍由用户做——那验证的是 net_zone 判 OffCampus 后的自动接线）。
//!
//! 纪律：凭据不进代码/日志；不打印 cookie/token/响应头；只读接口，无副作用。
//!
//! 运行：
//! `CAMPUS_HUB_CREDS=<凭据文件> cargo test -p campus-hub --test webvpn_offcampus_live -- --ignored --nocapture`

use campus_auth::captcha::{solve, KaptchaTemplates};
use campus_auth::cas::CasClient;
use campus_auth::rsa::rsa_encrypt_hex;
use campus_synjones::ecard::EP_CURRENT_CARD;
use campus_synjones::sso::sso_token_via;
use campus_synjones::{BERSERKER_BASE, SynjonesToken};
use campus_webvpn::{wrap_url, GATEWAY, WebVpnSession};

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
        user.expect("凭据文件缺少账号"),
        pass.expect("凭据文件缺少密码"),
    )
}

#[tokio::test]
#[ignore = "live：真实凭据 + 深澜网关，仅主智能体验收时 -- --ignored 运行"]
async fn offcampus_full_chain_via_webvpn() {
    // 1. CAS 登录拿 TGT
    let (username, password) = read_creds();
    let cas = CasClient::new().expect("创建 CasClient 失败");
    let password_rsa = rsa_encrypt_hex(&password).expect("RSA 加密失败");
    let templates = KaptchaTemplates::load();
    let mut tgt = None;
    for attempt in 1..=3 {
        let captcha = cas.kaptcha().await.expect("取验证码失败");
        let png = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &captcha.png_base64,
        )
        .expect("验证码 base64 解码失败");
        let Some(answer) = solve(&png, &templates) else {
            println!("[CAS 第 {attempt} 张] 识别失败，换一张");
            continue;
        };
        let ok = cas
            .login(&username, &password_rsa, &captcha.uid, &answer)
            .await
            .expect("CAS 登录请求失败");
        tgt = Some(ok.tgt);
        break;
    }
    let tgt = tgt.expect("3 张验证码均未识别或登录失败");
    assert!(tgt.starts_with("TGT-"));
    println!("[1] CAS TGT 就绪（前 12 字符={}…）", &tgt[..12.min(tgt.len())]);

    // 2. 静默登 WebVPN
    let vpn = WebVpnSession::login(&cas, &tgt)
        .await
        .expect("WebVPN 登录失败");
    assert!(vpn.is_alive().await, "WebVPN 会话应存活");
    println!("[2] WebVPN 会话就绪且存活");

    // 3. 经网关跑 SSO 桥换慧新E校 token（此前未验证的关键段）
    let target = format!("{BERSERKER_BASE}/campus-card-pc/");
    let token: SynjonesToken = sso_token_via(&cas, &tgt, &target, &vpn)
        .await
        .expect("经 WebVPN 的 SSO 桥换 token 失败");
    assert!(!token.access_token.is_empty(), "token 不应为空");
    println!(
        "[3] sso_token_via 经网关成功（token 长度 {}，token_type {:?}）",
        token.access_token.len(),
        token.token_type
    );

    // 4. wrapped_client 直调 berserker API（synjones-auth 头经网关透传）
    let url = wrap_url(&format!("{BERSERKER_BASE}{EP_CURRENT_CARD}"), GATEWAY)
        .expect("wrap_url 失败");
    let http = vpn.wrapped_client();
    let resp = http
        .get(&url)
        .query(&[("synAccessSource", "app")])
        .header("synjones-auth", token.auth_value())
        .header("synAccessSource", "app")
        .header("Accept", "application/json, text/plain, */*")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .expect("经网关请求 queryCurrentCard 失败");
    let status = resp.status().as_u16();
    let body = resp.text().await.expect("读响应失败");
    println!("[4] queryCurrentCard 经网关 → HTTP {status}，body {} 字节", body.len());
    assert_eq!(status, 200, "HTTP 层应 200：{}", &body[..body.len().min(120)]);

    // 5. 业务信封判定（berserker 系成功 = HTTP 2xx 且 code==200，parse_envelope 同款）+ 打印余额
    let v: serde_json::Value = serde_json::from_str(&body).expect("响应应为 JSON");
    let code = v.get("code").cloned().unwrap_or(serde_json::Value::Null);
    assert_eq!(code, serde_json::json!(200), "berserker 信封 code 应为 200：{code}");
    let elec_fen = v
        .pointer("/data/elec_accamt")
        .and_then(|x| {
            x.as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| x.as_f64())
        })
        .unwrap_or(-1.0);
    println!(
        "[5] 业务信封 code=200 ✓ 电子账户余额 = {:.2} 元（elec_accamt={} 分）",
        elec_fen / 100.0,
        elec_fen
    );
    // data = {account, card, errmsg, retcode, sno}：ykt 业务层带 retcode（双层判定第二层）
    let retcode = v.pointer("/data/retcode").cloned().unwrap_or_default();
    let errmsg = v.pointer("/data/errmsg").and_then(|x| x.as_str()).unwrap_or("");
    assert_eq!(
        retcode,
        serde_json::json!("0"),
        "业务层 retcode 应为 \"0\"（经网关的 token 被业务接受）：retcode={retcode} errmsg={errmsg}"
    );
    let card = v.pointer("/data/card").cloned().unwrap_or_default();
    let card_obj = if card.is_array() {
        card.get(0).cloned().unwrap_or_default()
    } else {
        card
    };
    let elec_fen = card_obj
        .get("elec_accamt")
        .and_then(|x| {
            x.as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| x.as_f64())
        })
        .unwrap_or(-1.0);
    println!(
        "[5] 业务层 retcode=0 ✓ 电子账户余额 = {:.2} 元（elec_accamt={} 分）",
        elec_fen / 100.0,
        elec_fen
    );
    assert!(elec_fen >= 0.0, "应取到 elec_accamt；card 键 = {:?}", card_obj.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()));
}
