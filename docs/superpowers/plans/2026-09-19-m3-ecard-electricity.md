# 计划：M3 一卡通 + 电费（慧新E校直连）+ M2 遗留补做

> 2026-09-19（会话 `sess-36e5d8e0`）· 分支 `feat/sess-36e5d8e0-m3-card-electricity`
> 用户授权：「M2 部分内容没完成，同时开始 M3，你负责规划，让智能体执行」；同日补充三句裁决：① 顶栏命令面板**要彻底解决**；② **首页与一卡通都用实时余额**；③ 充值**用官方链接但在软件内部跳转（内嵌），无需到浏览器，同时统一显示风格**。
> 范围 = `PLAN.md` §M3 + M2 遗留（命令面板、资讯报错修复）。**不碰课表（M2.5 归会话 `sess-1e38d0e8`）**。
> **For agentic workers:** REQUIRED SUB-SKILL: 用 `superpowers:subagent-driven-development` 逐任务执行；步骤用 `- [ ]` 跟踪。

**Goal:** 钱包页与首页的一卡通余额都来自慧新E校实时接口（带门户降级），电费页按「片区→校区→楼栋→房间」查到剩余金额与单价并记住常用房间，充值在应用内 webview 打开官方页面并保持与客户端一致的观感；同期修掉「各页资讯获取失败」的真实故障，并补做 M2 漏掉的顶栏命令面板。

**Architecture:** 新建 `crates/campus-synjones`（慧新E校协议单点，仿 `campus-portal` 与 `campus-auth/jwglxt.rs` 范式）：`sso.rs` 用 CAS TGT 走 lyCas 桥换 token，`client.rs` 持 token 并统一注入头组与三套信封解析，`ecard.rs` / `charge.rs` 分业务。Tauri 命令层只做「锁内 clone → 调 crate → 映射 `CommandResult`」。跨源聚合（首页钱包卡）放后端做，前端不写降级逻辑。

**Tech Stack:** Rust（reqwest + serde + tokio）、Tauri 2（命令面 + `WebviewWindowBuilder` 内嵌页）、React 19/TS + zustand。

**Spec:** `PLAN.md` §M3（第 115-124 行）+ 本文件 §1（2026-09-19 实测接口事实，协议细节以 §1 为准）。

## Global Constraints

- 行文与注释中文；命令/报错原文照抄。
- **凭据红线**：token／账号／密码只在内存，**绝不落盘、绝不进日志、绝不进文档/派单 prompt、绝不返回给前端**。内嵌 webview 的 token 由 Rust 端直接写进 `initialization_script`，不经过前端 JS API。live 样本只写仓库外 recon 目录（`%TEMP%/campushub-m3-recon`）。
- 一切慧新E校请求**双份携带** `synAccessSource=app`（GET 走 query、POST 走 body，且都加同名头）。
- **单 token 缓存**：token 是「单活」的（§1.3），全应用共用一个 `SynjonesClient` 实例，**禁止并发多次 SSO**；SSO 失败/401 后的重进必须串行。
- **票据不回显**：TGT 过期换票失败时服务端正文含票据，**任何日志/错误文案/样本都不得包含该正文**（只记类别与长度）。
- 网络超时 10s；错误归一为 crate 的错误枚举，不把 reqwest 错误冒到前端。
- 新增依赖需在交付说明里写理由；优先 std/现有依赖。
- 每批完成后由主智能体验收 + 合并（`bash ~/.zcode/scripts/git-merge-push.sh <worktree 目录>`），分身不合并、不 push、不碰其他会话的分支。
- 同一 worktree 内多分身并行时**文件范围必须互不重叠**（见各批 Files）。

---

## 一、侦察事实（2026-09-19 实测，除标注外均可匿名复核）

### 1.1 服务拓扑（**命名不可泛化**）

| 服务前缀 | 用途 | 证据 |
|---|---|---|
| `/berserker-auth/*` | 认证（oauth/token、cas 桥） | 302 实测 |
| `/berserker-app/*` | 一卡通（`ykt/tsm/*`）、`appScheme/info` | 401 实测 |
| `/berserker-search/*` | **一卡通流水/统计/标签**（本轮新发现） | bundle API 表 |
| `/charge/*` | 电费（feeitem 目录/详情/级联） | 401 实测 |
| `/charge-pc/*` | 电费前端 SPA 壳（**不是接口**） | 返回 HTML |

⚠️ `/berserker-acc/` 实测 **404**。

### 1.2 鉴权四件事

1. token 头：`synjones-auth: bearer <token>`。
2. 来源参数：值来自前端 `sessionStorage.agentType`（本项目固定 `app`），GET/DELETE 放 query、POST 放 body，**且所有方法都再加同名请求头**（官方拦截器行为）。
3. **4030 = HTTP 401 + `body.code==4030`**（不是 403）。
4. **三套信封不可共用解析器**：berserker `{code,success,data,msg}`、charge `{code,message}`（401 时 message 可能为空串）、search `{code,data,msg}`。

### 1.3 SSO 桥（CAS → 慧新E校）——批 1 已 live 打通并固化

浏览器入口 `GET /berserker-auth/cas/redirect/lyCas?targetUrl=<目标>` → 302 到 CAS；但**换 ST 要对准** `{BASE}/berserker-auth/cas/login/lyCas?targetUrl=<编码>`（与 `jwglxt` 同构）。

**实测桥为 2 跳，token 走落点 URL query、无 `Set-Cookie` 参与**：

```
GET  {BASE}/berserker-auth/cas/login/lyCas?targetUrl=<enc>&ticket=ST-…   → 302 {BASE}/campus-card-pc/?synjones-auth=<raw token>
GET  {BASE}/campus-card-pc/                                              → 200（子 SPA 壳）
```

**`targetUrl` 是硬性前提（四组对照，批 1 实测）**：

| targetUrl | 落点 | 是否带 token |
|---|---|---|
| `/campus-card-pc/`（**产品默认**，`sso.rs:55`） | `/campus-card-pc/?synjones-auth=…` | ✅ 验证 200 |
| `/charge-pc/pays/450` | `/charge-pc/pays/450?synjones-auth=…` | ✅ 验证 200 |
| `/plat/shouyeUser` | `/plat/?name=…&ticket=…` | ❌ **不带 token** |
| 无 targetUrl | `/plat/?name=index&ticket=…` | ❌ **不带 token** |

⚠️ **token 是「单活」的**：同一账号同一时刻只有**最新一次 SSO 签发**的 token 有效，早签发的在后续 SSO 之后调业务接口即 401。故产品必须**单 token 缓存**（`SynjonesClient` 已是此形态），**不得并发多次 SSO**。

⚠️ **CAS TGT 会隔夜过期**：过期 TGT 换票返回 `HTTP 500`（正文含票据，**禁止回显/落盘**）→ 需回落「已存账号 + 验证码自动识别」重登路径。

兜底（**不接入产品**，仅探针对照）：`POST /berserker-auth/oauth/token`，`Authorization: Basic bW9iaWxlX3NlcnZpY2VfcGxhdGZvcm06bW9iaWxlX3NlcnZpY2VfcGxhdGZvcm1fc2VjcmV0`，表单 `{username,password,grant_type:"password",scope:"all",loginFrom:"pc",logintype:"username"}` → `{access_token,token_type,flag}`。

### 1.4 一卡通接口（全部需 token）

| 用途 | 方法 | 路径 | 参数 | 返回要点 |
|---|---|---|---|---|
| 当前卡 | GET | `/berserker-app/ykt/tsm/queryCurrentCard` | 无 | `data:{retcode:"0", errmsg, card:[…]}` |
| 卡明细 | GET | `/berserker-app/ykt/tsm/queryCard` | `account=<卡号>` 或 `scene=recharge` | `data.card[]` |
| 多卡列表 | GET | `/berserker-app/ykt/tsm/getCampusCards` | — | `data.card[]` |
| 学校账户 | GET | `/berserker-app/ykt/tsm/getSchoolAccountinfo` | — | 字段待实测 |
| **消费流水** | GET | `/berserker-search/search/personal/turnover` | `size`、`current`、`account`、`type=2` | `{code:200,data:{total,records[]}}` |
| 流水汇总 | GET | `/berserker-search/statistics/turnover/sum/user` | — | 待实测（今日/本月消费） |
| 流水计数 | GET | `/berserker-search/statistics/turnover/count` | — | 待实测 |
| 交易类型字典 | GET | `/berserker-search/search/turnoverType` | — | 待实测 |

**卡对象字段（批 1 实测修订）**：`db_balance` + `unsettle_amount`（**单位分**）→ **卡账户**余额 `(db+unsettle)/100` 元；**`elec_accamt`（分）是一卡通「电子账户」余额（`elec`=electronic），不是电费余额**——实测三处数值完全一致：`elec_accamt = accinfo[0].balance = 流水记录里的 cardBalance`（本机样本 8151 分 = 81.51 元，而卡账户两项皆为 0）。**电费余额不在本接口，只能走 `/charge/*` 级联**（见 §1.5）。其余：`acc_status`（0=正常）、`lostflag`（1=挂失）、`account`、`cardtype`（如 `800#正式卡`）、`accinfo[]{name,type,balance}`（分）。

**流水字段（批 1 实测修订）**：`jndatetimeStr`、`resume`、`tranamt`（**分**，方向由 `typeFrom` 决定：`"1"` 为收入 `+`，否则支出 `-`）、`typeFrom`、**`cardBalance`（该笔交易后的余额快照，分）**、`locationName`、`payName`、`consumeTypeName`、`icon`、`orderId`。

**流水 `type` 参数 = 收支方向（批 1 实测）**：`type=1` → 收入（样本 total=36）、`type=2` → 支出（样本 total=1006）、`type=3` → 0 条。UI 若要"全部流水"，需分别取或用其它取值实测。

**`getSchoolAccountinfo` 无参会返回 `code=400「业务异常」`**，需参数（字段与参数仍待实测）。

### 1.5 电费接口

| 步骤 | 方法 | 路径 | 需 token | 说明 |
|---|---|---|---|---|
| 1 | GET | `/charge/feeitem` | **否** | `{code:200, feeitemList:[…]}`，**唯一匿名可读** |
| 2 | GET | `/charge/feeitem/singleFeeitem?feeitemid=450` | 是 | `{code, feeitem, view, sceneinfo, libraryinfo, tipinfo}` |
| 3 | POST | `/charge/feeitem/getThirdData` | 是 | form：`feeitemid` + `type` + `level` + 各级已选值 |

**feeitem 字段**：`feeitemid`、`name`、`status`（**1=启用、2=停用，必须过滤**）、`billing_unit`、`layout`（快捷金额）、`retain_money`、`maxmoney`、`flag`、`chargeunit`、`pay_productid`、`remark`。启用项 3 条：`450` 梅园1号-梅园3号、`449` 李园9号-李园11号、`448` 桃园1号-李园8号（另有同名停用项 `428`）。

**级联**：首次 `{feeitemid, type:"select", level:0}`；选中第 k 级后带该级参数、`level` 递增；**选中末级（房间）时 `type` 切 `"IEC"`**。响应 `map`：

- `map.total[]`（`{level,code,name,value}` 层级定义）、`map.data`（下一级选项）
- **`map.showData`**：最终展示信息，**键名由服务端动态下发**（前端通用字典遍历）→ 「剩余金额/单价」在里面，**键名必须带 token 实测固化**
- `map.money` / `map.iectranamt`：金额（元）
- `map.tipinfo`：提示文案（存在时 `showData` 置空）
- `thirdOptions.sceneinfo`：平台侧已绑定房间（含 `dataStr`）

**充值（批 3.3 用）**：官方下单**不是 ajax**——先 `SHA256` 签名，再动态创建 HTML form 自动 POST 跳转。**客户端不复刻这条链路**，改为应用内 webview 打开官方充值页（见 §2.6）。

### 1.6 被证伪的旧事实（已在本轮纠正）

`PLAN.md`、`docs/HANDOFF.md`、4030 记忆文件里的「电费 `charge-pc/pays/450` 匿名可用、返回剩余金额与单价」——**错**：那是 Vue SPA 壳 HTML，真接口见 §1.5 且除 feeitem 目录外都需 token。

### 1.7 必须用真实会话确认的未知

| # | 未知 | 确认方法 |
|---|---|---|
| 1 | ~~lyCas 桥落点的 token 传递方式~~ | ✅ **批 1 已解决**：落点 URL query `synjones-auth=<raw token>`、2 跳、无 Set-Cookie；且默认 targetUrl 改 `/campus-card-pc/`（`/plat/shouyeUser` 不带 token） |
| 2 | `map.showData` 的键名（剩余金额/单价） | 批 3 live 测试跑到末级，把键名固化进断言 |
| 3 | ~~流水 `type` 语义~~ / 账单详情接口 / `sum/user` 参数 | ✅ `type` 语义已实测（1=收入、2=支出、3=空，见 §1.4）；`sum/user` 参数与账单详情接口仍待批 2 实测 |
| 4 | 官方充值页在「无 CAS 会话但有 sessionStorage token」下能否工作，以及 `localStorage.configs.base` 的确切形态 | 批 3.3 实测（决定初始化脚本内容） |

---

## 二、设计决策

### 2.1 新建 crate `crates/campus-synjones`

模块：`lib.rs`（`CampusSynjonesError` + 常量 + 重导出）、`sso.rs`（`sso_token`）、`client.rs`（`SynjonesClient` + 头组 + 三套信封）、`ecard.rs`、`charge.rs`。根 `Cargo.toml` 若是 `crates/*` glob 无需改 members。

### 2.2 token 获取：SSO 桥优先

用已有 CAS TGT 走桥，不要求用户额外输入慧新E校密码；桥失败给 `SsoFailed` + 可读文案。业务请求 401/4030 → 用 TGT 静默重进桥一次 → 重试一次 → 仍失败归一 `NotLogin`（沿用 `jwglxt.rs` 失范）。

### 2.3 常用房间：本地存

`sceneBind/add` 是平台侧写接口（改学校数据），不采用。本地存 `SavedRoom { id, feeitem_id, feeitem_name, path: Vec<RoomStep>, label }`，其中 `RoomStep { level: u32, code: String, value: String, name: String }`；点击即重放 `getThirdData` 到末级取余额。落盘位置与格式沿用课表 config 的既有机制（执行者先读 `tauri-app/src-tauri/src/infra/` 找现成的 JSON 落盘 helper，复用而不是新造）。

### 2.4 首页钱包卡与钱包页：**都取实时余额**（用户 2026-09-19 裁决）

新增命令 **`get_wallet_cards`**（后端聚合，前端不写降级）：

```
get_wallet_cards() -> WalletCards {
  ecard:   { value_yuan: f64, source: "realtime" | "portal", updated_at: String },
  mail:    { unread: u32 },      // 仍取门户
  library: { borrowed: u32 },    // 仍取门户
  elec:    { value_yuan: Option<f64>, source: "realtime" | "none" },  // 电费余额：走 /charge/* 级联（**不来自 queryCurrentCard**，见 §1.4 修订）；未绑房间时为 None
}
```

- 一卡通：先试慧新E校 `queryCurrentCard`；失败（校外/桥失败/未登录）**回落到门户 `queryAppointCard`** 并把 `source` 标成 `portal`，前端在卡下显示一行小字「门户快照 / 实时」。
- 邮箱未读与图书借阅：门户是唯一来源，不变。
- 钱包页 `get_ecard` 与首页共用同一后端取数路径（`ecard.rs`），只多带流水与卡列表。

### 2.5 命令面（新增 10 条，`CommandResult` 契约不变）

| 命令 | 入参 | 出参 |
|---|---|---|
| `list_feeitems` | — | `Vec<FeeItem>`（已滤 `status==1`；**免登录**） |
| `query_electricity` | `feeitemId: String`, `path: Vec<RoomStep>` | `ElectricityQuery { options: Vec<Choice{label,value,code,level}>, is_final: bool, view: Option<ElectricityView{fields: Vec<Field{label,value}>, money: Option<f64>, tip: Option<String>}> }` |
| `get_electricity_rooms` | — | `Vec<SavedRoom>` |
| `save_electricity_room` | `room: SavedRoom` | `Vec<SavedRoom>` |
| `delete_electricity_room` | `id: String` | `Vec<SavedRoom>` |
| `open_recharge_page` | `feeitemId: String`, `room: SavedRoom` | `()`（开内嵌 webview，见 §2.6） |
| `get_ecard` | — | `EcardOverview { balance_yuan, elec_accamt_yuan, account, cards: Vec<CardInfo>, today_spend: Option<f64>, month_spend: Option<f64> }` |
| `get_ecard_transactions` | `account: String`, `page: u32` | `Transactions { total: u32, records: Vec<Transaction{time, summary, amount_yuan, is_income, pay_name}> }` |
| `get_wallet_cards` | — | 见 §2.4 |

（共 9 条新增；`CardInfo { account, cardname, balance_yuan, elec_accamt_yuan, status_label }`。）

### 2.6 充值：应用内 webview 打开官方页面（用户 2026-09-19 裁决）

- `open_recharge_page` 在 Rust 端：① 确保有慧新E校 token（走 §2.2）；② `WebviewWindowBuilder` 开一个新窗口（标题「电费充值 · 锡院助手」，约 1000×760，`decorations(true)`，随父窗口关闭）；③ URL = `http://10.3.100.110/charge-pc/pays/{feeitemid}`；④ `initialization_script` 注入会话与配置（**token 只在 Rust 内存 → 脚本字符串，不经过前端**）：
  ```js
  sessionStorage.setItem('access_token', <token>);      // 无 bearer 前缀
  sessionStorage.setItem('token_type', 'bearer');
  sessionStorage.setItem('agentType', 'app');            // ← synAccessSource 取值来源
  localStorage.setItem('configs', JSON.stringify({ base: 'http://10.3.100.110' }));
  ```
  ⚠️ `configs` 的键名与结构、以及官方页是否还需其他 sessionStorage 键，**由批 3.3 实测确定**（读官方 bundle 或抓一次真实加载）。
- **不做**签名下单复刻：用户在官方页面里自己完成支付（真正的钱走学校系统）。
- **统一风格**：窗口标题/图标用客户端品牌；外层包一层我们风格的轻量壳（标题栏 + 「在浏览器打开」兜底按钮）；注入最小 CSS 覆盖官方主色（**属尽力而为，官方改版可能失效，不承诺像素级统一**）。不重写官方页面。
- 安全：新窗口与主应用不同源，注入脚本不给官方页面任何访问 Tauri API 的桥（不开 `withGlobalTauri`/IPC 暴露）。

### 2.7 命令面板（M2 遗留，用户要求彻底解决）

前端独立功能，无后端改动：`Cmd/Ctrl+K` 呼起模态面板，数据源三类——① 面板跳转（复用 `AppShell.tsx:22,33` 的 PANELS 映射与 `DockNav.tsx` 的清单）；② 动作（刷新/导入课表、切换账号、切换主题、退出登录、打开充值页、重新登录等，复用现有 store action）；③ 设置项跳转。要求：键盘上下选择 + Enter 执行 + Esc 关闭、模糊匹配、空态提示、`role="dialog"` + `aria-activedescendant` 可访问性、不与输入框冲突（在 input 内按 Cmd+K 也生效）。删掉 `AppShell.tsx:73,83,91` 的「M2 接入」注释与 `aria-disabled` 占位。

---

## 三、任务分解

### 批 0：修「各页资讯获取失败」（P0，用户报告的真实故障）

**现象**：各界面资讯栏目报「获取登录信息失败: 响应解析失败: tryLoginUserInfo 缺少 userName」；09-18 正常。
**已定位**：`crates/campus-auth/src/cas.rs:417`（`extract_user_profile` 要求 `data.userName` 非空，注释自记「会话失效时响应缺 userName」）；头三元组链路 `crates/campus-portal/src/client.rs:79/134/161`。
**诊断已派**（探针 `crates/campus-portal/tests/portal_diag_live.rs`，只读诊断，未改产品代码）。

- [ ] Step 1：接收诊断结论（根因 + 证据），若结论为「会话失效被误判成解析失败」则按 Step 2 修；若为「门户结构变化」则改解析并补字段回退
- [ ] Step 2：最小修复：会话失效类响应（登录页 HTML / 空 body / `meta` 报未登录 / 缺 `data`）**归一为 `NotLogin`**，前端提示「登录已过期，请重新登录」并触发清会话降级；保留真解析失败为 `Parse`
- [ ] Step 3：离线单测覆盖各分支（登录页 HTML、空 body、缺 data、userName 为 null/空串、正常）
- [ ] Step 4：live 复跑确认资讯页恢复（真机点验）
- [ ] Step 5：提交

### 批 1：synjones crate + SSO 桥（**进行中**）

（Task 1.1 / 1.2 / 1.3 见本文件 §1.3 与 §2.1-2.2 的约束；已派单，正在 `crates/campus-synjones/` 落地。）

- [ ] Step 1：crate 骨架 + 常量防漂移单测
- [ ] Step 2：live 探针跑出桥的真实落点（打印每跳、token 打码），固化 `sso_token`
- [ ] Step 3：`client.rs` 头组（std `TcpListener` 最小桩单测）+ 三套信封解析单测
- [ ] Step 4：`cargo test --workspace` 全绿（基线 102 passed）+ 提交

### 批 2：一卡通后端 + 钱包页 + 首页钱包卡

**Files:** Create `tauri-app/src-tauri/src/commands/synjones.rs`、`crates/campus-synjones/src/ecard.rs`；Modify `commands/mod.rs`、`lib.rs`（注册）、`frontend/src/shared/{tauriApi.ts,types.ts}`、`panels/WalletPanel.tsx`、`panels/TodayPanel.tsx`（仅钱包卡部分）

- [ ] Step 1：`ecard.rs` 解析单测（内联 JSON 样本、**去 PII**）：`db_balance:"1751"`,`unsettle_amount:"0"` → `17.51`；`elec_accamt:"2500"` → `25.0`；流水 `tranamt:"350"`,`typeFrom:"1"` → `+3.50`
- [ ] Step 2：live 测试（`#[ignore]`）断言真实余额与流水条数 > 0；顺带实测 §1.7 未知 3（`type=1/2` 对比、`sum/user` 参数）
- [ ] Step 3：命令 `get_ecard` / `get_ecard_transactions` / `get_wallet_cards`（后端聚合 + 门户降级）
- [ ] Step 4：`WalletPanel` 接线（余额、今日/本月消费、交易记录分页、加载/空/错误态）；`TodayPanel` 钱包卡接 `get_wallet_cards` 并显示「实时 / 门户快照」来源小字
- [ ] Step 5：真机点验（`tauri dev` + CDP，方法见 `.codewiki/learnings/tauri-webview-ui-verification.md`）
- [ ] Step 6：提交

### 批 3：电费后端 + 电费页 + 充值内嵌页

**Files:** Create `crates/campus-synjones/src/charge.rs`、`tauri-app/src-tauri/src/commands/electricity.rs`、`frontend/src/panels/RechargePanel.tsx`（或组件）；Modify `commands/mod.rs`、`lib.rs`、`tauriApi.ts`、`types.ts`、`panels/PowerPanel.tsx`、`AppShell.tsx`（路由/窗口）

- [ ] Step 1：`charge.rs` 单测：`status==2` 过滤（用实测 8 条样本）；末级 `type` 切 `IEC`；`map.showData` 字典 → `fields`
- [ ] Step 2：live 测试跑到 450 的末级，**固化 `showData` 键名**（§1.7 未知 2）
- [ ] Step 3：命令 `list_feeitems` / `query_electricity` / 房间 CRUD（§2.5）
- [ ] Step 4：`PowerPanel`：片区选择 → 三级级联 → 剩余金额/单价（`fields` 通用渲染）→ 绑定常用房间（本地）→ 一键查询；文案保留「查询与充值」
- [ ] Step 5：`open_recharge_page` + 内嵌窗口（§2.6），实测初始化脚本所需的键（§1.7 未知 4），确认官方页在注入后可用
- [ ] Step 6：真机点验 + 提交

### 批 4：命令面板（前端独立，可与批 2/3 并行——文件不重叠）

**Files:** Create `frontend/src/components/CommandPalette.tsx`（+ 需要的子组件）；Modify `components/AppShell.tsx`、`stores/uiStore.ts`

- [ ] Step 1：`uiStore` 加 `paletteOpen` + `togglePalette`；`AppShell` 注册 `keydown`（Cmd/Ctrl+K，含输入框内触发）
- [ ] Step 2：面板组件（模糊搜索 + 键盘导航 + 可访问性），数据源三类（§2.7）
- [ ] Step 3：清掉 `AppShell.tsx:73,83,91` 的「M2 接入」注释与 `aria-disabled` 占位
- [ ] Step 4：真机点验（含只点键盘的完整链路）+ 提交

### 批 5：收尾

- [ ] Step 1：`PLAN.md`：M3 勾选与「完成情况说明」（含诚实边界）、M2 章节补记命令面板已补做、命令面条数更新
- [ ] Step 2：`docs/HANDOFF.md:110` 同句纠错
- [ ] Step 3：`CHANGELOG.md` 追加条目（bug 修复 + M3 各批，各自一条）
- [ ] Step 4：CodeWiki：新增 `modules/campus-synjones.md`、`learnings/synjones-auth-and-envelopes.md`、`learnings/portal-session-expiry-vs-parse-error.md`（批 0 的教训）；更新 `_architecture.md`；`cw index` + `cw meta update`，wiki 与代码同次提交
- [ ] Step 5：真机全链点验（登录 → 首页钱包卡实时 → 钱包流水 → 电费三级查询 → 绑定房间 → 充值内嵌页 → 命令面板 → 重开应用仍可查）
- [ ] Step 6：合并本会话分支（主智能体执行）

---

## 四、验收清单（对齐 `PLAN.md:120` + 用户裁决）

- [ ] 资讯页在会话有效时正常拉取；会话失效时提示「登录已过期，请重新登录」而不是「解析失败」（批 0）
- [ ] **各界面资讯栏目恢复**（用户报告的故障）
- [ ] 一卡通余额与官方渠道一致（与慧新E校 App/H5 对照一次）；首页钱包卡与钱包页**数字一致**
- [ ] 流水条数、金额符号（收入/支出）、时间正确
- [ ] 电费：三个启用片区都能选到房间并返回剩余金额与单价；停用片区不出现
- [ ] 常用房间可保存、一键复查、重启应用后仍在
- [ ] 充值：应用内 webview 打开官方页面、已是登录态、无需跳浏览器；窗口观感与客户端一致（标题/图标/外壳）
- [ ] 命令面板：Cmd+K 可用、键盘可全程操作、不再是灰掉的占位
- [ ] 校内全程不需浏览器；未登录时各页给登录引导而非报错
- [ ] `cargo test --workspace` 全绿（基线 102 passed + 新增）

## 五、风险

| 风险 | 应对 |
|---|---|
| lyCas 桥落点不带 token（与 bundle 推断不符） | 批 1 探针如实记录；停下等裁决，不擅自改走账密 |
| 官方充值页注入 sessionStorage 后仍不可用 | 批 3.3 实测；兜底「在浏览器打开」按钮 |
| 「统一风格」做不到像素级 | 已写明是尽力而为：窗口外壳统一 + 限量 CSS 注入，官方改版会失效 |
| 首页实时取数拖慢首屏 | 门户数据先渲染，实时值到达后替换（不阻塞首屏）；失败静默降级到门户值 + 来源标注 |
| `showData` 键名学校侧变更 | 通用字典渲染（不硬编码键名），键名只进测试断言 |
| 校外不可用（内网明文 IP） | M3 只验校内侧，校外归 M4；UI 失败时提示「需校园网」 |
| 多分身同 worktree 并发 | 按批分配互不重叠的文件范围；批 4（前端）与批 3（后端+电费页）错开文件 |

## 六、用户已裁决（2026-09-19）

1. **命令面板**：彻底解决 → 补做完整版（批 4）。
2. **首页钱包卡**：首页与一卡通**都使用实时余额**（§2.4，带门户降级与来源标注）。
3. **电费充值**：**用官方链接但在软件内部跳转**（内嵌 webview，不跳浏览器），并**统一显示风格**（§2.6）。
