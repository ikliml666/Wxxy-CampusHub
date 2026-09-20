//! plat 设备写操作探针——**红线：绝不真发写请求**（真发会踢掉用户设备登录态）。
//!
//! 本探针只做到：复用应用 TGT 换 token → **只读**枚举绑定设备 → 打印两个写端点的
//! 请求构造预览（端点 + body 形态）。报文的官方取证已完成（2026-09-20，
//! `/plat/js/searcher.89d412b9.js` deviceManage / authorizedList 组件）：
//! `POST /berserker-base/equipment/offlineEquipmentByUser`（下线在线设备）与
//! `POST /berserker-base/equipment/removeEquipmentByUser`（移除已授权设备），
//! body 均为 `{"equipmentUserBh":"<设备条目id>"}`（axios 默认 JSON）。
//!
//! 运行（ignored，默认不跑）：`cargo test -p campus-synjones --test plat_device_write_probe_live -- --ignored --nocapture`
//! 前置：应用在本机登录过一次（`%APPDATA%/campushub/session.json` 的 TGT 可用）。

use campus_auth::cas::CasClient;
use campus_synjones::sso::{default_target_url, sso_token};
use campus_synjones::{BERSERKER_BASE, SYN_ACCESS_SOURCE};
use serde_json::Value;

/// 复用应用 session.json 的 TGT（DPAPI 解密；与 plat_sso_probe_live 同款辅助）。
fn tgt_from_app_session() -> Option<String> {
    #[cfg(windows)]
    fn dpapi_unprotect(cipher: &[u8]) -> Result<Vec<u8>, String> {
        #[repr(C)]
        struct DataBlob {
            cb_data: u32,
            pb_data: *mut u8,
        }
        #[link(name = "crypt32")]
        extern "system" {
            fn CryptUnprotectData(
                pDataIn: *const DataBlob,
                ppszDataDescr: *mut *mut u16,
                pOptionalEntropy: *const DataBlob,
                pvReserved: *mut core::ffi::c_void,
                pPromptStruct: *mut core::ffi::c_void,
                dwFlags: u32,
                pDataOut: *mut DataBlob,
            ) -> i32;
            fn LocalFree(h: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        }
        unsafe {
            let mut out = DataBlob { cb_data: 0, pb_data: std::ptr::null_mut() };
            let ok = CryptUnprotectData(
                &DataBlob { cb_data: cipher.len() as u32, pb_data: cipher.as_ptr() as *mut u8 },
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                &mut out,
            );
            if ok == 0 {
                return Err("CryptUnprotectData 失败".into());
            }
            let slice = std::slice::from_raw_parts(out.pb_data, out.cb_data as usize);
            Ok(slice.to_vec())
        }
    }
    #[cfg(not(windows))]
    fn dpapi_unprotect(_cipher: &[u8]) -> Result<Vec<u8>, String> {
        Err("非 Windows 平台不支持 DPAPI".into())
    }
    let path = std::path::PathBuf::from(std::env::var("APPDATA").ok()?)
        .join("campushub")
        .join("session.json");
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let b64 = v.get("tgtB64")?.as_str()?;
    use base64::Engine as _;
    let plain = dpapi_unprotect(&base64::engine::general_purpose::STANDARD.decode(b64).ok()?).ok()?;
    String::from_utf8(plain).ok().filter(|t| !t.trim().is_empty())
}

/// plat 鉴权头的 GET（只读；与 plat_sso_probe_live 同款）。
async fn plat_get(client: &reqwest::Client, path: &str, token: &str) -> Value {
    let r = client
        .get(format!("{BERSERKER_BASE}{path}"))
        .header("synjones-auth", format!("bearer {token}"))
        .header("synAccessSource", SYN_ACCESS_SOURCE)
        .send()
        .await
        .expect("plat 请求网络失败");
    r.json().await.expect("plat 响应非 JSON")
}

#[tokio::test]
#[ignore = "live：需校园网与应用 TGT；红线=只枚举+打印请求构造，绝不真发写请求"]
async fn device_write_probe_dry_run() {
    let cas = CasClient::new().expect("创建 CasClient 失败");
    let tgt = tgt_from_app_session().expect("无应用 TGT（请先在应用内登录一次）");
    let token = sso_token(&cas, &tgt, &default_target_url())
        .await
        .expect("PC targetUrl 换票失败（TGT 过期？）");

    let http = reqwest::Client::new();
    for (status, label, endpoint) in [
        ("1", "已登录在线（可下线）", "offlineEquipmentByUser"),
        ("0", "已授权（可移除授权）", "removeEquipmentByUser"),
    ] {
        let v = plat_get(
            &http,
            &format!("/berserker-base/equipment/searchUserBindEquipment?status={status}"),
            &token.access_token,
        )
        .await;
        println!(
            "[枚举 {label}] code={:?} 条数={:?}",
            v.get("code").and_then(|x| x.as_i64()),
            v.pointer("/data").and_then(Value::as_array).map(|a| a.len()),
        );
        if let Some(items) = v.pointer("/data").and_then(Value::as_array) {
            for it in items {
                println!(
                    "  id={} name={:?} type={:?}",
                    it.get("id").map(|x| x.to_string()).unwrap_or_default(),
                    it.get("name").and_then(Value::as_str),
                    it.get("type").and_then(Value::as_str),
                );
            }
        }
        // 写请求构造预览（plat.rs 的 post_json 同款形态：synjones-auth + synAccessSource 头组 + JSON body）
        println!("[写端点-构造预览 {label}]");
        println!("  POST {BERSERKER_BASE}/berserker-base/equipment/{endpoint}");
        println!("  body={{\"equipmentUserBh\":\"<上列 id>\"}}");
    }
    println!("[红线] 探针到此为止：未发送任何写请求；真发由用户在应用内显式触发。");
}
