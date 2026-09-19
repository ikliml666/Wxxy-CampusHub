---
title: 慧新E校协议核心（campus-synjones）
type: module
source_files:
  - crates/campus-synjones/src/lib.rs
  - crates/campus-synjones/src/sso.rs
  - crates/campus-synjones/src/client.rs
  - crates/campus-synjones/src/ecard.rs
  - crates/campus-synjones/src/charge.rs
  - crates/campus-synjones/src/recharge.rs
  - crates/campus-synjones/src/turnover.rs
  - crates/campus-synjones/tests/m4_history_probe_live.rs
  - tauri-app/frontend/src/components/RechargeFlow.tsx
  - crates/campus-synjones/tests/synjones_live.rs
  - tauri-app/src-tauri/src/commands/synjones.rs
  - tauri-app/src-tauri/src/commands/electricity.rs
  - tauri-app/src-tauri/src/commands/electricity_history.rs
tags:
  - synjones
  - ecard
  - electricity
  - sso
  - recon
  - m3
  - history
  - m4
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
- **结果字段是单键自由文本**：`map.showData` 的键名**恒为 `信息`**，值是各片区**格式互不相同**的自由文本（有的含「剩余金额 + 单价」，有的含「余额 + 剩余电量」，有的只有「剩余电费」）；`map.money` / `map.iectranamt` 实测**都不存在**（恒 `null`）。所以 `ElectricityView.fields` 做**通用字典渲染**、前端按逗号折行、负数标红——**绝不做文本解构**（三片区格式各异，解构会随文案漂移静默失效）。
- **结构化余额 `ElectricityView.balance_yuan`（元，2026-09-19 批 2 补的契约口）**：末级视图除 `fields` 外**必须**带一个后端提取好的数值——`final_query` 构造视图时对 `fields` 调一次 `balance_from_fields`，结果放进该字段（serde ⇒ 前端 `balanceYuan`）。**前端结果卡主数字与趋势/统计一律读它，不许自己解析 `fields` 文本**（解析实现只有 `charge` 一处，复制到前端会随校方文案漂移）。语义：`None` = 未提取到（UI 显示「无数据」），**绝不用 0 代替**（`0.00` 是合法余额）。桌面端自采快照（`commands/electricity_history.rs::build_entry`）同样**直接透传**该字段、不重算。
- `map.data`（末级对象）含户号等 PII：**不透出、不入日志**。
- **余额提取（M4，`charge::balance_from_text` / `balance_from_fields`，结果进 `ElectricityView.balance_yuan`）**：既然不做文本解构，余额就从那句自由文本里按**严格形态**（`关键词 + 分隔符 + 数字`，只认相邻、不跨字段拼接、不做位置解构）提取，提不到返回 `None`（UI 显示「无数据」，**绝不臆造 0**）。关键词表刻意**排除**「剩余电量/电量」——`448` 的原文「当前余额517.05元,**当前剩余电量957.50度**」里两者同句，把 kWh 当钱是错报；千分位（`1,234.56`）歧义时也宁可 `None`。单测用三片区 live 原文钉住，并有一条**在视图构造路径上**（`final_query`）断言三条原文的取数与 camelCase 键名。

### 缴费历史与订单（`turnover.rs`，M4 批 1；端点 2026-09-19 live 实测）

**只读纪律**：该模块**只发 GET**（下方五条），禁止出现建单/支付/删单/退款/绑卡（写路径全在 `recharge.rs`）。

| 用途 | 端点 | 参数 | 关键字段 |
|---|---|---|---|
| 缴费账单（历史） | `GET /charge/turnover/app_account` | `current`/`size`（`size` 上限收到 100）/可选 `feeitemid` | 顶层 `count`（**全量条数**，非本页）+ `accountList[]`：`SUCCESSDATE`/`TRANAMT`/`TURNOVERID`/`ITEMNAME`/`ABSTRACTS`/`TYPENAME` |
| 某月合计 | `GET /charge/turnover/pie_account` | `createdate=YYYY-MM`（**参数生效**，可逐月拼曲线） | `pieAccountList[].tranamt`；**无数据的月回空数组** ⇒ 该月 `0.0` |
| 累计缴费额 | `GET /charge/turnover/app_totalAccount` | 可选 `feeitemid` | `accountTotal`（**可能 null** ⇒ `None`） |
| 订单（**含待支付**） | `GET /charge/order/personal_data` | 可选 `status`（0 待支付 / 1 已完成 / 2） | `orderList[]`：`orderid`/`tranamt`/`actulamt`/`status`/`commitdate`/`successdate`/`source`/`abstracts`/`feeitemlist[0].feeitemid`（顶层 `feeitemid` 恒 0 是占位） |
| 片区配置 | `GET /charge/feeitem/showFeeitem` | `feeitemid` | `list[0]`：`price`/`maxmoney`/`daymaxmoney`/`retain_money`/`billing_unit`/`layout` |

**⚠️ 单位红线（元 vs 分，最易错）**：`/charge/*` 侧 `TRANAMT` / `tranamt` / `accountTotal` / `pieAccountList[].tranamt` 实测单位是**元**（同一时刻一卡通流水扣 `tranamt=100`（分）而电费账单是 `TRANAMT=1`）⇒ **不要照抄官方 PC 页对 `TRANAMT` 的 `/100`**；而一卡通侧（`ecard.rs`）一切金额字段是**分**，由该模块的 `yuan()` 换算。跨源聚合（首页钱包卡）时两者口径不可混用。

**⚠️ 旧结论已修订**：项目早前记录「`/charge/order/personal_data?status=0` 任何形态恒 500 ⇒ 学校侧没有可用的待支付订单列表接口」（见 [[modules/campus-synjones|本文件]] §五 与 CHANGELOG M3.1 条目）。M4 探针实测：**路径与参数本就正确，缺的是 App 口径请求头组**——`synAccessSource=app` 必须**同时**进 query 与同名头（正是 `client.rs` 的 `with_headers` 行为），旧形态（只有来源头、无 query 一份）复跑同样 500。故「进充值前检查遗留订单」这条防线**技术上已恢复**（M4 批 2 由 `get_electricity_orders` 提供列表，取消复用 `recharge_cancel`）。

**其它实测事实**：`app_account` 的 `balance_amount` **恒 null**、`mouthAccount` 只给本月合计（参数被忽略）、`threeExpen_account` 恒空 ⇒ **宿舍电费的日余额序列在学校侧不存在**，只能客户端自采（见 [[decisions/electricity-daily-snapshot-and-merge|电费日快照与多端合并决策]]）。`balance_from_text` 由此成为唯一能拿到「某个瞬间的余额数字」的路径。


## 四、必须记住的坑

**坑一：token 单活 ⇒ 必须单实例缓存。** `commands/synjones.rs` 用**进程级 `static` + `tokio::sync::MutexGuard`** 把请求串行化，并把会话账号/TGT 变化作为重建条件；`commands/electricity.rs` 复用同一实例（为此把 `synjones_session` 等改成 `pub(crate)`，而**不是**另起第二套客户端——另起一套会互相顶掉 token）。

**坑二：内嵌官方缴费页那条路已废弃**（2026-09-19 用户裁决改变方向）。曾经的方案是用 `WebviewWindowBuilder` 打开官方 `charge-pc` 页面并注入 token/`localStorage.configs`，为此还写了绕开学校 4030 授权缺陷的 `synAccessSource` 改写 hook。**该方案连同 hook 已整体移除**（原因：官方 PC 页面自己硬编码 `pc` 来源会弹「服务大厅未授权」，且更根本地——PC 下单链路 `target="_self"` 整页跳走、客户端拿不到扣款结果）。现改为**客户端直调官方 App 版接口**，见下节。仅保留 `open_recharge_in_browser` 作为「去官网充值」兜底。

## 五、充值：客户端直调官方 App 口径（2026-09-19 用户裁决，替换原内嵌页面方案）

**为什么不用 PC 链路**：官方 `/charge/order/thirdOrder` 需 SHA256 签名（`APP_ID=56321` + 公开 `SECRET_KEY`）且 `target="_self"` **整页跳走**——客户端拿不到扣款结果。**App 版（`/charge-app`）是纯 JSON**：下单回 `orderid`、可轮询 `order.status`、可 `deleteOrder`。实现落在 `crates/campus-synjones/src/recharge.rs`（常量见 `:67-71`），命令 `recharge_*` 见 `commands/electricity.rs:355` 起，前端流程在 `components/RechargeFlow.tsx`。

**支付状态机（照此实现）**
① 建单 `POST /blade-pay/pay`（form：`feeitemid, tranamt, flag="choose", source="app", paystep:0[, third_party]`）→ `data.orderid`；
② `GET /charge/pay/getpayinfo?orderid=` → **顶层 `{order, payList}`**——`payList` 的来源是这里而**不是** `paystep`（最易误判处）；
③ 需密码时 `POST /blade-pay/pay`（`paystep:2` + `paytype/paytypeid`，**分两步**：不带 `accountno` 只回账号列表，带上才回 `ccctype` + `passwordMap`）；
④ 提交（`paystep:2` + `accountno/ccctype`；免密时无 password，需密码时带 `password`+`uuid`）；
⑤ 轮询 `getpayinfo` 的 `order.status`（0 待支付 / 1 已完成）；⑥ 清理 `POST /charge/order/deleteOrder`。

**免密判据**：`payList[i].nopassword === 1`。实测 450 片区唯一渠道是 `ACCOUNTTSM`（电子账户）且 `nopassword=false` ⇒ **该片区一律需要密码**（免密分支保留给其它片区）。

**安全红线（实现必须遵守）**：服务端下发的 `passwordMap[uuid]` 是「10 个字符的**显示**序列」，而提交的 `password` 是**用户点击的键位下标序列**（位置编码）⇒ **客户端不需要接触真实密码**；但客户端持有解码表，**只转发、绝不还原、绝不落盘/打日志/回填输入框**（把显示字符当密码提交 = 明文泄露用户密码）。

**四个学校侧的坑（均实测）**
1. `passwordMap[uuid]` 是 **10 字符字符串**（不是数组），官方前端逐字符渲染——解析必须兼容两种形态（首轮「拿不到键盘」的根因，`recharge.rs` 的 `parse_password_pad`）。
2. `POST /charge/order/deleteOrder` **只有 JSON body 才回 200**，form/query/GET 恒 500（crate 内自带 `json_post`，`recharge.rs:606`）。
3. 建单早期有「日消费上限」校验，值取自片区配置 `daymaxmoney`（448/449/450 均为 500）；费用项 id 不存在时该值取到 null → 服务端 NPE 报 `dayTotalMoney-日消费最大金额判断异常了-null`（**不是缺参数**）。
4. `third_party`（电费专用上下文串）= `JSON.stringify(末级 map.data)` + `myCustomInfo="<末级名>：<各级名 空格>"`；**不缀 `-ids-金额`**（那只在官方「选中应收项」形态出现，单房间充值无该分支）。因 `map.data` 含户号等 PII，**合成一律在后端**（`third_party_for_room`，`recharge.rs:359`），不下发前端。

**进入充值前检查遗留订单：已恢复（M4 批 1 更正）**。M3.1 时记录「无法实现」，根因不是端点不可用而是 App 口径头组缺失（`synAccessSource=app` 要同时进 query 与同名头），M4 探针实测 `status=0` 正常回 `orderList`。M4 批 2 的 `get_electricity_orders(status=0)` 已能列出待支付单（含当时遗留的那笔 1 元单），取消直接复用 `commands::electricity::recharge_cancel`（不新增写路径）。

**未验证项**：`submit_pay` 的成功路径只能由**用户真机试充**验证（红线：开发/点验阶段绝不提交支付）；多应收项场景的 `-ids-金额` 后缀未实现（单房间流程用不到）。

相关：[[modules/campus-auth|CAS 登录协议]]、[[modules/campus-portal|门户协议核心]]、[[learnings/portal-session-expiry-200-envelope|门户失效是 200 信封]]、[[learnings/tauri-webview-ui-verification|Tauri 真机 UI 验收方法]]、[[learnings/synjones-charge-yuan-vs-fen-and-pending-orders|元/分口径与待支付单旧结论修订]]、[[decisions/electricity-daily-snapshot-and-merge|电费日快照与多端合并决策]]。
