//! plat（移动服务平台）只读面：用户资料 / 设备管理 / 登录日志 / 脱机二维码开关。
//!
//! 鉴权与一卡通**同源**：[`SynjonesClient`] 的统一头组（`synjones-auth` +
//! `synAccessSource`）对 plat API 直接有效——2026-09-20 探针实证服务端不区分
//! token 签发体系（PC 落点 token 调 plat 接口全 200），无需第二套会话。
//! 探针取证见 `tests/plat_sso_probe_live.rs`。
//!
//! 红线：写操作（下线设备 / 解绑校园卡 / 改手机号 / 改密码）**一律由用户显式触发**
//! ——本模块只提供设备写操作两条（官方 bundle 取证见各函数注释），改手机号/改密码
//! 涉短信验证码流程不在此实现。

use crate::client::{Envelope, SynjonesClient};
use crate::CampusSynjonesError;
use serde_json::Value;

/// 用户资料（`GET /berserker-base/user`）——个人资料页数据源。
///
/// 实测字段：`account`/`sno`（学号）、`name`、`departmentName`（部门/班级）、
/// `identityName`（身份）、`sex`、`avatar`、`mobile`（未绑定时 null）、`idNumber`
/// （服务端已掩码）。PII 属「本人查看本人资料」口径原样透传，不进日志。
pub async fn user_profile(client: &SynjonesClient) -> Result<Value, CampusSynjonesError> {
    client
        .get("/berserker-base/user", &[], Envelope::Berserker)
        .await
        .map(|v| v["data"].clone())
}

/// 绑定设备列表（`GET /berserker-base/equipment/searchUserBindEquipment`）。
///
/// `status`：`"1"` = 已登录设备（当前在线会话，可下线）；`"0"` = 已授权手机设备。
/// 条目字段：`id`/`name`/`type`/`status`/`createTime`/`updateTime`（updateTime 即
/// 最近登录时间）。
pub async fn equipment(
    client: &SynjonesClient,
    status: &str,
) -> Result<Value, CampusSynjonesError> {
    client
        .get(
            "/berserker-base/equipment/searchUserBindEquipment",
            &[("status", status)],
            Envelope::Berserker,
        )
        .await
        .map(|v| v["data"].clone())
}

/// 登录日志（`GET /berserker-base/logs/login/user`，分页）。
///
/// 响应 `data` 为 MyBatis-Plus 分页形态：`{records, total, size, current, pages}`。
pub async fn login_logs(
    client: &SynjonesClient,
    page: u32,
    size: u32,
) -> Result<Value, CampusSynjonesError> {
    client
        .get(
            "/berserker-base/logs/login/user",
            &[("current", &page.to_string()), ("size", &size.to_string())],
            Envelope::Berserker,
        )
        .await
        .map(|v| v["data"].clone())
}

/// 脱机二维码开关（`GET /berserker-app/ykt/tsm/getUserOfflienSwitch`）。
///
/// 实测 `data.userOfflienSwitch` 为字符串 `"1"`/`"0"`。
pub async fn offline_switch(client: &SynjonesClient) -> Result<bool, CampusSynjonesError> {
    let v = client
        .get(
            "/berserker-app/ykt/tsm/getUserOfflienSwitch",
            &[],
            Envelope::Berserker,
        )
        .await?;
    Ok(v.pointer("/data/userOfflienSwitch").and_then(|x| x.as_str()) == Some("1"))
}

// ---------------- 付款码（一期；对齐官方 H5 plat/pay 页，三接口 live 探针实证） ----------------

/// 付款码支付方式列表（`GET /berserker-app/ykt/tsm/codebarPayinfo`）。
///
/// 实测 `data` 是**数组**，每项：`account`/`payacc("000")`/`paytype("1")`/
/// `name("一卡通电子钱包")`/`code("ACCOUNT")`/`icon`/`status`/`lostflag`/`freezeflag`/
/// `elec_accamt`(分)/`db_balance`/`unsettle_amount`/`bandacc`(绑定银行卡全号——**PII**，
/// 上层 DTO 绝不透出)/`expdate`/`payid`/`yktPayId`/`payif`。
/// 命令层取 `status==1 && code=="ACCOUNT"` 的项发码。
pub async fn codebar_payinfo(client: &SynjonesClient) -> Result<Value, CampusSynjonesError> {
    client
        .get("/berserker-app/ykt/tsm/codebarPayinfo", &[], Envelope::Berserker)
        .await
        .map(|v| v["data"].clone())
}

/// 取动态条码/二维码（`GET /berserker-app/ykt/tsm/batchGetBarCodeGet`）。
///
/// 实测 `data: {retcode:"0", errmsg, account, expires, barcode:[<20 位串>×10]}`——
/// 官方一次给多段（页面轮换），一期取 `barcode[0]`（命令层解析）；`expires` 原样透传
/// （秒级时间戳或有效期秒数，前端兼容判定）。
///
/// **双层判定的第二层**（第一层信封 `code==200` 已由 client 判过）：`data.retcode=="0"`
/// 才算成功，与写操作同口径（`ecard_ops::require_retcode_ok` 风格）。差异：这里 `retcode`
/// **必须存在且为 "0"**——条码是动态支付凭据，宁可报错重取，也不能把异常响应当成功下发；
/// 失败文案取 `data.errmsg`、回落顶层 `msg`。
pub async fn pay_code(
    client: &SynjonesClient,
    account: &str,
    payacc: &str,
    paytype: &str,
) -> Result<Value, CampusSynjonesError> {
    let v = client
        .get(
            "/berserker-app/ykt/tsm/batchGetBarCodeGet",
            &[("account", account), ("payacc", payacc), ("paytype", paytype)],
            Envelope::Berserker,
        )
        .await?;
    if crate::ecard::text_of(v.get("data").and_then(|d| d.get("retcode"))) == "0" {
        return Ok(v["data"].clone());
    }
    let msg = v
        .get("data")
        .and_then(|d| d.get("errmsg"))
        .and_then(Value::as_str)
        .or_else(|| v.get("msg").and_then(Value::as_str))
        .unwrap_or("获取付款码失败");
    Err(CampusSynjonesError::Parse(msg.trim().to_string()))
}

// ---------------- 设备管理写操作（2026-09-20 官方 plat bundle 取证；由用户显式触发） ----------------

/// 下线指定**已登录在线**设备（`POST /berserker-base/equipment/offlineEquipmentByUser`）。
///
/// 官方 bundle 证据（`/plat/js/searcher.89d412b9.js`，deviceManage 组件）：
/// `this.$api.post("/berserker-base/equipment/offlineEquipmentByUser",{equipmentUserBh:t})`
/// ——body 只有一个字段，值为设备条目 `id`（axios 默认 JSON，同本仓
/// [`SynjonesClient::post_json`] 范式）。官方成功判定仅 `code===200`（无 retcode
/// 第二层），故这里信封判定通过即成功；失败文案由信封层携带透出。
///
/// ⚠️ 写操作会真踢用户设备的登录态：live 验证只到「参数构造」为止（见
/// `tests/plat_device_write_probe_live.rs`），真发必须由用户在前端显式触发。
pub async fn offline_device(
    client: &SynjonesClient,
    equipment_user_bh: &str,
) -> Result<(), CampusSynjonesError> {
    client
        .post_json(
            "/berserker-base/equipment/offlineEquipmentByUser",
            &[("equipmentUserBh", equipment_user_bh.to_string())],
            Envelope::Berserker,
        )
        .await
        .map(|_| ())
}

/// 移除指定**已授权**设备（`POST /berserker-base/equipment/removeEquipmentByUser`）。
///
/// 官方 bundle 证据（同上 searcher chunk，authorizedList 组件）：
/// `this.$api.post("/berserker-base/equipment/removeEquipmentByUser",{equipmentUserBh:t})`
/// ——报文形态与 [`offline_device`] 完全同构，仅端点不同。红线同上。
pub async fn remove_device(
    client: &SynjonesClient,
    equipment_user_bh: &str,
) -> Result<(), CampusSynjonesError> {
    client
        .post_json(
            "/berserker-base/equipment/removeEquipmentByUser",
            &[("equipmentUserBh", equipment_user_bh.to_string())],
            Envelope::Berserker,
        )
        .await
        .map(|_| ())
}
