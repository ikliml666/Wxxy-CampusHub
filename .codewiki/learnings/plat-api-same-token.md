---
title: plat 体系鉴权与 API 清单（token 同源直调）
type: learning
source_files:
  - crates/campus-synjones/src/plat.rs
  - crates/campus-synjones/tests/plat_sso_probe_live.rs
  - crates/campus-synjones/src/client.rs
tags:
  - ecard
  - plat
  - auth
  - recon
---

# plat 体系鉴权与 API 清单（2026-09-20 探针实证）

## 鉴权结论：与我们现有 token 同源，无需第二套会话

官方「移动服务平台」（plat，`/plat/*` 路由：个人中心/付款码/设置）的 API 鉴权头是
**`synjones-auth: bearer <token>`**（不是 `Authorization`）——而我们的
`SynjonesClient` 统一头组（`client.rs::with_headers`）**恰好就是这个头**。探针
（`tests/plat_sso_probe_live.rs`）实证：SSO 桥（PC 落点）签发的 token 直接调 plat
API 全部 200——**服务端不区分 token 签发体系**。

此前担心的「token 单活」按 client 体系分池：官方页面 SSO 签 mobile JWT 不顶我们
PC token 的会话（浏览器实验与应用真机回归均证）。

**plat JWT 的官方获取链（研究性记录，我们不需要）**：plat/login「统一身份认证」
→ CAS 回跳带 ST → 前端 `POST /berserker-auth/oauth/token`（form：
`username=<ST>&password=<ST>&grant_type=password&logintype=sso&device_token=h5` +
`Authorization: Basic bW9iaWxlX3NlcnZpY2VfcGxhdGZvcm06bW9iaWxlX3NlcnZpY2VfcGxhdGZvcm1fc2VjcmV0`
= `mobile_service_platform:mobile_service_platform_secret`）→ JWT。**我们探针复刻
该链返回 400「业务异常」**（service 绑定或其余细节不符，四个候选 service 均拒）——
无需继续追：现有 token 已够用。

## plat API 清单（实测）

| 接口 | 用途 | 关键响应 |
|---|---|---|
| `GET /berserker-base/user` | 个人资料 | `account/sno/name/departmentName/identityName/sex/avatar/mobile/idNumber(已掩码)` |
| `GET /berserker-base/equipment/searchUserBindEquipment?status=` | 设备管理 | `status="1"` 已登录在线（可下线）/`"0"` 已授权设备；条目 `id/name/type/createTime/updateTime` |
| `GET /berserker-base/logs/login/user?current=&size=` | 登录日志 | MP 分页 `{records,total,size,current,pages}`（本校恒空） |
| `GET /berserker-app/ykt/tsm/getUserOfflienSwitch` | 脱机二维码开关 | `data.userOfflienSwitch`："1"/"0" |
| `GET /berserker-app/ykt/tsm/codebarPayinfo` | 付款码支付信息 | 支付方式数组：`account/payacc("000")/name/code("ACCOUNT")/elec_accamt(分)/bandacc(绑定卡全号!)` |
| `GET /berserker-app/ykt/tsm/batchGetBarCodeGet?account=&payacc=&paytype=` | **条码+二维码生成** | `data.{retcode:"0", expires(秒), barcode:[<20 位串>×10]}` |
| `berserker-app/ykt/tsm/offlienPar` | 脱机码参数（paycode chunk 引用） | 未实测 |
| `berserker-app/ykt/tsm/barcodeDel` | 删旧条码（刷新时） | 未实测 |
| `POST /berserker-auth/login/verificationCode`（base/login） | 短信验证码 | 安全设置绑手机/改密码用 |
| `berserker-app/orderPay/getOrderPayinfoStatus` / `updateCodebarPayinfoStatus` | 付款顺序查/写 | 未实测参数 |
| `berserker-app/ykt/tsm/updateCodebarPayinfo` | 条码支付信息更新 | 未实测参数 |

## 实现约定

- crate 层只读面：`plat.rs`（user_profile / equipment / login_logs / offline_switch），
  命令层 `commands/ecard.rs` 的 `get_plat_*` 四条。
- PII：`bandacc` 是**绑定银行卡全号**——付款码相关 DTO 若透出必须脱敏或剔除；
  user_profile 的 `idNumber` 服务端已掩码可透传（本人查看本人资料）。
- 写操作（下线设备/解绑校园卡/改手机/改密码/付款顺序调整/脱机开关切换）一律由用户
  显式触发，不在只读面顺路提供。

## 相关

- [[learnings/ecard-keyboard-pseudochar-protocol|安全键盘伪字符映射协议]]
- [[decisions/ecard-transfer-removed|账户转账删除决策]] — 官方全站扫描结论
- [[modules/campus-synjones|慧新E校协议核心]] — SSO 链与 token 单活
