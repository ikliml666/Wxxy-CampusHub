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
- 门户→慧新E校 SSO 桥：`http://10.3.100.110/berserker-auth/cas/redirect/lyCas?targetUrl=...`
- ⚠️ **4030 故障关键结论**：服务端按请求参数 `synAccessSource` 做来源授权，`pc` 来源校验失败（当前服务端配置问题），**`app` 来源放行**——本客户端调 berserker 系接口一律带 `synAccessSource=app`
- **电费查询直连（匿名可用，已实测）**：`http://10.3.100.110/charge-pc/pays/450`（校区→楼栋→房间→确认信息，返回剩余金额与单价；`450` 为电费项目 feeitemid，来自 appScheme 数据 `appCode:electricity`）
- 应用方案接口 `GET /berserker-app/appScheme/info` 可拿到全部服务应用列表
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

### M0 · 项目脚手架 ⬜
> 实施计划（任务级）：`docs/superpowers/plans/2026-09-17-m0-m1-foundation.md`（M0+M1，2026-09-17 定稿）
- [x] 项目定名 Wxxy-CampusHub，`git init` + 首次提交（2026-09-15）
- [ ] Tauri 2 + React 19 + TS + Vite + Tailwind 脚手架（目录结构对照 Wxxy-CampusLogin：`src-tauri/{commands,auth,infra,config,network,...}` + `frontend/src/{hooks,components,shared,...}`）
- [ ] Rust 协议核心 crate 骨架（为安卓 path 依赖预留：无桌面依赖约束）
- [ ] `tauriApi.ts` IPC 出口 + `CommandResult` 三态契约 + zustand 基建
- [ ] CodeWiki 初始化（`cw` 全量编译）；CHANGELOG.md
- 验收：`tauri dev` 双端（先只 Windows target）空窗口跑通，命令面注册表就位

### M1 · CAS 登录 + 账号管理 ⬜
- [x] CAS 登录协议逆向与实测（端点/参数/RSA/验证码形态全定案，probe.js 端到端验证 2026-09-17）
- [ ] CAS 账密登录全流程（`/lyuapServer/login` 表单流程 + 会话 cookie 管理）
- [ ] 验证码自动识别（形态确认后选方案：算术解析 / 轻量本地识别），失败自动重试
- [ ] 多账号管理 + 密码加密存储（Windows 先 DPAPI，对齐 Wxxy-CampusLogin `account::crypto` 模式）
- [ ] 登录态保活与过期检测（门户 cookie + CAS TGT）
- 验收：命令行/界面完成登录，掉线自动检测，敏感信息日志脱敏

### M2 · 门户核心页 ⬜
- [ ] **首页（v2 精简版，2026-09-17 用户拍板）**：问候 + 钱包三卡（一卡通/邮箱/图书）+ 下一节课横幅（依赖课表，无数据时隐藏）+ 固定快捷动作 4-6 个；**不含**资讯/快捷应用/待办/会议/信息服务/课表卡片——各模块独立成页并靠 M5 系统通知触达
- [ ] 资讯页：分类 rail + 列表 + **内嵌正文**（抓官网 `/info/<栏目>/<id>.htm` 本地渲染，官方是跳静态页）
- [ ] 待办页：我的待办/已办/申请（首页已移除，独立页）
- [ ] 应用页：常用钉选 + 分组网格（完整名称+部门副标题）+ 打开策略引擎（附录 A 的 A/B/C 分类：CAS 直达/WebVPN 包装/外链）
- [ ] 日程页：周视图默认 + 会议并入（时间/地点/主持人/参会人员）+ 5 类日程过滤
- [ ] 首页钱包三卡对接慧新E校实时接口（门户卡片数据有缓存偏差）
- 验收：各页数据与官方一致且修复官方已知缺陷（名称截断/空态占位）；首页无信息墙
- 明确不做：邮箱/图书的深度功能（仅卡片数字，点击跳官方）

### M2.5 · 课表（用户点名重点功能）⬜
- [ ] **教务自动导入**：复用 CAS 会话 SSO 进正方教务 → `POST /jwglxt/kbcx/xskbcx_cxXsKb.html?gnmkdm=N2151`（xnm/xqm，已实测）→ 解析 kbList（课程/教室/教学班/节次/周次位掩码）落库；按账号角色选端点（学生已验证，教师待侦察）
- [ ] **source 源隔离**：课程带 `source: 'import' | 'manual'` 标签（界面以【导】角标呈现）；一切自动更新/调课解析只作用于 import 课程，手动课程永不触碰
- [ ] **自动对比更新**：定期/手动抓最新课表快照 → diff（匹配键 `课程名+jxb_id`）→ 周次/教室/节次变化仅更新 import 课程 + 生成变更日志；新课程自动加入、消失课程标记停开
- [ ] **调课通知自动调整（override 模型，自建）**：M5 公告流命中关键词（调课/调休/停课/补课）→ L1 规则层提取要素（课程名/原周次星期节次/新时间教室）→ L2 置信分级：高置信自动应用（`override.source=notice:<id>`），低置信进「待确认」列表人工一键采纳；通知修订按 id 撤销
- [x] **算法与数据模型移植**（shiguangschedule，Apache-2.0，2026-09-17 完成）：Rust crate `crates/campus-schedule`（`weeks:number[]` 显式周次、开学日↔周次互算、重叠课程分列算法、默认节次常量、正方课表解析器 `zhengfang.rs`、CourseOverride 调课叠加模型）；**12 个单元测试全通过**（golden 锚定真实开学日 2026-09-07→9-17=第2周）；合规三件套（NOTICE.md/上游 LICENSE 副本/THIRD-PARTY-NOTICES.md + 文件头标注）
- [ ] 课表页 UI：周视图默认（§5.1 线框）、周次切换器（标记当前周）、课程详情浮层、手动添加课程（颜色自选）、ICS 导出
- 验收：真实账号一键导入 8 门课与教务一致；模拟调课通知 → 高置信自动生效 + 待确认列表可采纳；手动课程在多次自动更新后零变动

### M3 · 一卡通 + 电费查询 ⬜
- [ ] CAS→慧新E校 SSO 桥接换 token（`berserker-auth/cas/redirect/lyCas` 流程）
- [ ] 一卡通余额/流水（`berserker-app` 接口，全部带 `synAccessSource=app`）
- [ ] 电费查询页：校区/楼栋/房间 → 剩余金额/单价（charge-pc 直连 + 余额查询接口）
- [ ] 常用房间绑定（多房间记忆，为 M5 电费提醒铺路）
- 验收：查询结果与官方渠道一致；校内全程不需浏览器

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
