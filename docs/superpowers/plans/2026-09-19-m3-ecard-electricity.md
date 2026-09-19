# 计划：M3 一卡通 + 电费查询（慧新E校直连）

> 2026-09-19（会话 `sess-36e5d8e0`）· 分支 `feat/sess-36e5d8e0-m3-card-electricity`
> 用户授权：「PLAN.md 的 M2 部分内容好像没完成，同时开始完成 M3；M2.5 正在由其他会话完善，你负责规划，让智能体执行」。
> 范围 = `PLAN.md` §M3。**不碰课表（M2.5 归其他会话）**。主智能体负责侦察/规划/验收/合并，分身负责实现。
> **For agentic workers:** REQUIRED SUB-SKILL: 用 `superpowers:subagent-driven-development` 逐任务执行；步骤用 `- [ ]` 勾选跟踪。

**Goal:** 让「钱包」页与「电费」页在校园网内真实可用——一卡通余额/流水来自慧新E校实时接口，电费按「片区→校区→楼栋→房间」查到剩余金额与单价，并把常用房间记住。

**Architecture:** 新建 `crates/campus-synjones`（慧新E校协议单点，仿 `campus-portal` 与 `campus-auth/jwglxt.rs` 的既有范式）：`sso.rs` 用 CAS TGT 走 lyCas 桥换 token，`client.rs` 持有 token 并统一注入头组，`ecard.rs` / `charge.rs` 两个业务模块。Tauri 命令层只做锁内 clone + 调 crate + 映射 `CommandResult`。前端两个既有 panel 接线。

**Tech Stack:** Rust（reqwest + serde + tokio）、Tauri 2 命令面、React 19/TS + zustand。

**Spec:** `PLAN.md` §M3（第 115-120 行）+ 本文件 §1 的实测事实表（协议细节以本文件为准，它是 2026-09-19 实测的）。

## Global Constraints

- 行文与注释用中文；命令/报错原文照抄。
- **凭据红线**：token、账号、密码一律只在内存，**绝不落盘、绝不打日志、绝不进文档/提交**；live 测试样本只写仓库外 recon 目录（`%TEMP%/campushub-m3-recon`）。
- 一切慧新E校业务请求必须同时带 `synAccessSource=app`（GET 走 query、POST 走 body）与头 `synAccessSource: app`（官方前端行为是"双份携带"，照抄）。
- 网络超时 10s（与 `campus-portal` 一致）；错误归一为 `CampusSynjonesError`，**不要**把 reqwest 错误直接冒到前端。
- 只用现有依赖；新增依赖需在计划评审时说明理由。
- 每批完成后由主智能体验收 + 合并（`bash ~/.zcode/scripts/git-merge-push.sh <worktree 目录>`），分身不合并、不 push。

---

## 一、侦察事实（2026-09-19 实测，除标注外均可匿名复核）

### 1.1 服务拓扑（三个独立服务，命名**不可**泛化）

| 服务前缀 | 用途 | 证据 |
|---|---|---|
| `/berserker-auth/*` | 认证（oauth/token、cas 桥） | 302 实测 |
| `/berserker-app/*` | 一卡通（`ykt/tsm/*`）、应用方案 | 401 实测 |
| `/berserker-search/*` | **一卡通流水/统计/标签**（本轮新发现） | bundle 内 API 表 |
| `/charge/*` | 电费（feeitem 目录/详情/级联） | 401 实测 |
| `/charge-pc/*` | 电费前端 SPA 壳（**不是接口**） | 返回 HTML |

⚠️ `/berserker-acc/` 实测 **404**，服务名必须逐个实测，禁止按「berserker-*」猜。

### 1.2 鉴权四件事

1. **token 头**：`synjones-auth: bearer <token>`（`token_type` 缺省 `bearer`）。
2. **来源参数**：值来自前端 `sessionStorage.agentType`，缺省 `h5`；本项目固定用 **`app`**（PLAN §3.2 的 4030 结论：`pc` 被拒、`app` 放行）。GET/DELETE 放 query，POST 放 body（form 用 `qs.stringify` 合并、JSON 合并进对象），**且所有方法都再加同名请求头**——官方拦截器就是这么做的，照抄最稳。
3. **4030 的判定**：`HTTP 401` 且 `body.code == 4030`（不是 HTTP 403）。
4. **两套响应信封，解析器不可复用**：
   - berserker 系：`{code, success, data, msg}`（成功 `code==200`）
   - charge 系：`{code, message}`（成功 `code==200`；401 时 message 可能是空串）
   - search 系：`{code, data, msg}`（成功 `code==200`）

### 1.3 SSO 桥（CAS → 慧新E校）

实测 302：`GET /berserker-auth/cas/redirect/lyCas?targetUrl=<目标>` → `Location: https://wxcas.cwxu.edu.cn/lyuapServer/login?service=<URL 编码后的 http://10.3.100.110/berserker-auth/cas/login/lyCas?targetUrl=<二次编码的目标>>`。

**要点**：换 ST 要指向的 service 是 **`{BASE}/berserker-auth/cas/login/lyCas?targetUrl=<编码>`**（`cas/redirect/lyCas` 只是给浏览器用的入口）。这与 `jwglxt` 的「service 即业务回跳端点」模式同构 → **照抄 `crates/campus-auth/src/jwglxt.rs:49-146`**。

**子 SPA 的 token 交接方式**（campus-card-pc bundle 实测）：落点 URL 上带 `?synjones-auth=<raw token>`（无 `bearer ` 前缀），子 SPA `created()` 读该 query 写入 sessionStorage。plat 侧下发形态同为 `plat + "?" + qs.serialize({name, "synjones-auth": token})`。

**兜底**（桥拿不到 token 时）：`POST /berserker-auth/oauth/token`，`Content-Type: application/x-www-form-urlencoded`，`Authorization: Basic bW9iaWxlX3NlcnZpY2VfcGxhdGZvcm06bW9iaWxlX3NlcnZpY2VfcGxhdGZvcm1fc2VjcmV0`（= `mobile_service_platform:mobile_service_platform_secret`，已在 bundle 内 base64 逐字验证），表单 `{username, password, grant_type:"password", scope:"all", loginFrom:"pc", logintype:"username"}` → `{access_token, token_type, flag}`。⚠️ 需要用户在慧新E校的密码（可能≠CAS 密码），**默认不启用**，仅作探针的对照实验。

### 1.4 一卡通接口（全部需 token）

| 用途 | 方法 | 路径 | 参数 | 返回要点 |
|---|---|---|---|---|
| 当前卡 | GET | `/berserker-app/ykt/tsm/queryCurrentCard` | 无 | `data:{retcode:"0", errmsg, card:[…]}`（`retcode!="0"` 用 `errmsg` 报错） |
| 卡明细 | GET | `/berserker-app/ykt/tsm/queryCard` | `account=<卡号>` 或 `scene=recharge` | `data.card[]` |
| 多卡列表 | GET | `/berserker-app/ykt/tsm/getCampusCards` | — | `data.card[]` |
| 学校账户 | GET | `/berserker-app/ykt/tsm/getSchoolAccountinfo` | — | 待实测字段 |
| **消费流水** | GET | `/berserker-search/search/personal/turnover` | `size`、`current`（页码）、`account`、`type=2` | `{code:200, data:{total, records[]}}` |
| 流水汇总 | GET | `/berserker-search/statistics/turnover/sum/user` | — | 待实测（今日/本月消费用） |
| 流水计数 | GET | `/berserker-search/statistics/turnover/count` | — | 待实测 |
| 交易类型字典 | GET | `/berserker-search/search/turnoverType` | — | 待实测 |

**卡对象字段（bundle 实测，含前端单位换算逻辑）**：

- `db_balance` + `unsettle_amount`（**单位：分**）→ 显示余额 `(db_balance + unsettle_amount)/100` 元
- `elec_accamt`（分）→ 电控账户金额 `(elec_accamt/100)` 元 ← **慧新E校侧的「电费余额」字段**
- `acc_status`（`0`=正常）、`lostflag`（`1`=挂失）、`account`、`cardname`
- `accinfo[]`：电子账户 `{name, type, balance}`（balance 单位分）
- `ecardConfig.type` 决定展示卡账户还是电子账户

**流水记录字段**：`jndatetimeStr`（"日期 时间"）、`resume`（内容）、`tranamt`（**分**，`typeFrom=="1"` 为收入 `+`，否则支出 `-`）、`typeFrom`、`payName`（支付方式）、`icon`、`labelName`、`orderId`。

### 1.5 电费接口

| 步骤 | 方法 | 路径 | 需 token | 说明 |
|---|---|---|---|---|
| 1 | GET | `/charge/feeitem` | **否** | `{code:200, feeitemList:[…]}`（**匿名可读**） |
| 2 | GET | `/charge/feeitem/singleFeeitem?feeitemid=450` | 是 | `{code, feeitem, view, sceneinfo, libraryinfo, tipinfo}` |
| 3 | POST | `/charge/feeitem/getThirdData` | 是 | form-urlencoded，体 = `feeitemid` + `type` + `level` + 各级已选值 |

**feeitem 字段**：`feeitemid`、`name`、`impl_interface`、`interfacechoice`、`status`（**1=启用，2=停用，必须过滤**）、`billing_unit`（"元"）、`layout`（快捷金额 "10,50,100"）、`retain_money`（最小额）、`maxmoney`（最大额）、`flag`、`chargeunit`、`pay_productid`、`remark`。

2026-09-19 实测的启用项（**3 条，不是只有 450**）：`450` 梅园1号-梅园3号、`449` 李园9号-李园11号、`448` 桃园1号-李园8号（另有同名停用项 `428`，靠 `status` 过滤掉）。

**级联规则**（charge-pc bundle 实测）：首次请求 `{feeitemid, type:"select", level:0}`；选中第 k 级后带该级参数、`level` 递增；**选中最后一级（房间）时 `type` 切为 `"IEC"`**（`moreImpl` 为真时：`level>1` 且非终极 → `select`）。响应 `map`：

- `map.total[]`：层级定义 `{level, code, name, value}`
- `map.data`：下一级选项数组（`type=select` 时）
- `map.showData`：**最终展示信息，键名由服务端动态下发**（前端是通用字典遍历渲染 `键: 值`）→ **「剩余金额」「单价」就在这里面，具体键名必须带 token 实测后固化**
- `map.money` / `map.iectranamt`：金额（元）
- `map.tipinfo`：错误/提示文案（存在时 `showData` 置空）
- `thirdOptions.sceneinfo`：已绑定房间（含 `dataStr`）
- 绑定的**平台侧**写接口是 `POST /charge/sceneBind/add`（本计划**不用**，见 §2.3）

### 1.6 必须用真实会话确认的三项未知（**批 1 的首要产出**）

| # | 未知 | 确认方法 |
|---|---|---|
| 1 | lyCas 桥落点的 token 传递方式（URL query？Set-Cookie？都没有？） | live 探针手动跟随 302 链，打印每跳 URL（token 值打码）与 `Set-Cookie` 名，然后用拿到的 token 调 `queryCurrentCard` 断言 200 |
| 2 | `map.showData` 的键名（剩余金额/单价） | live 测试对 450 跑 `level 0→1→2` 直到 `type=IEC`，把 `showData` 的**键名**（不含值）写入 recon 样本并固化断言 |
| 3 | 流水 `type=2` 语义 + 一卡通账单详情接口 | live 测试对比 `type=1/2` 返回；详情接口 `orderId` 相关端点逐个试 |

### 1.7 被证伪的旧事实（**必须在同一任务里纠正**）

`PLAN.md:52`、`docs/HANDOFF.md:110`、4030 记忆文件里「电费查询直连 `http://10.3.100.110/charge-pc/pays/450` 匿名可用、返回剩余金额与单价」——**错**：该 URL 返回 Vue SPA 壳（HTML），真正的数据接口是 `/charge/feeitem*`，且除 feeitem 目录外都需要 token。

---

## 二、设计决策

### 2.1 新建 crate `crates/campus-synjones`

理由：与 `campus-portal`（一个外部系统一个 crate）对称；慧新E校与门户/教务的鉴权、信封、错误语义完全不同，塞进 `campus-auth` 会让它变成杂物间；M6 安卓端要复用协议核心。根 `Cargo.toml` 若是 `crates/*` glob 则无需改动 members（执行者先确认）。

模块（一文件一职责）：
- `lib.rs`：`CampusSynjonesError` 枚举 + 重导出
- `sso.rs`：`sso_token(tgt) -> Result<SynjonesToken>`（lyCas 桥 + 落点解析 + 兜底信号）
- `client.rs`：`SynjonesClient`（持 token、共享 reqwest client、`with_headers()` 统一注入、三套信封解析）
- `ecard.rs`：`EcardOverview` / `CardInfo` / `Transaction` + 取数函数
- `charge.rs`：`FeeItem` / `CascadeState` / `ElectricityResult` + 级联函数

### 2.2 token 获取：SSO 桥优先

用现有 CAS TGT（`AppState` 已有）走 lyCas 桥，**不要求用户额外输入慧新E校密码**。桥失败时返回明确错误（`BerserkerSsoFailed`），UI 提示"请重新登录"；oauth 账密路径**不接入产品**（探针里作对照实验即可）。

会话失范式沿用 `jwglxt.rs` 的既有做法：业务请求 401/4030 → 用 TGT 静默重进桥一次 → 重试一次 → 仍失败归一为 `NotLogin`。

### 2.3 常用房间：**本地存**，不调 `sceneBind/add`

`sceneBind/add` 是平台侧写接口（改学校系统里的数据），风险与依赖都高。本计划把「常用房间」实现为本地配置（落 `config` 同款机制，或前端 zustand 持久化）：保存 `{feeitemid, path: Vec<选中的层级值>, 显示名}`，点一下即重放 `getThirdData` 到末级拿余额。YAGNI：满足 M3「多房间记忆」与 M5 余额提醒的铺路需求即可。

### 2.4 不做充值/缴费下单

charge-pc 的下单不是 REST：先 `SHA256` 签名再用动态 HTML form 自动 POST 跳转，且涉及真实金钱。`PLAN.md:118` 只要求查询。`PowerPanel` 里"查询与充值"文案改为"查询"。**待用户裁决**（见 §6）。

### 2.5 命令面（6 条，`CommandResult` 契约不变）

| 命令 | 入参 | 出参 |
|---|---|---|
| `list_feeitems` | — | `FeeItem[]`（已过滤 `status==1`；**匿名可用，未登录也能列**） |
| `query_electricity` | `feeitemId: String`, `path: String[]` | `{ options: {label,value}[] , isFinal: bool, result?: { fields: {label,value}[], money?: f64 } }` |
| `get_electricity_rooms` | — | `SavedRoom[]` |
| `save_electricity_room` | `room: SavedRoom` | `SavedRoom[]` |
| `delete_electricity_room` | `id: String` | `SavedRoom[]` |
| `get_ecard` | — | `{ balance: f64, elecAccamt: f64, cards: CardInfo[], account: String }` |
| `get_ecard_transactions` | `account: String`, `page: u32` | `{ total, records: Transaction[] }` |

共 7 条（含 3 条房间 CRUD）；`list_feeitems` 免登录，其余需会话。

---

## 三、任务分解

### 批 1：SSO 桥 + 探针（**最关键，未知最多**）

#### Task 1.1：crate 骨架 + 错误类型 + 常量

**Files:** Create `crates/campus-synjones/{Cargo.toml, src/lib.rs, src/client.rs}`；Modify 根 `Cargo.toml`（若 members 非 glob）

**Interfaces:**
- Produces: `pub const BERSERKER_BASE: &str = "http://10.3.100.110";`、`pub const LY_CAS_SERVICE_PREFIX: &str = "http://10.3.100.110/berserker-auth/cas/login/lyCas";`、`pub enum CampusSynjonesError { NotLogin, SsoFailed(String), Api{code:i32,msg:String}, Http(String), Parse(String) }`、`pub struct SynjonesToken { pub access_token: String, pub token_type: String }`（**实现 `Debug` 时手写、token 打码**）

- [ ] Step 1：写常量防漂移测试（断言两个常量字面值）
- [ ] Step 2：跑 `cargo test -p campus-synjones` 确认失败（crate 不存在）
- [ ] Step 3：建 crate（`Cargo.toml` 依赖：`reqwest`（同 campus-auth 的 features）、`serde`、`serde_json`、`url`、`thiserror`（同现有 crate 用法）；dev-deps：`tokio`（rt-multi-thread、macros）、`base64`）——**照抄 `crates/campus-auth/Cargo.toml` 的依赖版本与 features**
- [ ] Step 4：跑测试通过
- [ ] Step 5：提交

#### Task 1.2：`sso.rs` 桥接 + live 探针

**Files:** Create `crates/campus-synjones/src/sso.rs`、`crates/campus-synjones/tests/synjones_live.rs`

**Interfaces:**
- Consumes: `campus_auth::cas::{CasClient, sso_follow}`（`crates/campus-auth/src/cas.rs:199-247` 的手动跟随）、`crates/campus-auth/src/jwglxt.rs:49-146` 的范式
- Produces: `pub async fn sso_token(...) -> Result<SynjonesToken, CampusSynjonesError>`

- [ ] Step 1：**先写 live 探针测试**（`#[ignore]`，仿 `crates/campus-auth/tests/jwglxt_live.rs` 全文结构，含 `CAMPUS_HUB_CREDS` 解析与 recon 目录）
  - 凭据来源（按序尝试，**只读、不打印**）：① `CAMPUS_HUB_CREDS` 环境变量；② `%APPDATA%/campushub/accounts.json` DPAPI 解密（复用 `tauri-app/src-tauri/src/infra/` 现有 DPAPI 实现思路，测试内自带 FFI 小函数）——②需在测试文件头注释里写明依据
  - 切主题：探针断言 ① SSO 桥每跳 URL 的 host/path（token 打码）；② 落点是否含 `synjones-auth` query 或 `Set-Cookie`；③ 用得到的 token 打 `queryCurrentCard` 断言 HTTP 200 且 `code==200`
  - 样本写 `%TEMP%/campushub-m3-recon/`（仓库外）
- [ ] Step 2：跑 `cargo test -p campus-synjones -- --ignored synjones_sso_live --nocapture` 看真实落点（**这一步是取证，不是绿灯**）
- [ ] Step 3：按实测落点实现 `sso_token`：`sso_ticket(tgt, service)` → 手动跟随 302 → 从落点 URL query 取 `synjones-auth`（无前缀）→ 包成 `SynjonesToken{token_type:"bearer"}`
- [ ] Step 4：把探针改成断言（固化事实），并把落点规律写进 `sso.rs` 的文件头注释
- [ ] Step 5：若桥拿不到 token：在探针里追加 oauth 对照实验（**需要用户密码，只在用户在场时跑**），并把 `SsoFailed` 的文案设计为可操作提示
- [ ] Step 6：提交

**验收判据**：live 测试通过，`queryCurrentCard` 返回 200 且拿到真实卡信息（余额字段能解析出分→元）。

#### Task 1.3：`client.rs` 头组与信封

- [ ] Step 1：单测——用 `httpmock`/本地 `TcpListener` 起桩服务（**优先用 std `TcpListener` 手写最小桩，避免新增依赖**）断言：GET 请求 query 含 `synAccessSource=app`、头含 `synjones-auth: bearer <t>` 与 `synAccessSource: app`；POST form 请求 body 含 `synAccessSource=app`
- [ ] Step 2：跑测试失败 → 实现 `with_headers()` → 通过
- [ ] Step 3：单测覆盖三套信封解析（`{code:200,success:true,data:…}` / `{code:401,message:…}` / `{code:4030,…}` → 各自映射的错误变体）
- [ ] Step 4：提交

### 批 2：一卡通后端 + 钱包页

#### Task 2.1：`ecard.rs` 取数

- [ ] Step 1：写解析单测（内联 JSON 样本，来自探针的 recon 文件，**去掉 PII/卡号**）：`db_balance:"1751"`, `unsettle_amount:"0"` → `balance == 17.51`；`elec_accamt:"2500"` → `25.0`；流水 `tranamt:"350"`,`typeFrom:"1"` → `+3.50`
- [ ] Step 2：实现 `fetch_current_card` / `fetch_transactions`
- [ ] Step 3：live 测试断言真实余额与流水条数 > 0
- [ ] Step 4：提交

#### Task 2.2：Tauri 命令 + 前端接线（WalletPanel）

**Files:** Create `tauri-app/src-tauri/src/commands/synjones.rs`；Modify `commands/mod.rs`、`lib.rs:19-62`（注册）、`tauri-app/frontend/src/shared/{tauriApi.ts,types.ts}`、`panels/WalletPanel.tsx`

- [ ] Step 1：命令层照抄 `commands/timetable.rs:196-236` 范式（锁内只 clone、drop guard 再 await、失败透出可读文案、`CommandResult` 三态）
- [ ] Step 2：`cargo test -p campus-synjones` + `cargo test --workspace` 全绿；`cargo check` 通过
- [ ] Step 3：前端：余额大数字、今日/本月消费、交易记录列表（分页/加载更多、空态、加载态、错误态）
- [ ] Step 4：真机点验（`tauri dev` + CDP，方法见 `.codewiki/learnings/tauri-webview-ui-verification.md`）
- [ ] Step 5：提交

### 批 3：电费后端 + 电费页

#### Task 3.1：`charge.rs` 级联

- [ ] Step 1：单测：`status==2` 的 feeitem 被过滤（用 2026-09-19 实测的 8 条样本）；`type` 在末级切 `IEC` 的逻辑；`map.showData` 字典 → `fields`
- [ ] Step 2：实现 `list_feeitems` / `query_cascade`
- [ ] Step 3：live 测试：`feeitemid=450` 从 `level=0` 走到末级，断言拿到 `showData` 且**把键名固化进断言**（Task 1.2 之外的第二个取证点）
- [ ] Step 4：提交

#### Task 3.2：房间记忆 + 命令 + PowerPanel

- [ ] Step 1：房间记忆落盘（同 `timetable` config 机制），单测覆盖增删查
- [ ] Step 2：命令注册（5 条）
- [ ] Step 3：PowerPanel：片区选择 → 三级级联选择器 → 剩余金额/单价（`fields` 通用渲染）→ 绑定常用房间（本地）→ 一键查询已绑定房间；文案去掉"充值"
- [ ] Step 4：真机点验
- [ ] Step 5：提交

### 批 4：收尾

- [ ] Step 1：`PLAN.md` §1.7 的事实纠错（第 52/53 行）、M3 章节勾选与完成情况说明（含诚实边界）
- [ ] Step 2：`docs/HANDOFF.md:110` 同句纠错；4030 记忆文件纠错（**注意记忆文件不在仓库，由主智能体处理**）
- [ ] Step 3：`CHANGELOG.md` 追加 M3 条目（日期/模块/摘要）
- [ ] Step 4：CodeWiki：新增 `modules/campus-synjones.md`、`learnings/synjones-auth-and-envelopes.md`（两套信封 + synAccessSource 双份携带 + 4030 判定），更新 `_architecture.md` 模块图；跑 `cw index` + `cw meta update`，wiki 改动与代码同一次提交
- [ ] Step 5：真机全链点验（登录 → 钱包 → 电费 → 绑定房间 → 重开应用仍可查）
- [ ] Step 6：合并本会话分支（主智能体执行）

---

## 四、验收清单（对齐 `PLAN.md:120`）

- [ ] 一卡通余额与官方渠道一致（与慧新E校 App/H5 对照一次）
- [ ] 流水条数、金额符号（收入/支出）、时间正确
- [ ] 电费：三个启用片区都能选到房间并返回剩余金额与单价；停用片区不出现
- [ ] 常用房间可保存、可一键复查、重启应用后仍在
- [ ] 校内全程不需浏览器（无任何跳浏览器动作）
- [ ] 未登录时两页显示登录引导，不报错
- [ ] 会话失效（401/4030）时自动重进一次，仍失败给"请重新登录"而非原始报错
- [ ] `cargo test --workspace` 全绿（基线 102 passed + 本轮新增）

## 五、风险

| 风险 | 应对 |
|---|---|
| lyCas 桥落点不带 token（与 bundle 推断不符） | 探针如实记录；回落 oauth 账密需用户决策（§6） |
| `showData` 键名学校侧变更 | 通用字典渲染（不硬编码键名），键名只进测试断言 |
| 校外不可用（内网明文 IP） | **M3 只验校内侧**，校外归 M4（WebVPN）；UI 在请求失败时提示"需校园网" |
| `type=2` 语义误读导致流水错 | 探针对比实测；不确定就在 UI 只显示"全部" |
| 账号在多设备登录触发平台风控 | 只读接口、低频请求；不做轮询（轮询归 M5） |

## 六、待用户裁决（不阻塞批 1-3 主干）

1. **顶栏命令面板**：M2 遗留（`AppShell.tsx:73,83,91` 注释称「M2 接入」，但 PLAN §M2 无此条）——补做 / 砍掉注释 / 推后？
2. **首页钱包三卡数据源**：是否随 M3 一起切到慧新E校实时接口（当前用门户 `queryAppointCard`）？
3. **电费充值**：M3 不做（PLAN 未要求、涉及金钱与签名表单）——确认还是需要？（若需要，UI 至少可提供"跳官方页面充值"）
