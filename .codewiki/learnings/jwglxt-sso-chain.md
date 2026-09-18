---
title: 正方教务 SSO 链与课表接口取证（ST 绑定 service / TGT 不落 cookie / 901 会话特征）
type: learning
source_files:
  - "crates/campus-auth/src/jwglxt.rs"
  - "crates/campus-auth/src/cas.rs"
  - "crates/campus-auth/tests/jwglxt_live.rs"
  - "crates/campus-schedule/src/zhengfang.rs"
tags:
  - jwglxt
  - cas
  - sso
  - schedule
  - recon
---

# 正方教务 SSO 链与课表接口取证（ST 绑定 service / TGT 不落 cookie / 901 会话特征）

2026-09-18 真实账号实测（live 测试 `tests/jwglxt_live.rs`，逐跳日志与响应样本均验证），M2.5「课表自动导入」链路前提确认：**CAS 登录 → REST 换票 → 一键 SSO 进教务 → 拉课表 JSON，全链打通**。

## 坑一：ST 与 service 绑定——门户 ticket 直接去教务会 404

`login()` 签发的 ticket 绑定的是门户 service（`PORTAL_SERVICE`）。拿它 GET `https://jwgl.cwxu.edu.cn/sso/lyiotlogin?ticket=ST-…` 返回 **404**（教务端 CAS filter 拒绝），且该 404 响应也会种下 `route`/`JSESSIONID`——空壳会话，后续课表接口一律 901。

**修**：先 `POST {CAS_BASE}/v1/tickets/{tgt}`（form `service=https://jwgl.cwxu.edu.cn/sso/lyiotlogin`）换发**教务 service 的新 ST**（响应体纯文本 `ST-…`，非 JSON），再去走 SSO 链（`jwglxt.rs:25-43`）。service 注册值可复核：无 ticket 访问 `/sso/lyiotlogin` 会 302 到 `lyuapServer/login?service=<同一值>`（自引用）。

## 坑二：CAS 服务端不种任何登录 cookie，TGT 只在响应体里

live 实测 `login()` 成功后 jar 为空（无 `CASTGC` 之类）——CAS 完全无 cookie 态，「会话复用」只能靠客户端保存 `CasLoginOk.tgt` 换新 ST。**上层必须把 tgt 与 jar 一起持久化**，否则教务会话过期后要重新走账密+验证码登录。

**M2.5 批次 1 已落地**：`CasSession.tgt`（内存明文）+ `SessionRecord.tgtB64`（DPAPI 密文，session.json，兼容旧格式缺省）——落盘点收在公共路径 `finish_login`（`login_saved` 免密重登同享）；教务会话失效（901）时 `CasClient::fetch_timetable_json(tgt,…)` 用 TGT 静默重进（`jwglxt_sso` 换新 ST 走 5 跳链）后重试一次，重进失败或仍 901 归一为 `CampusAuthError::JwglNotLogin`。

## SSO 302 链（5 跳，逐跳实测）

```
GET /sso/lyiotlogin?ticket=ST-…            → 302 /sso/lyiotlogin        （种 route+JSESSIONID 空壳）
GET /sso/lyiotlogin                        → 302 /jwglxt/ticketlogin?uid=…&timestamp=…&verify=…
GET /jwglxt/ticketlogin?uid=…              → 302 http://…/xtgl/login_slogin.html  （种教务正式 JSESSIONID+rememberMe）
GET /xtgl/login_slogin.html                → 302 /xtgl/index_initMenu.html?jsdm=xs&_t=…&echarts=1
GET /xtgl/index_initMenu.html              → 200 「教学管理信息服务平台」（jsdm=xs 学生身份）
```

中间跳 Location 是 http:// 明文，需 https 升级——`sso_follow` 已内置该逻辑（含 body 中断容忍），`jwglxt_sso` 直接复用。链中 `ticketlogin?uid=…` 的 uid 是学号（PII），打日志要打码。

## 会话无效的两种返回形态（901 判据）

课表接口 `POST /jwglxt/kbcx/xskbcx_cxXsKb.html?gnmkdm=N2151`，body `xnm=<学年起始年>&xqm=<学期>`：

| 状态 | 请求形态 | 语义 |
|---|---|---|
| **HTTP 901**（自定义码，reqwest 不识别、`as_u16()==901`）+ 空 body、无 Content-Type | 带 `X-Requested-With: XMLHttpRequest` | 会话无效（ajax 形态）→ 上层按「未登录」降级的信号 |
| 200 + `text/html` 21KB 登录页（title=教学管理信息服务平台） | 不带 XRW | 同样会话无效，回登录页 HTML |
| 200 + `application/json`，含 `kbList` | 有效教务会话 | 正常数据 |

**已登录后 `X-Requested-With` 与 `Referer` 都非必需**（实测仅 Content-Type 也能 200 JSON）——但建议上层仍带 XRW，让会话失效表现为明确的 901 而非 200 登录页。`gnmkdm=N2151` 在 **query**（实测有效）；教务会话与门户会话相互独立（jwgl 域 cookie 单独种）。

## kbList 字段核验（与 zhengfang.rs 映射一致）

学生课表实测 8 条目（23.8KB），字段全部存在且形态与解析器约定一致：`kcmc`/`cdmc`/`jxbmc`/`jxb_id`(32 位 hex)/`xqj`("1".."7")/`jcs`("3-4")/`oldzc`(十进制位掩码，4095=1-12 周)/`xm`(教师)/`kcxz`/`khfsmc`/`kczxs`；另有 `jc`("3-4节")/`jcor`/`oldjc` 等冗余字段未用。顶层还有 `xsxx`（XH 学号/XM 姓名/BJMC 班级/XNM/XQM 等）。参数语义实测：`xqm=12`（本学年第 2 学期）返回**空 kbList 的 200**（不是错误）；`xnm=2025`（上一学年）返回 36KB 历史课表——历史学年可拉。

## xnm/xqm 推导口径（推荐）

用门户 `query_semester_info()`（`campus-portal`）：`xnm = SemesterInfo.start_date 前 4 位`（实测 "20260907"→2026 ✓）；`xqm = SemesterInfo.semester` 映射（"1"→3、"2"→12；暑期 16 无来源字段）。**不要用 `grade` 字段**——其语义是入学年级（对老学生≠学年起始年），仅新生恰好与 xnm 同值，未对老学生账号复核。

## 未验证项

- 教务 JSESSIONID 的服务端过期时长（无法瞬时实测）；过期后表现应同 901（推测，未验证）。
- 教师端点：按正方命名规律应为 `kbcx/jskbcx_cxJsKb.html`（xs→js），无教师账号，未验证。
- `Semester::Summer`（xqm=16）与转专业/无课学生等边界。

## 复跑

```bash
CAMPUS_HUB_CREDS=<凭据文件> cargo test -p campus-auth -- --ignored jwglxt_sso_kbcx_live --nocapture
```

## 相关

- [[modules/campus-auth|CAS 协议核心]]
- [[modules/campus-schedule|课表领域核心]]
- [[learnings/cas-sso-plaintext-redirect|CAS SSO 回跳三坑]]
