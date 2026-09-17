//! CAS 解析与构造的离线测试（不起网络，Task 7 Step 1）。

use campus_auth::cas::{build_login_body, parse_kaptcha, parse_login_response, CasLoginError};
use campus_auth::jar::RecordingJar;
use reqwest::cookie::CookieStore;
use reqwest::header::HeaderValue;
use reqwest::Url;

// ── 1. 错误码映射全覆盖（REPORT.md 全集 16 码） ──────────────────────────────

fn err_of(code: &str) -> CasLoginError {
    parse_login_response(&format!(
        r#"{{"meta":{{"code":0}},"data":{{"code":"{code}"}}}}"#
    ))
    .expect_err("失败响应应返回 Err")
}

#[test]
fn error_code_mapping_covers_full_set() {
    // 专属映射四码
    assert_eq!(err_of("NOUSER"), CasLoginError::WrongUserOrPwd);
    assert_eq!(err_of("CODEFALSE"), CasLoginError::WrongCaptcha);
    assert_eq!(err_of("USERLOCK"), CasLoginError::UserLocked);
    let two = err_of("TWOVERIFY");
    assert!(
        matches!(&two, CasLoginError::NeedTwoVerify(raw) if raw.contains("TWOVERIFY")),
        "TWOVERIFY 应携带 data 原文，实际 {two:?}"
    );
    // 其余 12 码归 Unknown(原样码)
    for code in [
        "USERDISABLED",
        "PASSERROR",
        "USERNOTONLY",
        "PEOPLEMOREACCOUNT",
        "ISBINDOTP",
        "ISBINDWX",
        "ISMODIFYPASS",
        "NETWORKCOMMITMENT",
        "NOREGISTER",
        "NOAUTHORIZATION",
        "OTPERROR",
        "PENDINGACTIVATE",
    ] {
        assert_eq!(
            err_of(code),
            CasLoginError::Unknown(code.to_string()),
            "错误码 {code} 应归 Unknown"
        );
    }
}

// ── 2. 成功响应解析 ─────────────────────────────────────────────────────────

#[test]
fn parses_top_level_tgt_ticket() {
    // REPORT.md 三节实测：成功响应顶层即 {tgt, ticket}，无 data 包裹
    let ok = parse_login_response(r#"{"meta":{"code":0},"tgt":"TGT-abc","ticket":"ST-xyz"}"#)
        .expect("成功响应应解析通过");
    assert_eq!(ok.tgt, "TGT-abc");
    assert_eq!(ok.ticket, "ST-xyz");
}

#[test]
fn parses_data_wrapped_tgt_ticket() {
    // cas.js 兼容形态：data 包裹 {tgt, ticket}
    let ok = parse_login_response(r#"{"data":{"tgt":"TGT-1","ticket":"ST-2"}}"#)
        .expect("data 包裹形态应解析通过");
    assert_eq!(ok.tgt, "TGT-1");
    assert_eq!(ok.ticket, "ST-2");
}

#[test]
fn rejects_response_without_tgt_ticket() {
    assert!(matches!(
        parse_login_response(r#"{"meta":{"code":0}}"#),
        Err(CasLoginError::Unknown(_))
    ));
}

// ── 3. build_login_body 构造 ────────────────────────────────────────────────

#[test]
fn login_body_contains_all_fields_urlencoded() {
    let body = build_login_body(
        "22000000",
        "aabbccdd",
        "https://my.cwxu.edu.cn/shiro-cas",
        "uid-32hex",
        "63",
    );
    assert!(body.contains("username=22000000"), "body={body}");
    assert!(body.contains("password=aabbccdd"), "body={body}");
    // service 的 :// 与 / 需百分号编码
    assert!(
        body.contains("service=https%3A%2F%2Fmy.cwxu.edu.cn%2Fshiro-cas"),
        "body={body}"
    );
    assert!(body.contains("loginType="), "body={body}");
    assert!(body.contains("id=uid-32hex"), "body={body}");
    assert!(body.contains("code=63"), "body={body}");
    assert!(body.contains("otpcode="), "body={body}");
    // 字段总数：username/password/service/loginType/id/code/otpcode（REPORT.md 一节协议体）
    assert_eq!(body.split('&').count(), 7, "body={body}");
    // 值里的保留字符不得裸奔
    assert!(!body.contains("://"), "service 不得未编码出现，body={body}");
}

// ── 4. kaptcha 响应解析 ─────────────────────────────────────────────────────

#[test]
fn parses_kaptcha_payload() {
    // REPORT.md 一节实测形态：content 带 data URI 前缀
    let info = parse_kaptcha(
        r#"{"kaptchaType":"1","uid":"a1b2c3d4e5f60718293a4b5c6d7e8f90","content":"data:image/png;base64,iVBORw0KGgo="}"#,
    )
    .expect("kaptcha 响应应解析通过");
    assert_eq!(info.kaptcha_type, "1");
    assert_eq!(info.uid, "a1b2c3d4e5f60718293a4b5c6d7e8f90");
    // 前缀已剥（probe.js:52 同款处理）
    assert_eq!(info.png_base64, "iVBORw0KGgo=");
}

// ── 5. RecordingJar set/restore/has 往返（不起网络） ─────────────────────────

#[test]
fn recording_jar_set_restore_has_roundtrip() {
    let url = Url::parse("https://my.cwxu.edu.cn/").expect("测试 URL 应合法");
    let jar = RecordingJar::new();
    let heads = [
        HeaderValue::from_static("CASTGC=TGT-abc; Path=/; HttpOnly"),
        HeaderValue::from_static("customsid=shiro-xyz; Path=/; HttpOnly"),
        HeaderValue::from_static("rememberMe=b64value==; Max-Age=31536000"),
        HeaderValue::from_static("not-a-cookie"),
    ];
    jar.set_cookies(&mut heads.iter(), &url);

    assert!(jar.has("CASTGC"));
    assert!(jar.has("customsid"));
    assert!(!jar.has("nope"), "未设置的 cookie 不应存在");
    assert!(!jar.snapshot().is_empty(), "set 后 snapshot 非空");

    let snap = jar.snapshot();
    assert!(snap.contains(&("CASTGC".to_string(), "TGT-abc".to_string())));
    assert!(snap.contains(&("customsid".to_string(), "shiro-xyz".to_string())));
    // 值含 = 时按第一个 = 切分，尾部的 = 属于值
    assert!(snap.contains(&("rememberMe".to_string(), "b64value==".to_string())));
    // 无 = 的头不入记录
    assert!(snap.iter().all(|(n, _)| n != "not-a-cookie"));

    // 新 jar restore 往返：记录回到位 + 内置 Jar 真正持有（请求路径可用）
    let jar2 = RecordingJar::new();
    jar2.restore(&snap);
    assert!(jar2.has("CASTGC"));
    assert!(jar2.has("customsid"));
    assert_eq!(jar2.snapshot(), snap, "restore 往返后 snapshot 应一致");
    let cookie_header = jar2
        .cookies(&url)
        .expect("restore 后内置 Jar 应能取回 Cookie 头");
    let cookie_str = cookie_header.to_str().expect("Cookie 头应为可见 ASCII");
    assert!(cookie_str.contains("CASTGC=TGT-abc"), "cookie={cookie_str}");
    assert!(cookie_str.contains("customsid=shiro-xyz"), "cookie={cookie_str}");
}

#[test]
fn recording_jar_restore_overwrites_same_name() {
    let jar = RecordingJar::new();
    jar.restore(&[("customsid".to_string(), "old".to_string())]);
    jar.restore(&[("customsid".to_string(), "new".to_string())]);
    let snap = jar.snapshot();
    let customsids: Vec<_> = snap
        .iter()
        .filter(|(n, _)| n == "customsid")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(customsids, ["new"], "同名 restore 应覆盖旧值");
}
