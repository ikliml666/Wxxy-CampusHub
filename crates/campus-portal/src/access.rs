//! 应用可达性元数据（设计文档 `frontend-design.md` 附录 A 实测矩阵的代码化）。
//!
//! 背景：门户 `isCas` 字段**不可全信**（附录 A：有标 cas 实则停在自家登录页的，
//! 也有域名仅 WebVPN 可达的），客户端自带按 host 归类的可达性表；未命中 host
//! 回落 [`AppAccess::External`]（保守默认：浏览器外链直开）。
//!
//! ⚠️ WebVPN URL 包装不在此实现：网关对未登录请求一律回落 CAS 登录页、丢弃
//! 目标路径（2026-09-18 实测三种明文包装形式最终 URL 完全相同），包装格式在
//! 无 WebVPN 会话的前提下无法验证——该能力与 A 类 CAS 直达签发整体留给 M4。

use reqwest::Url;

/// 应用可达性分类（IPC 以小写字符串透出：`cas` / `webvpn` / `external` / `unavailable`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AppAccess {
    /// 附录 A 类：CAS 直达可用（签发 ST 自动进内页）。
    Cas,
    /// 附录 B 类：域名仅经 WebVPN 网关可达（需校园网或 WebVPN 会话，M4 打通包装）。
    Webvpn,
    /// 浏览器外链（公网可达，直开无提示；表未命中的保守默认）。
    External,
    /// 实测不可达（死链）或需自有登录（非 CAS）——前端点击只提示不打开。
    Unavailable,
}

/// 附录 A 实测矩阵（2026-09-17 浏览器逐站实测；host 全小写，按 appLink 实际
/// host 对照归类）。仅收录有实测结论的 host——未实测的（sygl / tsggcszh /
/// tsgzzwy / www1 等）不进表，由回落规则按 [`AppAccess::External`] 处理；
/// 附录 A C 类中「SSO 通但 403」属账号权限而非链路问题，不单独归类。
const ACCESS_TABLE: &[(&str, AppAccess)] = &[
    // ---- A 类 · CAS 直达可用 ----
    ("whall.cwxu.edu.cn", AppAccess::Cas), // 办事大厅 / 一键通 / gemini 表单底座
    ("cxcyjy.cwxu.edu.cn", AppAccess::Cas), // 创新创业管理平台
    ("jwgl.cwxu.edu.cn", AppAccess::Cas), // 教务系统（正方）
    ("yd.cwxu.edu.cn", AppAccess::Cas), // 低代码表单（申请邮箱/网络报修/漏洞处置单）
    ("10.3.100.110", AppAccess::Cas), // 一卡通（慧新E校）SSO 桥
    ("fysso.chaoxing.com", AppAccess::Cas), // 超星泛雅教学平台（CASSO）
    ("lib.cwxu.edu.cn", AppAccess::Cas), // 电子资源（图书馆公开页）
    ("www.wanfangdata.com.cn", AppAccess::Cas), // 万方（公网 + 校园 IP）
    // ---- B 类 · 需 WebVPN 会话（域名仅经深澜网关可达） ----
    ("jxzlbz1.cwxu.edu.cn", AppAccess::Webvpn), // 教学质量保障系统
    ("cwbx.cwxu.edu.cn", AppAccess::Webvpn), // 财务系统
    ("tsgcnki.cwxu.edu.cn", AppAccess::Webvpn), // 中国知网校园镜像
    ("tsgieee.cwxu.edu.cn", AppAccess::Webvpn), // IEEE
    ("tsgscid.cwxu.edu.cn", AppAccess::Webvpn), // ScienceDirect
    ("tsgwebof.cwxu.edu.cn", AppAccess::Webvpn), // SCIE
    ("tsgkjyy.cwxu.edu.cn", AppAccess::Webvpn), // 图书馆空间管理
    // ---- C 类 · 实测异常 ----
    ("lw.cwxu.edu.cn", AppAccess::Unavailable), // 毕业论文管理系统：SSO 断，停自家登录页
    ("cwxu.flyread.com.cn", AppAccess::Unavailable), // 虚拟图书馆：可达但为自有登录（非 CAS）
];

/// 按 appLink 的 host 归类：host 精确匹配或为表项子域（`*.表项`）命中；
/// URL 解析失败 / 无 host / 未命中 → [`AppAccess::External`]（保守默认）。
pub fn classify_app_access(link: &str) -> AppAccess {
    let Ok(url) = Url::parse(link) else {
        return AppAccess::External;
    };
    let Some(host) = url.host_str() else {
        return AppAccess::External;
    };
    let host = host.to_ascii_lowercase();
    for (h, access) in ACCESS_TABLE {
        // strip_suffix 防前缀伪造（evilwhall.cwxu.edu.cn 不命中 whall.cwxu.edu.cn）
        if host == *h || host.strip_suffix(h).is_some_and(|prefix| prefix.ends_with('.')) {
            return *access;
        }
    }
    AppAccess::External
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_host_and_ip_hits() {
        assert_eq!(classify_app_access("https://whall.cwxu.edu.cn/pt/home"), AppAccess::Cas);
        assert_eq!(classify_app_access("http://10.3.100.110/plat/shouyeUser"), AppAccess::Cas);
        assert_eq!(
            classify_app_access("https://tsgcnki.cwxu.edu.cn/webvpn/xxx"),
            AppAccess::Webvpn
        );
        assert_eq!(classify_app_access("https://lw.cwxu.edu.cn/login"), AppAccess::Unavailable);
    }

    #[test]
    fn subdomain_matches_and_prefix_spoof_does_not() {
        // 子域命中：*.表项
        assert_eq!(classify_app_access("https://a.whall.cwxu.edu.cn/"), AppAccess::Cas);
        // 前缀伪造不命中
        assert_eq!(classify_app_access("https://evilwhall.cwxu.edu.cn/"), AppAccess::External);
        assert_eq!(classify_app_access("https://whall.cwxu.edu.cn.example.com/"), AppAccess::External);
    }

    #[test]
    fn miss_falls_back_to_external() {
        // 未实测 host（附录 A 无结论）→ 保守默认
        assert_eq!(classify_app_access("https://sygl.cwxu.edu.cn/"), AppAccess::External);
        assert_eq!(classify_app_access("https://www1.cwxu.edu.cn/"), AppAccess::External);
        // 解析失败 / 无 host / 空串
        assert_eq!(classify_app_access("not a url"), AppAccess::External);
        assert_eq!(classify_app_access("mailto:x@example.com"), AppAccess::External);
        assert_eq!(classify_app_access(""), AppAccess::External);
    }

    /// 「不信任 isCas」的行为契约见 parse.rs 测试 `access_overrides_portal_iscas`
    ///（app_item_from 为 parse.rs 私有 fn，测试放同文件）。

    #[test]
    fn access_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&AppAccess::Webvpn).unwrap(),
            r#""webvpn""#
        );
    }
}
