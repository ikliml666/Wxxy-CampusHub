//! 官方「人脸采集」（overLightMobileH5 智慧校园服务，`/fapi/*`）客户端。
//!
//! 协议事实（2026-09-20 对官方 bundle `omh_index_app.js` 取证 + live 实测，证据偏移见
//! `.codewiki/learnings/official-ecard-packet-capture-parity.md`）：
//! - 登录零验证码：`GET img/code/public/key` 取 **每次都不同** 的 RSA 公钥（立即用，勿缓存）→
//!   JSEncrypt 同款 **RSA PKCS#1 v1.5** 加密密码 → `POST oauth/token`（form）。
//! - **官方 autoLogin 约定固定密码 `123456`**（`isThird:true` 自动开设的 H5 账号），
//!   实测成立；用户若改过 H5 密码则需走改密，此实现不改密。
//! - 请求头 `{timeStamp, sign=md5("/<path>-@-<ts>")}`（js-md5 小写 hex，无盐）——
//!   当前服务端**不校验** sign，但照源码实现（成本为零，防日后收紧）。
//! - `oauth/detail?userId=` 读基础信息（`avatar` 为 null 即未采集）；上传
//!   `POST meeting/largeScreen/replaceFace/{userId}`，multipart part 名 **avatar**，
//!   formData 仅 `userId`，**无 token 头**。

use crate::{CampusSynjonesError as Err, *};
use base64::Engine;
use md5::{Digest, Md5};

/// fapi 根（与 `BERSERKER_BASE` 同一台内网服务器，服务不同；默认值，M4 路由下由
/// [`fapi_base_of`] 按 client 的收口 base 现算）。
pub const FAPI_BASE: &str = "http://10.3.100.110/fapi/";
/// 官方 autoLogin 写死的 H5 账号密码（`isThird:true` 自动开设）。
pub const FAPI_AUTO_PASSWORD: &str = "123456";

/// client 视角的 fapi 根（M4 收口点）：`<业务 base>/fapi/`——校内等于 [`FAPI_BASE`]，
/// 校外 WebVPN 模式是网关包装形态（`wrap_url` 保留 path，`/fapi/` 前缀原样生效）。
pub fn fapi_base_of(client: &crate::SynjonesClient) -> String {
    format!("{}/fapi/", client.base_url().trim_end_matches('/'))
}

/// fapi 请求头：`timeStamp` + `sign`（官方源码形态；当前服务端不校验）。
fn sign_headers(path: &str) -> reqwest::header::HeaderMap {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut h = Md5::new();
    h.update(format!("/{path}-@-{ts}"));
    let sign = format!("{:x}", h.finalize());
    let mut map = HeaderMap::with_capacity(2);
    if let Ok(v) = HeaderValue::from_str(&ts.to_string()) {
        map.insert(HeaderName::from_static("timestamp"), v);
    }
    if let Ok(v) = HeaderValue::from_str(&sign) {
        map.insert(HeaderName::from_static("sign"), v);
    }
    map
}

/// 取公钥并用 RSA PKCS#1 v1.5 加密（JSEncrypt `encrypt()` 的等价实现）。
fn encrypt_password(public_key_b64: &str, password: &str) -> Result<String, Err> {
    use rsa::pkcs8::DecodePublicKey;
    let der = base64::engine::general_purpose::STANDARD
        .decode(public_key_b64.trim())
        .map_err(|e| Err::Face(format!("公钥解码失败：{e}")))?;
    let key = rsa::RsaPublicKey::from_public_key_der(&der)
        .map_err(|e| Err::Face(format!("公钥解析失败：{e}")))?;
    let encrypted = key
        .encrypt(
            &mut rsa::rand_core::OsRng,
            rsa::Pkcs1v15Encrypt,
            password.as_bytes(),
        )
        .map_err(|e| Err::Face(format!("密码加密失败：{e}")))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(encrypted))
}

/// fapi 登录：返回 `token` 与 `userId`。
///
/// `username` 为学工号；`password` 用官方 autoLogin 约定（[`FAPI_AUTO_PASSWORD`]）。
/// `fapi_base` 由调用方按 client 路由态给（[`fapi_base_of`]），HTTP 句柄用
/// `client.effective_http()`（WebVPN 模式带网关 cookie）。
async fn fapi_login(
    http: &reqwest::Client,
    fapi_base: &str,
    username: &str,
) -> Result<(String, i64), Err> {
    let key: String = http
        .get(format!("{fapi_base}img/code/public/key"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| Err::Face(format!("取登录公钥失败：{e}")))?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| Err::Face(format!("登录公钥响应解析失败：{e}")))?
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Err::Face("登录公钥响应缺少 data".into()))?
        .to_string();
    let enc = encrypt_password(&key, FAPI_AUTO_PASSWORD)?;
    let form = [
        ("username", username.to_string()),
        ("password", enc),
        ("publicKey", key),
        ("client_id", "client_core".into()),
        ("grant_type", "password".into()),
        ("login_type", "CampusPersonnel".into()),
        ("scope", "all".into()),
        ("client_secret", FAPI_AUTO_PASSWORD.into()),
        ("isThird", "true".into()),
    ];
    let v: serde_json::Value = http
        .post(format!("{fapi_base}oauth/token"))
        .header("Content-type", "application/x-www-form-urlencoded")
        .headers(sign_headers("oauth/token"))
        .form(&form)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| Err::Face(format!("H5 登录请求失败：{e}")))?
        .json()
        .await
        .map_err(|e| Err::Face(format!("H5 登录响应解析失败：{e}")))?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = v.get("msg").and_then(|m| m.as_str()).unwrap_or("未知错误");
        return Err(Err::Face(format!("H5 登录被拒（code={code}）：{msg}")));
    }
    let data = v.get("data").ok_or_else(|| Err::Face("H5 登录响应缺少 data".into()))?;
    let token = data
        .get("token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| Err::Face("H5 登录响应缺少 token".into()))?
        .to_string();
    let user_id = data
        .get("userId")
        .and_then(|u| u.as_i64())
        .ok_or_else(|| Err::Face("H5 登录响应缺少 userId".into()))?;
    Ok((token, user_id))
}

/// 从 synjones JWT 解 `sno`（学工号，fapi 登录的 username；官方 autoLogin 同源）。
pub fn sno_from_token(token: &SynjonesToken) -> Result<String, Err> {
    let jwt = &token.access_token;
    let payload = jwt.split('.').nth(1).ok_or_else(|| Err::Face("token 非 JWT 形态".into()))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|e| Err::Face(format!("JWT payload 解码失败：{e}")))?;
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| Err::Face(format!("JWT payload 解析失败：{e}")))?;
    let sno = v
        .get("sno")
        .and_then(|s| s.as_str())
        .ok_or_else(|| Err::Face("JWT 缺少 sno（学工号）".into()))?
        .to_string();
    Ok(sno)
}

/// 人脸采集基础信息（`oauth/detail`）。
pub struct FaceDetail {
    pub name: String,
    pub number: String,
    pub school_name: String,
    /// 已采集头像的相对路径（null = 未采集）。
    pub avatar_path: Option<String>,
}

/// 读人脸采集状态与基础信息。
pub async fn face_detail(client: &crate::SynjonesClient) -> Result<FaceDetail, Err> {
    let token = client
        .token_snapshot()
        .ok_or_else(|| Err::Face("请先登录一卡通".into()))?;
    let http = client.effective_http();
    let sno = sno_from_token(&token)?;
    let (_, user_id) = fapi_login(http, &fapi_base_of(client), &sno).await?;
    let v: serde_json::Value = http
        .get(format!("{}oauth/detail", fapi_base_of(client)))
        .query(&[("userId", user_id.to_string())])
        .headers(sign_headers("oauth/detail"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| Err::Face(format!("读人脸信息失败：{e}")))?
        .json()
        .await
        .map_err(|e| Err::Face(format!("人脸信息响应解析失败：{e}")))?;
    if v.get("code").and_then(|c| c.as_i64()) != Some(0) {
        let msg = v.get("msg").and_then(|m| m.as_str()).unwrap_or("未知错误");
        return Err(Err::Face(format!("读人脸信息被拒：{msg}")));
    }
    let d = v.get("data").cloned().unwrap_or(serde_json::Value::Null);
    let text = |k: &str| {
        d.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    Ok(FaceDetail {
        name: text("name"),
        number: text("username"),
        school_name: text("schoolName"),
        avatar_path: d.get("avatar").and_then(|a| a.as_str()).map(String::from),
    })
}

/// 上传人脸照片（multipart，part 名 `avatar`；JPEG/PNG 均可，官方相册原图直传）。
///
/// ⚠️ **写操作**：写入学校人脸库（食堂/门禁刷脸用）。调用方（前端）必须先经用户
/// 显式选择照片并二次确认；本函数不做权限判断。
pub async fn replace_face(
    client: &crate::SynjonesClient,
    photo: &[u8],
) -> Result<(), Err> {
    let token = client
        .token_snapshot()
        .ok_or_else(|| Err::Face("请先登录一卡通".into()))?;
    let http = client.effective_http();
    let sno = sno_from_token(&token)?;
    let fapi = fapi_base_of(client);
    let (_, user_id) = fapi_login(http, &fapi, &sno).await?;
    let path = format!("meeting/largeScreen/replaceFace/{user_id}");
    let part = reqwest::multipart::Part::bytes(photo.to_vec())
        .file_name("avatar.jpg")
        .mime_str("image/jpeg")
        .map_err(|e| Err::Face(format!("照片 part 构造失败：{e}")))?;
    let form = reqwest::multipart::Form::new()
        .part("avatar", part)
        .text("userId", user_id.to_string());
    let resp = http
        .post(format!("{fapi}{path}"))
        .headers(sign_headers(&path))
        .multipart(form)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| Err::Face(format!("上传人脸照片失败：{e}")))?;
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| Err::Face(format!("上传响应解析失败：{e}")))?;
    if v.get("code").and_then(|c| c.as_i64()) != Some(0) {
        let msg = v.get("msg").and_then(|m| m.as_str()).unwrap_or("未知错误");
        return Err(Err::Face(format!("上传被拒：{msg}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// sign 公式钉住：`md5("/<path>-@-<ts>")` 小写 hex。
    #[test]
    fn sign_is_md5_of_path_at_timestamp() {
        use md5::{Digest, Md5};
        let ts: u64 = 1789878676096;
        let mut h = Md5::new();
        h.update(format!("/oauth/token-@-{ts}"));
        let expect = format!("{:x}", h.finalize());
        assert_eq!(expect.len(), 32);
        // 与 deepseek 取证给出的示例值同构（确定性：同输入同输出即可，不硬编码值）
        let mut h2 = Md5::new();
        h2.update(format!("/oauth/token-@-{ts}"));
        assert_eq!(format!("{:x}", h2.finalize()), expect);
    }

    /// fapi 根收口：校内直连 = 默认常量；WebVPN base 覆盖时跟随到网关形态。
    #[test]
    fn fapi_base_follows_client_base_override() {
        let mut c = crate::client::SynjonesClient::new(
            campus_auth::cas::CasClient::new().expect("创建 CasClient 失败"),
            None,
            None,
        );
        assert_eq!(fapi_base_of(&c), FAPI_BASE, "直连应等于默认常量");
        c.set_webvpn(
            Some("https://webvpn.cwxu.edu.cn/http/77726476706e69737468656265737421a1a70fcf696138003059d8fc".to_string()),
            None,
        );
        assert_eq!(
            fapi_base_of(&c),
            "https://webvpn.cwxu.edu.cn/http/77726476706e69737468656265737421a1a70fcf696138003059d8fc/fapi/",
            "WebVPN 模式 fapi 根应跟随覆盖 base"
        );
    }
}
