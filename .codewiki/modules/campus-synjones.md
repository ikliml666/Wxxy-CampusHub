---
title: 慧新E校协议核心（campus-synjones）
type: module
source_files:
  - crates/campus-synjones/src/lib.rs
  - crates/campus-synjones/src/sso.rs
  - crates/campus-synjones/src/client.rs
  - crates/campus-synjones/src/ecard.rs
  - crates/campus-synjones/src/charge.rs
  - crates/campus-synjones/tests/synjones_live.rs
  - tauri-app/src-tauri/src/commands/synjones.rs
  - tauri-app/src-tauri/src/commands/electricity.rs
tags:
  - synjones
  - ecard
  - electricity
  - sso
  - recon
  - m3
---

# 慧新E校协议核心（campus-synjones）

`crates/campus-synjones`：校内慧新E校平台（`http://10.3.100.110`，哈尔滨新中中/synjones）的**协议单点**，供 M3 的「钱包（一卡通）」与「电费」两条业务使用。分层与 [[modules/campus-portal|门户协议核心]] 对称：一个外部系统一个 crate，鉴权与信封不与会话/门户混用。命令层落在 `commands/synjones.rs`（一卡通）与 `commands/electricity.rs`（电费 + 充值窗口），只做「锁内 clone → 调 crate → 映射 `CommandResult`」。

## 一、鉴权：四件必须记住的事

1. **来源参数与请求头双份携带**（`client.rs` 的 `with_headers`）：`synAccessSource=app` 既放 GET 的 query / POST 的 form body，**又**加同名请求头——这是官方 axios 拦截器的真实行为，照抄最稳。值域来自官方 `sessionStorage.agentType`，本客户端固定 `app`。
2. **`synjones-auth: bearer <token>`**，token 类型缺省 `bearer`。
3. **4030 = HTTP 401 + `body.code == 4030`**（不是 HTTP 403）。学校服务端按来源授权，`pc` 被拒、`app` 放行——这是 [[decisions/...|已知临时措施]] 级别的平台缺陷，见下「坑二」。
4. **三套信封不可共用解析器**（`client.rs` 的 `Envelope`）：berserker 系 `{code,success,data,msg}`、charge 系 `{code,message}`（401 时 message 可能为空串）、search 系 `{code,data,msg}`；成功判据统一 `code == 200`。

## 二、SSO 桥：CAS TGT → 慧新E校 token（`sso.rs`）

实测链路是 **2 跳**，token 从**落点 URL 的 query** 取（无 `Set-Cookie` 参与）：

```
GET {BASE}/berserker-auth/cas/login/lyCas?targetUrl=<enc>&ticket=ST-…  → 302 {BASE}/campus-card-pc/?synjones-auth=<raw token>
```

- **`targetUrl` 是硬性前提**（四组对照实测）：`/campus-card-pc/`、`/charge-pc/pays/450` 这类**子应用路径**才带 token；`/plat/shouyeUser` 与不传 targetUrl 都**不回 token**。默认值取 `DEFAULT_TARGET_PATH = "/campus-card-pc/"`（一卡通子应用正是 token 的消费者，且路径不含 feeitemid、不会随片区调整失效）。浏览器入口 `cas/redirect/lyCas` 只是给人用的，换 ST 要对准 `cas/login/lyCas`——与 [[learnings/jwglxt-sso-chain|教务 SSO]] 同构。
- **token 是「单活」的**：同一账号只有**最新一次** SSO 签发的 token 有效，早签发的在后续 SSO 之后调业务接口即 401。所以 `SynjonesClient` 必须**全应用单一实例**（见「坑一」），且**禁止并发 SSO**。
- **CAS TGT 会隔夜过期**：过期 TGT 换票得 HTTP 500 且响应正文含票据 —— **禁止回显/落盘该正文**，只记类别与长度。过期时需回落「已存账号 + 验证码自动识别」重登路径。

## 三、业务接口事实

### 一卡通（`ecard.rs`，全部需 token）

- 卡信息 `GET /berserker-app/ykt/tsm/queryCurrentCard`（无参）/ `queryCard?account=` / `?scene=recharge`；`data.card[]`。
- **余额口径（易错点）**：`elec_accamt`（单位**分**）是**电子账户**余额（`elec` = electronic），不是电费；卡账户是 `(db_balance + unsettle_amount)`。两者可差很远（实测样本里一个非零、另一个为 0），**UI 以电子账户为主、卡账户次级展示**（`WalletPanel`）。
- `cardname` 实测**可能是空串** → 回落链 `cardname → card_name → cardtype`（`ecard.rs`）。
- 消费流水在**独立服务** `GET /berserker-search/search/personal/turnover`（`size`/`current`/`account`/`type`；信封用 `Envelope::Search`）→ `{code,data:{total,records[]}}`。**`type` 是收支方向**：`1`=收入、`2`=支出、`3`=空；**不传 `type` 即全量**（实测全量 = 收入 + 支出总数）。记录字段：`jndatetimeStr`、`resume`、`tranamt`（分）、`typeFrom`（`"1"` 为收入）、`cardBalance`（交易后余额快照，分）、`locationName`、`payName`、`consumeTypeName`。
- 「今日/本月消费」**无解**：`statistics/turnover/sum/user` 需五参、多种参数组合实测全部返回空 `data`；`statistics/turnover/count` 给的是**全时段**总额且无视日期参数。故 UI 显示「暂不可用」，不要用 count 冒充。

### 电费（`charge.rs`）

三级链路：`GET /charge/feeitem`（**唯一匿名可读**，免登录也能列片区）→ `GET /charge/feeitem/singleFeeitem?feeitemid=`（需 token）→ `POST /charge/feeitem/getThirdData`（form，需 token）。

- **片区口径 = `status == 1 && impl_interface` 非空**：`/charge/feeitem` 返回 8 条、其中 `status==1` 有 6 条，但另 3 条是补卡/充值/扫码类（无 `impl_interface`），电费口径过滤后**恰好 3 条**（梅园/李园/桃园）。同名停用项（如 428）必须靠 `status` 排除。
- **级联**：首次 `{feeitemid, type:"select", level:0}`，逐级带上已选值、`level` 递增；层级由服务端 `map.total[]` 给出（`campus → building → room`）。
- **末级是「输入级」不是下拉**：三片区 `flag[4]=='3'`（`last_level_is_input`），`level` 到最后一级时 `map.data` 返回**空**，房间号由用户输入（官方语义「先选择再输入」）。房间号格式要求严格：`101` 命中，`1-101` / `101室` 之类会得到 `tipinfo`「缴费系统返回数据错误child==NULL！」。
- **结果字段是单键自由文本**：`map.showData` 的键名**恒为 `信息`**，值是各片区**格式互不相同**的自由文本（有的含「剩余金额 + 单价」，有的含「余额 + 剩余电量」，有的只有「剩余电费」）；`map.money` / `map.iectranamt` 实测**都不存在**。所以 `ElectricityView.fields` 做**通用字典渲染**、前端按逗号折行、负数标红——**绝不做文本解构**（三片区格式各异，解构会随文案漂移静默失效）。
- `map.data`（末级对象）含户号等 PII：**不透出、不入日志**。

## 四、两个必须记住的坑

**坑一：token 单活 ⇒ 必须单实例缓存。** `commands/synjones.rs` 用**进程级 `static` + `tokio::sync::MutexGuard`** 把请求串行化，并把会话账号/TGT 变化作为重建条件；`commands/electricity.rs` 复用同一实例（为此把 `synjones_session` 等改成 `pub(crate)`，而**不是**另起第二套客户端——另起一套会互相顶掉 token）。

**坑二：官方 `charge-pc` 页硬编码 `pc` 来源 ⇒ 内嵌充值页会弹「服务大厅未授权」。** 我们注入的 `agentType=app` 只能影响**读该键**的请求；官方页部分请求写死 `synAccessSource=pc`，撞上 4030 策略（真机点验确认：弹「提示 服务大厅未授权(1) 确定」）。处置：`RECHARGE_4030_HOOK` 常量（`electricity.rs:318-449`）由 `init_script` **排在整个注入脚本的第一段**——`initialization_script` 先于官方页脚本执行，故 hook 先于官方 axios 拦截器生效，覆盖**三种携带位置**：① URL query（`XHR.open` / `fetch` 的字符串形态）；② 请求头（`setRequestHeader` / `fetch` 的 `init.headers` 三种形态 / `Request` 实例就地改写——URL 需改写时用 `new Request(url, opt)` 重建，`duplex:'half'`，失败退回原对象绝不阻断请求）；③ 请求体（urlencoded 串 / `URLSearchParams` / `FormData`）。纪律：**只改不增**（官方没带该参数的请求保持原样，不给它加参数）、**只改这一个键**（authorization 等不动）、**幂等**（单次安装标记）、全程 try/catch（hook 异常不得破坏 token/configs 注入而致白屏）。来源值经占位符 `__SYN_ACCESS_SOURCE__` 在 Rust 侧替换为 crate 常量，避免 JS 里重复硬编码。**这是绕开学校服务端授权缺陷的临时措施**，学校修复 PC 授权后整段可移除；单测 `init_script_puts_4030_hook_first_and_covers_three_carriers`（`electricity.rs:769-797`）钉住「排最前 + 三处覆盖 + 不做缺失追加」。同目的的参考实现是用户自写的油猴脚本 `fix-4030.user.js`。

## 五、内嵌充值窗口（`commands/electricity.rs`）

`open_recharge_page` 用 `tauri::WebviewWindowBuilder` 打开 `{BASE}/charge-pc/pays/{feeitemid}?synjones-auth=<token>`，注入（token 只在 Rust 内存里拼进脚本，**不经过前端 JS API**）：

- `sessionStorage.access_token`（裸 token，无 `bearer ` 前缀）/ `token_type` / `agentType='app'`
- `localStorage.configs`（键 `title`/`version`/`base`）——**缺失会让官方页 `JSON.parse(null)` 白屏**，必须预注入
- 上述 `synAccessSource` 改写 hook

**安全边界（已真机验证）**：官方页所在窗口不被任何 capability 覆盖 ⇒ 其 JS 虽然能拿到 `__TAURI_INTERNALS__`（Tauri 内部对象，无法隐藏），但**调用会被 ACL 拒绝**（实测 `list_feeitems not allowed. Plugin not found`），因此**不需要**为它改 `capabilities/default.json`。Rust 端 `WebviewWindowBuilder::build()` 也不受前端 ACL 约束——`core:webview:allow-create-webview-window` 只管控前端 JS 发起的建窗命令。

相关：[[modules/campus-auth|CAS 登录协议]]、[[modules/campus-portal|门户协议核心]]、[[learnings/portal-session-expiry-200-envelope|门户失效是 200 信封]]、[[learnings/tauri-webview-ui-verification|Tauri 真机 UI 验收方法]]。
