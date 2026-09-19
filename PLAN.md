# Wxxy-CampusHub（锡院助手）— 无锡学院融合门户优化版 · 项目计划

> 已定名：仓库 `Wxxy-CampusHub`，应用显示名 **锡院助手**（2026-09-15 用户确认）。
> 本文档是项目的唯一计划来源，里程碑推进时在此勾选更新。

## 一、项目定位

把学校官方融合门户（`https://my.cwxu.edu.cn/#/studentNewPage`）与内网慧新E校平台（`http://10.3.100.110`，新中中）的核心能力，重写为一个**自研客户端**：

- **Windows 桌面版先行，安卓版跟进**（先把坑踩完，安卓端复用桌面端协议核心）
- 技术栈与 [Wxxy-CampusLogin](../Wxxy-CampusLogin) 完全同款：**Tauri 2（Rust 后端 + React 19/TypeScript 前端，WebView 渲染）**，沿用其"协议单点 + 平台外壳"双端同构架构
- 官方门户功能整体重写（自研 UI）；**课表为独立重制功能（M2.5，2026-09-17 用户确认推翻原"暂不做"）**
- 三类系统通知：门户公告推送、电费余额提醒、待办事项提醒
- 校内直连内网服务，校外自动走学校 WebVPN（深澜 Srun）

## 二、已确认的技术决策

| 决策 | 内容 | 来源 |
|---|---|---|
| 技术栈 | Tauri 2 + Rust + React 19/TS | 用户指定，与 Wxxy-CampusLogin 一致 |
| 双端策略 | 先 Windows 后 Android；协议核心 crate 放桌面端，安卓以 Cargo path 依赖引用 | 用户指定 + CodeWiki 架构模式 |
| 前端状态 | zustand 领域 store，IPC 唯一出口 `tauriApi.ts`，命令返回 `CommandResult { success, message?, data? }` | 同上 |
| 网络路由 | 移植 Wxxy-CampusLogin 校园网检测模块；校内直连 `10.3.100.110`，校外走 `webvpn.cwxu.edu.cn`（仅内网服务走 VPN） | 用户指定 |
| CAS 验证码 | 自动识别填写 | 用户指定 |
| **首页形态** | **v2 精简版（用户 2026-09-17 拍板）**：去掉资讯/快捷应用/待办/会议/信息服务/课表卡片，只留问候+钱包三卡+下一节课横幅+快捷动作；各模块靠独立页+系统通知 | 用户指定 |
| **课表方案** | **做课表（推翻"暂不做"）**：独立页面 M2.5；教务自动导入 + 调课通知自动调整 + 自动更新；导入/手动课程按 source 标签隔离；算法与数据模型移植自 shiguangschedule（Apache-2.0） | 用户指定 |
| git 仓库 | 已 `git init`（2026-09-15，项目名确认后） | 工作区规则 |

## 三、侦察结论（已验证的技术事实）

逆向与实测得到的关键事实，开发时直接引用，避免重复探索：

### 1. 融合门户（公网）
- 入口 `https://my.cwxu.edu.cn/#/studentNewPage`，SPA + hash 路由，**公网可访问**
- 登录走 CAS：`https://wxcas.cwxu.edu.cn/lyuapServer/login?service=https://my.cwxu.edu.cn/shiro-cas`
  - 三种登录方式：微信扫码 / 短信 / 账号密码（学生账号=学号）
  - **协议已逆向并实测打通（2026-09-17，详见 `docs/cas-recon/REPORT.md`）**：
    ① `GET /lyuapServer/kaptcha` 取算术题验证码（两一位数 +/-/*，uid+base64 PNG，无 Cookie 依赖）
    ② `POST /lyuapServer/v1/tickets`（x-www-form-urlencoded）：`username/password(RSA密文)/service/loginType/id(=验证码uid)/code(=答案)/otpcode`，请求头 `token=RSA("lyasp"+毫秒时间戳)`
    ③ 响应 JSON 直含 `ticket`(ST) 与 `tgt`(TGT)，回跳 `service?ticket=ST` 完成门户 SSO
    ④ 密码加密为 textbook RSA（1024 位，公钥 e=010001、n 见 REPORT，little-endian 组块 126 字节、无 padding、hex 不补零）
    ⑤ 错误码全集见 REPORT（NOUSER=账号密码错、CODEFALSE=验证码错等）
    ⑥ **真实账号端到端登录已验证（2026-09-17）**：CAS 签发 TGT+ST → 门户会话 Cookie（customsid/Authorization/rememberMe）建立；成功响应顶层即 `{tgt,ticket}`
    ⑦ **WebVPN（深澜 Srun）CAS 联动已验证（2026-09-17）**：CAS service 换成 `https://webvpn.cwxu.edu.cn/login?cas_login=true` 即可签发 WebVPN 会话（先取初始 wengine_vpn_ticket → ticket 回跳 → wengine-vpn-token-login）；深澜代理 URL 样本已采集（M4 素材）
- 学生首页功能模块：问候/搜索、个人卡片（**一卡通余额、邮箱未读、图书借阅**）、信息服务链接（知网/校历/官网/图书馆）、快捷入口、应用中心（`#/newlyappCenter`）、**资讯中心**（通知公告/校园要闻/教务处/学工处/团委等分类）、**待办中心**（待办/已办/申请）、日程会议列表

### 2. 慧新E校（内网，新中中平台）
- Spring Cloud OAuth 体系；登录 `POST /berserker-auth/oauth/token`，Basic 头为 `mobile_service_platform:mobile_service_platform_secret` 的 base64
- token 存 sessionStorage（`access_token`/`token_type`），请求带自定义头 `synjones-auth: bearer <token>`；⚠️ sessionStorage 不跨标签页，新开标签即掉登录（2026-09-17 实测，官方产品缺陷，客户端后端持 token 规避）
- 门户→慧新E校 SSO 桥（2026-09-19 实测补全）：浏览器入口 `http://10.3.100.110/berserker-auth/cas/redirect/lyCas?targetUrl=...` → 302 到 CAS，其 `service` 指向 `{BASE}/berserker-auth/cas/login/lyCas?targetUrl=<二次编码>`——**换 ST 要对准后者**（与教务 `jwglxt` 同构）；子 SPA 的 token 交接靠落点 URL 的 `?synjones-auth=<raw token>` query
- ⚠️ **4030 故障关键结论**：服务端按请求参数 `synAccessSource` 做来源授权，`pc` 来源校验失败（当前服务端配置问题），**`app` 来源放行**——本客户端调 berserker 系接口一律带 `synAccessSource=app`
- **电费接口（2026-09-19 复核；旧说法「`charge-pc/pays/450` 匿名可用」已证伪——那只是 Vue SPA 壳 HTML）**：真链路 = `GET /charge/feeitem`（**唯一匿名可读**，返回启用片区 450 梅园1-3号 / 449 李园9-11号 / 448 桃园1-8号，另有同名停用项 428 须按 `status==1` 过滤）→ `GET /charge/feeitem/singleFeeitem?feeitemid=`（需 token）→ `POST /charge/feeitem/getThirdData`（form，`level` 0→1→2 级联、末级 `type=IEC`；结果在 `map.showData` 动态键名字典，金额 `map.money|iectranamt`）；**级联与详情均需 token**
- 应用方案接口 `GET /berserker-app/appScheme/info?type=user&serviceType=<agentType>` 可拿到服务应用列表（**匿名可读**，2026-09-19 实测 200；缺参数报「缺少用户信息类型!」）
- **一卡通接口（2026-09-19 实测）**：余额/卡信息 `GET /berserker-app/ykt/tsm/queryCurrentCard`、`queryCard?account=`（`db_balance`+`unsettle_amount` 单位**分** → 元；`elec_accamt` 为电控账户金额，即慧新E校侧电费余额）；**消费流水在独立服务** `GET /berserker-search/search/personal/turnover?size&current&account&type=2` → `{code,data:{total,records[]}}`（`tranamt` 分、`typeFrom=="1"` 为收入）；全部需 token
- 鉴权细节：来源参数与头**双份携带**（GET 走 query、POST 走 body，另加同名头 `synAccessSource: app`）；4030 的判定是 **HTTP 401 + `body.code==4030`**（不是 403）；berserker 系信封 `{code,success,data,msg}` 与 charge 系 `{code,message}` **不可共用解析器**
- ⚠️ 一卡通余额两源不同步（门户卡片 28.01 vs 慧新E校 17.51，2026-09-17 同时刻）——客户端以慧新E校实时接口为准

### 3. WebVPN（校外通道）
- `https://webvpn.cwxu.edu.cn/` = **深澜（Srun）WebVPN**（`wengine_vpn_ticket` cookie 已确认）
- 资源代理 URL 加密规则社区有公开实现（HMAC + AES），需按学校部署参数适配

### 4. 正方教务系统（课表数据源，2026-09-17 实测）
- SSO 入口 `https://jwgl.cwxu.edu.cn/sso/lyiotlogin`（CAS 联动已验证，角色按账号自动判定）
- **课表接口（已验证）**：`POST /jwglxt/kbcx/xskbcx_cxXsKb.html?gnmkdm=N2151`，body `xnm=<学年起始年>&xqm=<学期代码：3=第1学期/12=第2学期/16=暑期>`
- 响应 `kbList[]`：`kcmc` 课程名 / `cdmc` 教室 / `jxbmc` 教学班 / `jxb_id` 教学班 ID（自动更新 diff 的匹配键）/ `jc` 节次（"3-4节"）/ `oldzc` **周次十进制位掩码**（bit0=第1周）/ `kcxz` 必修选修 / `khfsmc` 考核方式 / `kczxs` 学时；`xsxx` 学生信息（班级/姓名/学年学期）
- 教师身份课表接口同源待侦察（测试账号在教务为学生角色；M2.5 导入时按账号角色选端点）

## 四、里程碑

### M0 · 项目脚手架 ✅（2026-09-17 完成）
> 实施计划（任务级）：`docs/superpowers/plans/2026-09-17-m0-m1-foundation.md`（M0+M1，2026-09-17 定稿）
- [x] 项目定名 Wxxy-CampusHub，`git init` + 首次提交（2026-09-15）
- [x] Tauri 2 + React 19 + TS + Vite + Tailwind v4 脚手架（`tauri-app/{frontend,src-tauri}` + 根 Cargo workspace；域色 token + shadcn 源码入库）
- [x] Rust 协议核心 crate（`crates/campus-auth` CAS 协议 + `crates/campus-schedule` 课表，均无桌面依赖）
- [x] `tauriApi.ts` IPC 出口 + `CommandResult` 三态契约 + zustand 基建（含悬浮 Dock 导航 8 面板）
- [x] CodeWiki 初始化（10 篇文章）＋ CHANGELOG.md
- 验收 ✅：`tauri dev` 起真实窗口渲染登录页（截图 `docs/verify/`），命令面注册表就位（7 条命令）

### M1 · CAS 登录 + 账号管理 ✅（2026-09-17 完成，真实账号端到端打通）
- [x] CAS 登录协议逆向与实测（端点/参数/RSA/验证码形态全定案，probe.js 端到端验证 2026-09-17）
- [x] CAS 账密登录全流程（REST `/v1/tickets` + 手动跟随 SSO 302 链 + RecordingJar 会话捕获；live 测试通过）
- [x] 验证码自动识别（颜色不变强度图 + bbox 锚定 + NCC 模板匹配，**100%/0 自信错误**；失败自动换图 ≤3 张）
- [x] 多账号密码加密存储（DPAPI 裸 FFI + accounts.json）＋ 已存账号一键免密重登（切换/重命名 UI 归 M2 设置页）
- [x] 登录态检测（session.json DPAPI 持久化 + 启动 check_session + 失败清会话降级；定时保活轮询归 M5）
- 验收 ✅：真实账号 live 端到端通过（ST/TGT 签发 → 门户三 cookie → Alive）；日志用户名打码、密码永不落盘；`docs/verify/` 有界面截图

### M2 · 门户核心页 ✅（2026-09-18 完成，五页全部接通真实数据）
- [x] **首页（v2 精简版，2026-09-17 用户拍板）**：问候 + 钱包三卡（一卡通/邮箱/图书）+ 下一节课横幅（依赖课表，无数据时隐藏）+ 固定快捷动作 4-6 个；**不含**资讯/快捷应用/待办/会议/信息服务/课表卡片——各模块独立成页并靠 M5 系统通知触达（2026-09-18 批次 1 交付）
- [x] 资讯页：分类 rail + 列表 + **内嵌正文**（抓官网 `/info/<栏目>/<id>.htm` 本地渲染，官方是跳静态页）（2026-09-18 批次 2 交付；`content.jsp` 系两栏受官网鉴权门保护，按设计降级为「浏览器打开原文」）
- [x] 待办页：我的待办/已办/申请（首页已移除，独立页）（2026-09-18 批次 2 交付）
- [x] 应用页：常用钉选 + 分组网格（完整名称+部门副标题）+ 打开策略引擎（附录 A 的 A/B/C 分类：CAS 直达/WebVPN 包装/外链）（2026-09-18 批次 3 交付：图标由后端带会话代拉为 data URL；打开策略当前为**协议白名单直开**，WebVPN B 类包装未做——见下方完成情况；同日遗留项批次补**可达性元数据**：`access.rs` 附录 A 实测表代码化，卡片按 access 显示徽标，webvpn 提示后仍打开 / unavailable 只提示不打开，不信任门户 isCas）
- [x] 日程页：周视图默认 + 会议并入（时间/地点/主持人/参会人员）+ 5 类日程过滤（2026-09-18 批次 3 交付：周视图 + 5 类彩色过滤 + 日程详情卡；同日遗留项批次补齐**月视图 + 每日计数角标 + 会议并入**——DJZ 按教学周次构造标题、`SJ` 自然语言时间解析、失败降级与 `[meeting-diag]` 可观测化，见下方完成情况与设计文档附录 H）
- [x] 首页钱包三卡数据对接（2026-09-18 批次 1 **已决并交付**：采用**门户同源结构化接口** `queryAppointCard`——返回 JSON、数据与官方页面一致、零新增鉴权，满足「与官方一致」验收口径；「对接慧新E校实时接口」不再作为 M2 项——慧新E校（校内网）实时直连列为**可选增强，未做**）
- 验收：各页数据与官方一致且修复官方已知缺陷（名称截断/空态占位）；首页无信息墙
- 2026-09-18 完成情况：五页全部接通门户真实数据并通过真机验收（`cargo test --workspace` **102 passed** / 0 failed；明细见 `CHANGELOG.md` M2 批次 1–3 与遗留项条目、设计文档附录 E–H）。原三项裁剪的现状：① **WebVPN B 类应用包装仍未做**——实测网关对未登录请求一律回落（会议代理端点无凭据 GET 返回 302 → 首页；应用域名一律落 CAS 登录页），三种明文包装形式最终 URL 完全相同、网关丢弃目标路径，包装格式在无 WebVPN 会话前提下无法验证，**会话打通 + URL 包装 + A 类 CAS 直达签发整体归 M4**（`open_app` 仍协议白名单直开；本轮已补可达性元数据徽标与分级提示，WebVPN/暂不可用类不再静默直开）；② **日程月视图与每日计数角标已做**（2026-09-18 遗留项批次：`get_schedule_day_counts` 命令 24→25，月视图自然月网格 + 服务端 count 角标，点击日期跳周；角标为全量计数——计数接口无分类参数，如实呈现）；③ **会议卡并入已做**（同日批次：DJZ 按教学周次构造标题实测四组对照、`SJ` 自然语言时间转 24h、解析不出按全天不伪造、失败降级不影响课表与日历、`[meeting-diag]` 失败打点可观测）；④ **慧新E校实时直连未做**（列为可选增强，待用户拍板）。
- 明确不做：邮箱/图书的深度功能（仅卡片数字，点击跳官方）
- **2026-09-19 补记（会话 `sess-36e5d8e0`）**——用户指出「M2 部分内容好像没完成」，核查后确认五页数据接线属实，缺的是三项：
  ① **顶栏命令面板已补做**：M2 清单里**从未列过**此项（属代码注释里的超前标注「命令面板 · M2 接入」，PLAN 未承诺），2026-09-19 按用户裁决实现——`Cmd/Ctrl+K` 呼起，数据源 = Dock 页面项 + 现有 store 动作 + 设置跳转，键盘全程可达；`AppShell.tsx` 的 `aria-disabled` 占位与三处「M2 接入」注释已清除。
  ② **修复「各界面资讯栏目获取失败」**：报错文案 `获取登录信息失败: 响应解析失败: tryLoginUserInfo 缺少 userName`。根因是**门户会话失效被误报成解析失败**——门户用 **HTTP 200 + `data:null` + `meta.statusCode=302`** 表达失效（与匿名响应逐字段相同），客户端只认 `data.userName`、上层又把非 HTTP 错误一律归 `Parse`；同时 `portal_probe` 对死会话误报 Alive（`check_session` 因此不清会话）。已归一为 `PortalNotLogin`（文案「请先登录」）+ `probe` 追加信封判定，失效即清会话引导重登。详见 [[learnings/portal-session-expiry-200-envelope]]。
  ③ 首页钱包三卡与钱包页**统一取慧新E校实时余额**（一卡通实时优先、失败回落门户快照并标注来源），见 §M3。

### M2.5 · 课表（用户点名重点功能）✅
- [x] **教务自动导入**：复用 CAS 会话 SSO 进正方教务 → `POST /jwglxt/kbcx/xskbcx_cxXsKb.html?gnmkdm=N2151`（xnm/xqm，已实测）→ 解析 kbList（课程/教室/教学班/节次/周次位掩码）落库；按账号角色选端点（学生已验证，教师待侦察）
- [x] **source 源隔离**：课程带 `source: 'import' | 'manual'` 标签（界面以【导】角标呈现）；一切自动更新/调课解析只作用于 import 课程，手动课程永不触碰
- [x] **自动对比更新**：定期/手动抓最新课表快照 → diff（匹配键 `课程名+jxb_id`）→ 周次/教室/节次变化仅更新 import 课程 + 生成变更日志；新课程自动加入、消失课程标记停开
- [x] **调课通知自动调整（override 模型，自建）**：M5 公告流命中关键词（调课/调休/停课/补课）→ L1 规则层提取要素（课程名/原周次星期节次/新时间教室）→ L2 置信分级：高置信自动应用（`override.source=notice:<id>`），低置信进「待确认」列表人工一键采纳；通知修订按 id 撤销
- [x] **算法与数据模型移植**（shiguangschedule，Apache-2.0，2026-09-17 完成）：Rust crate `crates/campus-schedule`（`weeks:number[]` 显式周次、开学日↔周次互算、重叠课程分列算法、默认节次常量、正方课表解析器 `zhengfang.rs`、CourseOverride 调课叠加模型）；**12 个单元测试全通过**（golden 锚定真实开学日 2026-09-07→9-17=第2周）；合规三件套（NOTICE.md/上游 LICENSE 副本/THIRD-PARTY-NOTICES.md + 文件头标注）
- [x] 课表页 UI：周视图默认（§5.1 线框）、周次切换器（标记当前周）、课程详情浮层、手动添加课程（颜色自选）、ICS 导出
- 验收 ✅（2026-09-18 真机，真实账号）：
  - **导入 8 门课与教务一致** ✅：`新增 8 · 更新 0 · 停开 0 · 共 8 门`，顶栏「第 2 周 / 共 19 周」，网格按校本 5 大节作息渲染，块位置与节次一致（`ceil(小节/2)`）
  - **模拟调课通知 → 高置信自动生效 + 待确认列表可采纳** ✅：解析「第5周周四3-4节 信息隐藏与取证技术 调整到 D4-305」→ `conf=high` → 采纳后第 5 周出现【调】角标新时段块 + 原时段「已调出」占位；低置信路径（缺要素/多门匹配/无匹配）由 12 个单测覆盖，未在真机逐条点验
  - **手动课程多次自动更新后零变动** ✅：二次导入 `新增 0 · 更新 0 · 停开 0`，手动课程原样保留（标「手动」）
- 完成情况说明（诚实边界）：
  - **教师身份端点未做**：无教师账号可测；学生端点 `xskbcx_cxXsKb` 是唯一实现，教师端点名（`kbcx/jskbcx_cxJsKb.html`）只是命名规律推断，**未验证**
  - **公告流自动订阅未做**（依赖 M5 通知中心）：L1/L2 内核与「粘贴通知文本」入口已可用，M5 接入时复用同一解析路径，不需改内核
  - **鼠标点击链路已真机点验**（2026-09-18 收尾轮补齐）：给 WebView2 开远程调试端口 + CDP 触发真实 DOM 事件，逐项点验导入/详情浮层/手动添加/调课与停课/撤销/删除/作息编辑/ICS 导出/深色模式；仅「旧 localStorage 面板持久化升级场景」未测。方法见 `.codewiki/learnings/tauri-webview-ui-verification.md`
  - **作息表编辑 UI 已做**（§5.1 第 6 条「可编辑」）：课表页「作息」按钮 → 弹层按行编辑各大节起止时间、可增删行，保存落 `config.slots`、可一键恢复本校默认；网格行数不写死、ICS 展开同源
  - **ICS 导出**：交付形态已修正——原前端 Blob 下载在 WebView2 里静默失效（真机点验发现），改为后端写入 `dirs::download_dir()` 并回显路径；真机确认写出 23136 字节、82 个 VEVENT 的合法 iCalendar 文件
  - **命令面 35 条**（M2.5 新增 10 条：`get_timetable` / `import_timetable` / `add_course_manual` / `update_course` / `delete_course` / `parse_notice` / `apply_override` / `revoke_notice` / `export_ics` / `save_time_slots`）

### M3 · 一卡通 + 电费查询 ✅（2026-09-19 完成，会话 `sess-36e5d8e0`）
> 实施计划（任务级，含全部实测接口契约）：`docs/superpowers/plans/2026-09-19-m3-ecard-electricity.md`；模块与坑见 `.codewiki/modules/campus-synjones.md`
- [x] **CAS→慧新E校 SSO 桥接换 token**：`{BASE}/berserker-auth/cas/login/lyCas` 换 ST + 302 链，**token 在落点 URL query `?synjones-auth=`**（实测 2 跳、无 Set-Cookie）；**`targetUrl` 必须指向子应用**——`/campus-card-pc/`、`/charge-pc/pays/{id}` 带 token，`/plat/shouyeUser` 与裸调用**都不带**（默认值据此定为前者）
- [x] **一卡通余额/流水**：卡信息 `berserker-app/ykt/tsm/*`；流水在独立服务 `berserker-search/search/personal/turnover`（`type` 为**收支方向**：1 收入 / 2 支出 / 3 空，**不传即全量**）；余额口径以**电子账户 `elec_accamt`** 为主（`elec` = electronic，**不是电费余额**——实测三处数值一致，推翻旧推断）；一律 `synAccessSource=app` **双份携带**（query/body + 同名头）
- [x] **电费查询页**：片区→校区→楼栋→**手输房间号** → 剩余金额/单价（`/charge/feeitem` 三级链路）；片区口径 = `status==1 && impl_interface` 非空（**恰好 3 条**，同名停用项靠 status 排除）；末级是**输入级**（`flag[4]=='3'`）不是下拉；结果 `map.showData` 键名**恒为「信息」**、值是三片区**格式各异**的自由文本（`map.money`/`iectranamt` 实测不存在）→ 通用字典渲染 + 逗号折行 + 负数标红，**不做文本解构**
- [x] **常用房间绑定**：本地落盘 `%APPDATA%/campushub/electricity_rooms.json`（含三级 path 的 level/code/value/name；同片区同路径 upsert、上限 20）；平台侧 `sceneBind/add` 是写接口，不采用
- [x] **充值（用户 2026-09-19 裁决：官方链接、软件内跳转、统一风格）**：应用内嵌 webview 打开官方缴费页，Rust 端注入 token 与 `localStorage.configs`（缺失会 `JSON.parse(null)` 白屏）+ **`synAccessSource` 改写 hook**（官方页自己硬编码 `pc`，会撞学校 4030 策略弹「服务大厅未授权」）；窗口标题/图标用客户端品牌 + 主色 CSS 覆盖（尽力而为）；**不复刻签名下单**，真实支付在官方页完成
- [x] **token 单活纪律**：全应用共用单一 `SynjonesClient`（进程级 static + 互斥锁串行化），禁止并发 SSO；CAS TGT 隔夜过期（换票 500 且正文含票据，不得回显）
- 验收 ✅（2026-09-19 真机 CDP 点验，`tauri dev` + WebView2 远程调试端口，方法见 `.codewiki/learnings/tauri-webview-ui-verification.md` 与 `tauri-multiwindow-cdp-verification.md`）：
  - 电费三级链路：三片区列出 → 校区(1 项) → 楼栋(1号楼/3号楼/2号楼) → 手输房间号 `101` → 返回「房间号：101 / 剩余金额：-545.70 / 单价：0.5400」，**负数行标红** ✅
  - 常用房间：保存 → 确认落盘 `electricity_rooms.json`（结构完整）→ 列表出现「查余额/删除」✅
  - 钱包页：电子账户余额与 live 实测一致、**1042 条**流水分页、收入 `+`/支出 `-`、时间倒序 ✅
  - 首页钱包卡：一卡通显示**实时值 + 「实时」来源标注**（失败静默回落门户快照，不阻塞首屏）✅
  - 内嵌充值窗口：**登录态**（渲染出官方缴费表单而非登录页）；该窗口调 IPC 被 ACL 拒绝（安全边界验证通过）✅
  - 自动化：`cargo test --workspace` **271 passed / 0 failed**；前端 `tsc --noEmit` + `vite build` 通过 ✅
- 诚实边界：
  - **今日/本月消费无解**：「`statistics/turnover/sum/user`」需五参、多种参数组合实测恒空 → UI 显示「暂不可用」（`count` 返回全时段总额且无视日期参数，不可替代）
  - **主窗口关闭时子窗口是否联动关闭未验**：`core:window:allow-close` 未授予 JS，CDP 触发不了窗口装饰按钮，需人工点一次确认
  - **校外不可用**：内网明文 IP，依赖 M4 的 WebVPN
- 不做：充值金额的签名下单复刻（charge-pc 走 SHA256 签名 + 动态 HTML form 跳转，真实金钱操作留在官方页）
- 命令面：M3 新增 10 条（一卡通 3 + 电费 7），全局 **53 条**（auth 8 / profile 5 / portal 12 / timetable 18 / synjones 3 / electricity 7）

### M4 · 网络智能路由 ⬜
- [ ] 移植 Wxxy-CampusLogin 校园网检测（/18 子网 + Portal 可达性判定；仅移植检测，登录/注销协议不带过来）
- [ ] 深澜 WebVPN 适配：登录（统一认证账号复用 CAS）+ 资源 URL 加密规则实现 + 登录态维持
- [ ] 路由决策层：按目标服务性质分流——公网服务（门户资讯）直连；内网服务校内直连 / 校外自动走 WebVPN；前端无感知
- 验收：断开校园网（校外/手机热点）后电费查询仍可用（走 WebVPN）

### M5 · 通知中心 ⬜
- [ ] 通知基建：Rust 后台定时任务 + 去重（已读游标落盘）+ 系统通知（`tauri-plugin-notification`）+ 应用内通知中心页
- [ ] 门户公告推送：轮询资讯接口，新公告弹系统通知（分类订阅开关）
- [ ] 电费余额提醒：绑定房间定时查余额，低于阈值提醒（阈值可配）
- [ ] 待办事项提醒：轮询待办中心，新待办提醒
- [ ] 托盘常驻（Windows），通知在后台静默工作
- 验收：三类通知各真实触发一次；轮询间隔与失败退避可配置

### M6 · 安卓端 ⬜
- [ ] `android/` 外壳：path 依赖协议核心，补平台探针/状态（对照 Wxxy-CampusLogin 安卓端模式）
- [ ] 前端复刻树（同形 tauriApi + 同名命令），手机/平板双外壳
- [ ] 安卓特性：前台服务通知保活、AndroidKeyStore 加密、开机自启、应用内更新（APK 下载+安装器）
- [ ] 省电约束：无常驻 rAF、系统锁按需持有、巡检频率分档（CodeWiki 教训直接沿用）
- 验收：真机全功能对齐桌面端（托盘/DPAPI 等桌面专属项除外）

## 五、风险与未决事项

| 事项 | 状态 | 应对 |
|---|---|---|
| CAS 验证码具体形态 | **已定案（2026-09-17）**：算术题（两一位数 + - *，本地解析可行，模板匹配方案） | 实测见 `docs/cas-recon/REPORT.md` |
| 深澜 WebVPN URL 加密参数（学校部署的 salt/key） | 待逆向 | 参考社区公开实现 + 抓包对照；M4 内解决 |
| 门户接口无文档，学校升级可能变更 | 长期风险 | 协议层集中封装 + 失败降级提示 |
| `synAccessSource=app` 依赖服务端现状 | 长期风险 | 学校若修复 PC 授权则对 app 来源无影响；该参数保持常量集中管理 |
| **调课通知为自然语言，规则解析可能漏/错** | 中（M2.5 新增） | 两级方案：高置信自动应用 + 低置信进「待确认」人工采纳；override 按 notice id 可撤销；上线初期默认「全部人工确认」观察期 |
| 教师身份课表接口未侦察（测试账号为学生角色） | 低（M2.5） | 导入时按角色选端点；教师端点用教师账号实测补齐 |
| shiguangschedule 移植合规（Apache-2.0） | 低 | 附 LICENSE 副本 + 文件头标注参考与修改；不使用其名号 |
| 内网服务校外连通性依赖 WebVPN 稳定性 | 中 | M4 实测；不可用时明确报错提示 |
| 双端同步无自动化守门（CodeWiki Known Issues #1） | 流程风险 | 沿用"通用改动同一次提交双端各一份"纪律 + `git diff --stat` 自检；M6 起生效 |

## 六、参考资料

- **Windows 前端设计文档：`docs/design/frontend-design.md`（2026-09-17 实机走查 + 设计定稿：痛点清单/功能映射/信息架构/视觉 token/选型）**
- 架构模板：`../Wxxy-CampusLogin/.codewiki/_architecture.md`（协议单点+平台外壳、IPC 契约、省电约束、Known Issues）
- 4030 诊断与平台接口速查：`~/.zcode/cli/memories/.../synjones-platform-4030-diagnosis.md`
- 深澜 WebVPN URL 规则：社区公开实现（srun webvpn url encrypt/decrypt）+ `docs/cas-recon/REPORT.md` 已采集的本校代理 URL 样本
