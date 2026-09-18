//! jwglxt（正方教务）SSO 与课表拉取 live 全链测试（M2.5 取证固化）。
//!
//! 断言固定的实测事实（2026-09-18 侦察样本）：
//! 1. `jwglxt_sso(tgt)` 一键完成 TGT 换教务 ST + 302 链，落点学生主界面；
//! 2. SSO 后同 jar 直接 `POST /jwglxt/kbcx/xskbcx_cxXsKb.html?gnmkdm=N2151`
//!    （body `xnm=<学年>&xqm=<学期>`）返回 200 JSON 且含 `kbList`，
//!    `parse_kb_response` 解析出课程 > 0；
//! 3. 未登录（教务域无会话）：带 `X-Requested-With` 请求返回自定义状态码 **901**
//!    空 body；不带则返回 200 + 登录页 HTML（`text/html`，不含 kbList）；
//!    M2.5 批次 1 起另断言封装 `fetch_timetable_json`：会话有效时带/不带 TGT 均
//!    返回含 kbList 的 JSON 原文，未登录 + tgt=None 归一为 `JwglNotLogin`。
//!
//! 纪律：ticket/cookie/PII 不打全值；原始响应样本只写仓库外 recon 目录
//! （`CAMPUS_HUB_RECON_DIR`，默认 `%TEMP%/campushub-m25-recon`）。
//! 复跑：`CAMPUS_HUB_CREDS=<凭据文件> cargo test -p campus-auth -- --ignored jwglxt`

use campus_auth::captcha::{solve, KaptchaTemplates};
use campus_auth::cas::{CasClient, JWGL_SERVICE};
use campus_auth::rsa::rsa_encrypt_hex;

/// 从 CAMPUS_HUB_CREDS 指向的凭据文件读取 (账号, 密码)（cas_live.rs 同款解析）。
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

/// recon 目录（仓库外，样本含 PII 绝不入仓库）。
fn recon_dir() -> std::path::PathBuf {
    std::env::var("CAMPUS_HUB_RECON_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("campushub-m25-recon"))
}

const KBCX_URL: &str = "https://jwgl.cwxu.edu.cn/jwglxt/kbcx/xskbcx_cxXsKb.html?gnmkdm=N2151";

/// CAS 登录（验证码自动识别，≤3 张），返回 CasLoginOk。
async fn cas_login(client: &CasClient) -> campus_auth::cas::CasLoginOk {
    let (username, password) = read_creds();
    let templates = KaptchaTemplates::load();
    let password_rsa = rsa_encrypt_hex(&password).expect("RSA 加密密码失败");

    let mut ok = None;
    for _ in 1..=3 {
        let captcha = client.kaptcha().await.expect("获取验证码失败");
        let png = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &captcha.png_base64,
        )
        .expect("验证码 base64 解码失败");
        let Some(answer) = solve(&png, &templates) else {
            continue; // 识别不出换一张，不消耗登录错误计数
        };
        let r = client
            .login(&username, &password_rsa, &captcha.uid, &answer)
            .await
            .expect("CAS 登录请求失败（网络层）");
        ok = Some(r);
        break;
    }
    ok.expect("3 张验证码均未能识别（或登录均失败）")
}

/// 课表 POST（`xnm`/`xqm` 表单体；头组可裁剪做对照），返回 (status, content_type, body)。
async fn post_kbcx(
    http: &reqwest::Client,
    xnm: u32,
    xqm: u32,
    with_xrw: bool,
    with_referer: bool,
) -> (reqwest::StatusCode, String, String) {
    let mut rb = http
        .post(KBCX_URL)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded;charset=UTF-8",
        )
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/javascript, */*; q=0.01",
        );
    if with_xrw {
        rb = rb.header("X-Requested-With", "XMLHttpRequest");
    }
    if with_referer {
        rb = rb.header(
            reqwest::header::REFERER,
            "https://jwgl.cwxu.edu.cn/jwglxt/frt/index.html",
        );
    }
    let resp = rb
        .body(format!("xnm={xnm}&xqm={xqm}"))
        .send()
        .await
        .expect("课表 POST 失败");
    let status = resp.status();
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.text().await.unwrap_or_default();
    (status, ct, body)
}

/// 全链：CAS 登录 → jwglxt_sso → 拉课表解析 → 头组/参数对照 → 失败模式断言。
#[tokio::test]
#[ignore = "live：需校园网与真实凭据，仅主智能体验收时 -- --ignored 运行"]
async fn jwglxt_sso_kbcx_live() {
    let dir = recon_dir();
    std::fs::create_dir_all(&dir).expect("创建 recon 目录失败");

    // 1. CAS 登录 + 一键 SSO 进教务
    let client = CasClient::new().expect("创建 CasClient 失败");
    let ok = cas_login(&client).await;
    let landing = client
        .jwglxt_sso(&ok.tgt)
        .await
        .expect("jwglxt SSO 失败");
    println!("[sso] 落点: {}", landing);
    assert_eq!(
        landing.host_str(),
        Some("jwgl.cwxu.edu.cn"),
        "SSO 落点应在教务域"
    );
    assert!(
        landing.path().contains("index_initMenu"),
        "SSO 落点应为学生主界面（index_initMenu），实际 {landing}"
    );
    assert!(
        client.jar().has("JSESSIONID"),
        "教务会话应种下 JSESSIONID"
    );

    // 2. 拉课表（全头组，当前学期：xnm=学年起始年、xqm=3 第1学期）
    let (status, ct, body) = post_kbcx(client.http_client(), 2026, 3, true, true).await;
    println!("[kbcx] status={status} ct={ct} len={}", body.len());
    assert_eq!(status.as_u16(), 200, "已登录拉课表应 200");
    assert!(ct.contains("application/json"), "应返回 JSON，实际 ct={ct}");
    assert!(body.contains("kbList"), "响应应含 kbList");
    std::fs::write(dir.join("kb_sample.json"), &body).ok();

    let courses =
        campus_schedule::zhengfang::parse_kb_response(&body, "recon").expect("解析应成功");
    println!("[parse] 课程条目数={}", courses.len());
    assert!(!courses.is_empty(), "解析出的课程数应 > 0");
    assert!(courses.iter().all(|c| !c.name.is_empty()), "课程名应非空");

    // 2b. 批次 1 封装：fetch_timetable_json 返回 JSON 原文（会话有效时带/不带 TGT 均 200）
    let raw =
        client.fetch_timetable_json(Some(&ok.tgt), 2026, 3).await.expect("fetch_timetable_json 失败");
    assert!(raw.contains("kbList"), "封装拉取应含 kbList");
    let raw2 =
        client.fetch_timetable_json(None, 2026, 3).await.expect("fetch_timetable_json(None) 失败");
    assert!(raw2.contains("kbList"), "会话有效时无 TGT 也应直接 200");

    // 3. 头组对照（打印取证，不打断言——最小头组结论以本输出为准）
    let (s_no_xrw, ct_no_xrw, b_no_xrw) =
        post_kbcx(client.http_client(), 2026, 3, false, true).await;
    println!(
        "[对照 无X-Requested-With] status={s_no_xrw} ct={ct_no_xrw} 含kbList={}",
        b_no_xrw.contains("kbList")
    );
    let (s_no_ref, ct_no_ref, b_no_ref) =
        post_kbcx(client.http_client(), 2026, 3, true, false).await;
    println!(
        "[对照 无Referer] status={s_no_ref} ct={ct_no_ref} 含kbList={}",
        b_no_ref.contains("kbList")
    );
    let (s_bare, ct_bare, b_bare) =
        post_kbcx(client.http_client(), 2026, 3, false, false).await;
    println!(
        "[对照 仅Content-Type] status={s_bare} ct={ct_bare} 含kbList={}",
        b_bare.contains("kbList")
    );

    // 4. 参数对照：第 2 学期（xqm=12）与上学年（xnm=2025）
    for (xnm, xqm) in [(2026u32, 12u32), (2025, 3)] {
        let (s, c, b) = post_kbcx(client.http_client(), xnm, xqm, true, true).await;
        println!(
            "[对照 xnm={xnm} xqm={xqm}] status={s} ct={c} len={} 含kbList={}",
            b.len(),
            b.contains("kbList")
        );
    }

    // 5. 失败模式 A：未登录（干净 jar，教务域无 cookie）+ XRW → 901 空 body
    let fresh = CasClient::new().expect("创建干净 CasClient 失败");
    let (sf, ctf, bf) = post_kbcx(fresh.http_client(), 2026, 3, true, true).await;
    println!("[未登录+XRW] status={} ct={ctf:?} len={}", sf.as_u16(), bf.len());
    assert_eq!(sf.as_u16(), 901, "未登录 ajax 请求应返回自定义状态码 901");
    assert!(bf.is_empty(), "901 响应应为空 body");
    // 批次 1 封装：未登录 + tgt=None → 错误归一为 JwglNotLogin（上层「请先登录」降级信号）
    let err = fresh
        .fetch_timetable_json(None, 2026, 3)
        .await
        .expect_err("未登录拉取应报错");
    assert!(
        matches!(err, campus_auth::CampusAuthError::JwglNotLogin),
        "未登录应映射 JwglNotLogin，实际 {err:?}"
    );

    // 6. 失败模式 B：未登录且无 XRW → 200 + 登录页 HTML
    let (sf2, ctf2, bf2) = post_kbcx(fresh.http_client(), 2026, 3, false, false).await;
    println!("[未登录非ajax] status={sf2} ct={ctf2} len={}", bf2.len());
    assert_eq!(sf2.as_u16(), 200);
    assert!(ctf2.starts_with("text/html"), "应回登录页 HTML，实际 ct={ctf2}");
    assert!(!bf2.contains("kbList"), "登录页不应含 kbList");
    std::fs::write(dir.join("kb_anon_login_page.html"), &bf2).ok();
}

/// 常量防漂移：JWGL_SERVICE 必须是 CAS 端注册的教务回跳值（实测自引用）。
#[test]
fn jwgl_service_shape() {
    assert_eq!(JWGL_SERVICE, "https://jwgl.cwxu.edu.cn/sso/lyiotlogin");
}
