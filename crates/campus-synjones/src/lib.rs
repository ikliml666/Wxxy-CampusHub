//! 慧新E校（哈尔滨新中中 / synjones 移动服务平台 v1.07.fix）协议核心。
//!
//! 提供：CAS → lyCas SSO 桥换 token（[`sso`]）、统一头组注入与三套信封解析（[`client`]）。
//! 无 Tauri 依赖，安卓端可复用。
//!
//! # 协议事实（2026-09-19 匿名实测 + 各前端 bundle 逆向，细节见
//! `docs/superpowers/plans/2026-09-19-m3-ecard-electricity.md` §1）
//!
//! - **服务前缀各成一个服务，禁止按前缀猜**：`/berserker-auth`（认证）、
//!   `/berserker-app`（一卡通 `ykt/tsm/*`、appScheme）、`/berserker-search`（一卡通流水/统计）、
//!   `/charge`（电费）。`/berserker-acc` 实测 **404**。
//! - **鉴权两件**：头 `synjones-auth: <token_type> <token>`（`token_type` 缺省 `bearer`）；
//!   来源参数 `synAccessSource` 取值固定 [`SYN_ACCESS_SOURCE`]，且**双份携带**——GET 走 query
//!   再另加同名头，POST form 合并进 body 再加同名头（官方前端请求拦截器行为，照抄最稳）。
//! - **4030 判定**：`HTTP 401` 且 `body.code == 4030`（不是 HTTP 403），含义是来源授权被拒。
//! - **三套响应信封不可混用**：berserker 系 `{code,success,data,msg}`、charge 系
//!   `{code,message}`（401 时 message 可能是空串）、search 系 `{code,data,msg}`。
//! - **token 只在内存**：不落盘、不进日志、不进错误消息；[`SynjonesToken`] 的 `Debug` 手写打码。
//!
//! # 字段语义修订（2026-09-19 live 实测，推翻计划 §1.4 的注释）
//!
//! 计划 §1.4 把 `elec_accamt` 注为「慧新E校侧的电费余额」——**错**。实测对照：
//! `queryCurrentCard` 的 `elec_accamt = 8151`（分），与同日最新一条消费流水的
//! `cardBalance = 8151` 以及 `accinfo[0].balance = 8151` 三者完全一致 ⇒
//! **`elec_accamt` 是一卡通电子账户余额**（`elec` = electronic，非「电控」）。
//! 一卡通余额取值口径：电子账户余额 `elec_accamt`（＝`accinfo[].balance` 聚合），
//! 卡账户余额 `(db_balance + unsettle_amount)`；**电费余额不在此接口**，须走 `/charge/*` 级联。
//! 另：流水 `type` 实测是收支方向过滤（`1`=收入 36 条 / `2`=支出 1006 条 / `3`=0 条），
//! 与记录里的 `typeFrom`（`"1"`/`"2"`）一一对应。

pub mod charge;
pub mod client;
pub mod ecard;
pub mod ecard_ops;
pub mod ecard_stats;
pub mod recharge;
pub mod sso;
pub mod turnover;

pub use charge::{
    list_feeitems, query_cascade, Choice, ElectricityQuery, ElectricityView, FeeItem, Field,
    RoomStep,
};
pub use client::SynjonesClient;
pub use ecard::{CardInfo, Transaction, Transactions};
pub use recharge::{
    cancel_order, create_order, fetch_order_status, fetch_pay_methods, query_account, submit_pay,
    PayMethod, PasswordPad, RechargeOrder,
};
pub use sso::sso_token;

/// 慧新E校平台内网根（明文 http，校外需 WebVPN，M3 只验校内侧）。
pub const BERSERKER_BASE: &str = "http://10.3.100.110";

/// CAS 端注册的 lyCas service 路径。
///
/// ⚠️ 换票（`POST {CAS_BASE}/v1/tickets/{tgt}`）必须用它；[`LY_CAS_REDIRECT_PATH`] 只是给浏览器用的入口。
/// 实测：`GET {LY_CAS_REDIRECT_PATH}?targetUrl=…` 302 到 CAS 登录页，其 `service` 即本路径 + `targetUrl`。
pub const LY_CAS_LOGIN_PATH: &str = "/berserker-auth/cas/login/lyCas";

/// 浏览器入口路径（人/浏览器用，客户端换票**不用**它）。
pub const LY_CAS_REDIRECT_PATH: &str = "/berserker-auth/cas/redirect/lyCas";

/// lyCas service 全值前缀（= `BERSERKER_BASE + LY_CAS_LOGIN_PATH`，由单测钉死不漂移）。
pub const LY_CAS_SERVICE_PREFIX: &str = "http://10.3.100.110/berserker-auth/cas/login/lyCas";

/// 来源参数值：官方前端取 `sessionStorage.agentType`（缺省回落 `h5`）。
/// 4030 结论：`pc` 来源被服务端拒绝、`app` 放行，故本项目固定 `app`。
pub const SYN_ACCESS_SOURCE: &str = "app";

/// 慧新E校协议层错误。
///
/// 与 [`campus_auth::CampusAuthError`] 一样把 reqwest 错误压成字符串——
/// 上层只认本枚举，不把网络层细节冒到前端。
#[derive(Debug, thiserror::Error)]
pub enum CampusSynjonesError {
    /// 会话无效（HTTP 401 / 业务 code∈{401,4030,4037,4038,4011}）：需重新走 SSO 桥。
    #[error("慧新E校会话已失效，请重新登录")]
    NotLogin,
    /// SSO 桥失败（换票、跟链、落点无 token）。文案可操作，引导重新登录。
    #[error("慧新E校单点登录失败：{0}")]
    SsoFailed(String),
    /// 业务错误信封（HTTP 200 但 code != 200）。
    #[error("慧新E校接口错误（code={code}）：{msg}")]
    Api { code: i32, msg: String },
    /// 非 401 的 HTTP 异常（状态码写进字符串，不冒 reqwest 错误）。
    #[error("慧新E校请求失败：{0}")]
    Http(String),
    /// 响应解析失败（信封缺失/字段类型不符）。
    #[error("慧新E校响应解析失败：{0}")]
    Parse(String),
}

/// 慧新E校访问令牌。
///
/// `Debug` 手写、token 打码——本类型可能被 `{:?}` 打进 panic 消息或诊断日志。
#[derive(Clone)]
pub struct SynjonesToken {
    pub access_token: String,
    /// 实测 `oauth/token` 返回 `token_type`（一般 `bearer`，可能为空串）。
    pub token_type: String,
}

impl SynjonesToken {
    /// 以实测缺省 `token_type = "bearer"` 构造（SSO 桥落点不带 token_type）。
    pub fn bearer(access_token: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            token_type: "bearer".to_string(),
        }
    }

    /// `synjones-auth` 头/query 值：`<token_type> <token>`（token_type 为空时回落 `bearer`）。
    pub fn auth_value(&self) -> String {
        let tt = if self.token_type.trim().is_empty() {
            "bearer"
        } else {
            self.token_type.trim()
        };
        format!("{tt} {}", self.access_token)
    }

    pub fn is_empty(&self) -> bool {
        self.access_token.trim().is_empty()
    }
}

impl std::fmt::Debug for SynjonesToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SynjonesToken {{ access_token: ***, token_type: {:?} }}",
            self.token_type
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 常量防漂移：service 全值前缀必须等于 BASE + LOGIN_PATH（拆成两个常量供不同用途）。
    #[test]
    fn ly_cas_service_prefix_matches_base_plus_path() {
        assert_eq!(
            LY_CAS_SERVICE_PREFIX,
            format!("{BERSERKER_BASE}{LY_CAS_LOGIN_PATH}")
        );
    }

    /// 常量防漂移：字面值钉死（改动必须是有意识的协议变更）。
    #[test]
    fn constants_literal_values() {
        assert_eq!(BERSERKER_BASE, "http://10.3.100.110");
        assert_eq!(
            LY_CAS_LOGIN_PATH,
            "/berserker-auth/cas/login/lyCas",
            "换票 service 路径（redirect 路径另有用处，勿混）"
        );
        assert_eq!(
            LY_CAS_REDIRECT_PATH,
            "/berserker-auth/cas/redirect/lyCas"
        );
        assert_eq!(SYN_ACCESS_SOURCE, "app", "pc 被服务端拒（4030），必须 app");
    }

    /// Debug 必须打码：token 不得出现在 `{:?}` 输出里。
    #[test]
    fn token_debug_masks_secret() {
        let t = SynjonesToken::bearer("super-secret-token-value");
        let dbg = format!("{t:?}");
        assert!(!dbg.contains("super-secret-token-value"), "实际 {dbg}");
        assert!(dbg.contains("***"));
    }

    /// auth_value：token_type 空串回落 bearer。
    #[test]
    fn token_auth_value_falls_back_to_bearer() {
        assert_eq!(SynjonesToken::bearer("T").auth_value(), "bearer T");
        let empty_type = SynjonesToken {
            access_token: "T".to_string(),
            token_type: String::new(),
        };
        assert_eq!(empty_type.auth_value(), "bearer T");
        let upper = SynjonesToken {
            access_token: "T".to_string(),
            token_type: "Bearer".to_string(),
        };
        assert_eq!(upper.auth_value(), "Bearer T");
    }
}
