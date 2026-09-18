# 更新日志

## 2026-09-18 · 外壳视觉重设计 + 游客模式 + 右上角账号系统与头像

- **模块**：`tauri-app/frontend`（设计系统/壳/8 面板/账号与头像 UI）、`tauri-app/src-tauri`（头像与账号命令、登录显示名）、`crates/campus-auth`（门户资料接口）、`docs/`（计划/设计文档/验收截图）
- **起因**：用户反馈三条——前端观感廉价、未登录被登录页锁死主界面、账号无头像；并要求「登录收进右上角账号系统」+ 复查融合门户找遗漏
- **门户复查（真实账号实机）**：顶栏账号菜单=我的账号/上传头像/退出（官方上传头像仍为裸文件框+三行红字，无裁切压缩）；**官方头像可经 `GET /api/upp/userControl/getLoginInfo` → `data.headPortrait` 取回**（base64 PNG，~38KB）；真实姓名经 `POST /tryLoginUserInfo` → `data.userName`/`departmentName`；铃铛=四类消息（系统/办事/资讯/日程）；门户无游客态；首页接口全景（`getPageContent`/`querySimpleInfoCenter`/`querySimpleFlowItems`/`queryAWeekSchedule`/`querySemesterInfo`/`queryBriefMessage`/`queryMyStore` 等）实测 URL 已记入设计文档附录 C（M2 直接复用）
- **游客模式**：`App.tsx` 去掉登录门禁，壳与各面板恒可进入；启动 `check_session` 非阻塞探测；登录入口收敛到账号菜单 / 今日页引导条 / 各页空态按钮，均调同一登录弹层（四态状态机 + CAPTCHA_MANUAL 手动兜底 + 已保存账号免密）
- **右上角账号系统**：`AccountMenu`（未登录：登录 + 已保存账号免密/删除 + 外观 + 设置；已登录：身份头 + 上传头像 / 同步学校头像 / 切换账号 / 外观 / 设置 / 退出）；`LoginDialog`（由 LoginPanel 迁移，全屏页删除）；`AvatarDialog`（拖拽/选择 → Canvas 中心裁方 → 256×256 → JPEG q0.9/PNG，显示「原 X → Y」，512KB 上限）
- **头像三态**：本地 > 官方 > 首字默认；登录后自动同步一次学校头像；游客态不展示账号头像（回落「锡」占位）
- **后端**：新增 5 条命令 `get_avatar` / `set_avatar` / `clear_avatar` / `sync_official_avatar`（无会话→「请先登录」）/ `remove_account`；头像落盘 `%APPDATA%/campushub/profile.json`（明文 base64，非凭据，≤512KB）；`list_accounts` 增 `displayName`；`campus-auth` 新增 `portal_user_profile()` + 纯函数 `extract_user_profile()`（含 4 个离线单测）；`finish_login` 取真实姓名落库，失败回退学号且不抹旧值
- **视觉执行层**（域色 token 与底部 Dock 签名保留不变）：新增字号阶（28/22/18/14/12）、圆角阶（14/10/8）、**域色染色阴影**、`surface-2`/`line-strong`、`:focus-visible` 统一品牌色 outline、`prefers-reduced-motion` 降级、body 域色径向氛围底；新建 `PanelHeader`/`EmptyState`/`Avatar`/`Surface` 共享组件；8 面板重做（今日页头像问候 + 登录引导条 + 5/4/3 非对称钱包卡 + 禁用态快捷动作；其余页「数据接入中」职业化空态，不再裸写"建设中"）
- **根因修复（P0）**：shadcn 语义 token（`--primary` 等）此前只写在 `:root`、未进 Tailwind v4 `@theme`，`bg-primary`/`text-primary-foreground`/`bg-card`/`border-border` 等工具类**从未生成**（主按钮长期渲染为裸文字）；改用 `@theme inline` 映射到域色 token，产物 CSS 已见 `.bg-primary{background-color:var(--color-brand)}`
- **验证**：`cargo test --workspace` → **47 passed / 3 ignored / 0 failed**（基线 35 → 43 → 47）；`tsc --noEmit` → 0 错误；`vite build` 通过；**真机 `tauri dev` 全流程实测**（补上交接报告欠账）：冷启动落游客态主界面 → 账号菜单 → 已保存账号免密登录 → 学校头像自动同步 + 门户真实姓名落库 → 退出登录回落游客态；验收截图 `docs/verify/ui2-*.png` 8 张（游客今日/游客菜单/游客待办/登录弹窗/头像弹窗/登录态今日/登录态菜单/登录态待办）
- **延后**：react-easy-crop 拖拽缩放裁切器、头像上传回学校（需授权）、教学周与真实数据接线（M2/M2.5/M5）

## 2026-09-18 · 交接报告 HANDOFF.md（面向 M2 接手方）

- **模块**：文档（无代码行为改动）：新增 `docs/HANDOFF.md`；`crates/campus-auth/tests/captcha_solve.rs` 注释同步；`.codewiki/` 索引与基线
- **摘要**：为下一阶段（M2 门户各页 / M2.5 课表接线与 UI）接手方补写交接报告，含：
  - 现状坐标：M0/M1 已交付验收、`campus-schedule` 算法内核就绪（12 测试）、M2 未动工（除登录面板外 8 个面板为占位）
  - 仓库与协作现状：无远端、本地 ff 合并（不走 git-merge-push.sh）、`git add` 明确路径、凭据与验证码红线
  - 代码地图（三 crate + tauri 接线 + 前端面板状态表）与分层约定（协议在 crates、IPC 契约 `CommandResult` 三态）
  - 已打通链路可复用事实：CAS 全流程（含手动跟随重定向原因）、验证码方案与三分类指标、正方课表数据源（jwgl 端点/`oldzc` 位掩码）、慧新E校 `synAccessSource=app`
  - 运行命令（全量测试/前端构建/tauri dev 与打包/live 测试/验证码样本重建与评测）、铁律 5 条、已知坑索引（指向 `.codewiki/learnings/`）
  - 下一步切入点（建议先补门户业务接口侦察 + 建 `crates/campus-portal`）、M2.5 剩余项（含 `PanelId` 追加 `timetable` 需同步的三处）、未决风险表
- **附带修正**：`captcha_solve.rs` 两处注释仍写"类内取均值"，而实现早已改为"每类保留最多 `PER_CLASS_LIMIT`(=10) 张样本补丁"（均值会使同类 NCC 落到阈值边缘、正确率实测跌至 54%），注释已同步；CodeWiki `cw index` + `cw meta update` 推进基线至 `4b58765`
- **验证**：`cargo test --workspace` → 35 passed / 3 ignored / 0 failed；`tauri-app/frontend` `npm run build` → 通过（vite 6.4.3，JS 537.91 kB）；`cw status` → up to date
- **遗留**：桌面窗口内用真实账号点一次登录尚未人工确认（live 测试只覆盖 Rust 协议层），已写入交接报告「未决与风险」首条

## 2026-09-17 · M0+M1 交付：脚手架 + CAS 登录闭环（含验证码自动识别，真实账号端到端打通）

- **模块**：`crates/campus-auth/`（新）、`tauri-app/{frontend,src-tauri}`（新）、根 workspace、CodeWiki
- **计划与评审**：`docs/superpowers/plans/2026-09-17-m0-m1-foundation.md`（12 任务）——先经 deepseek-flash 独立评审（5 P0/10 P1/14 P2 全部消化，含 golden 值实测固化、reqwest CookieStore 读回限制、shadcn CLI 行为、beforeDevCommand cwd 等）
- **M0 脚手架**：
  - 根 Cargo workspace（campus-schedule + campus-auth + tauri-app/src-tauri）
  - 前端：Vite 6 + React 19 + TS strict + Tailwind v4；域色 token 系统（10 色 + shadcn 语义映射 + 深色提亮档 + Outfit 数字字体）；`tauriApi.ts` 唯一 IPC 出口 + `CommandResult` 三态契约；AppShell 顶条 + **域色悬浮 Dock 导航**（8 项、framer-motion 域色胶囊/指示条、gsap 磁吸、reduced-motion 降级）；8 面板骨架；shadcn button/input/card/tooltip 源码入库
  - `src-tauri`：AppState（tokio Mutex 锁纪律）+ DPAPI 裸 FFI 账号加密库 + session.json 会话持久化 + Tauri 2 配置（capabilities 最小集、CSP img-src data:）
  - **CodeWiki 初始化**（7 篇架构/模块/概念/决策文章 + 3 篇踩坑记录）
- **M1 登录闭环**：
  - `crates/campus-auth`：textbook RSA（golden 对拍线上 JS，2 组固化向量）、CAS 客户端（kaptcha/login/16 错误码映射/**手动跟随 302 链**/portal_probe）、**RecordingJar**（自实现 CookieStore 记录会话，弥补 reqwest 0.12 内部 Jar 不可读回）
  - **算术验证码自动识别 100%**：颜色不变强度图（`765-Σrgb` 按峰值归一化）+ bbox 锚定 16×14 画布 + NCC(0.88/0.03) + ±1px 位移补偿；**模板集 70/70、holdout 30/30、自信错误 0、拒绝 0**（三分类评测口径）
  - 7 条 Tauri 命令（login/login_manual/login_saved/get_captcha/check_session/logout/list_accounts）+ 重试状态机（仅 WrongCaptcha 重试 ≤3、NOUSER 绝不自动重试、识别失败不消耗 CAS 错误计数）
  - 登录页（四态状态机 + CAPTCHA_MANUAL 手动兜底 + 已存账号免密重登）+ 今日页骨架
- **验证（均为实际输出）**：`cargo test --workspace` 全绿（campus-schedule 12 + campus-auth 12 + campus-hub 11）；clippy 新增代码零告警；`npm run build` 通过；**真实账号 live 测试通过**（`cargo test -p campus-auth -- --ignored cas_live`：自动识别 → CAS 签发 ST/TGT → SSO 回跳 → 门户三 cookie → 会话 Alive）；`npm run tauri dev` 起真实窗口渲染登录页（截图 `docs/verify/`）；浏览器验收 Dock 域色胶囊/深浅主题/面板切换（截图 `docs/verify/`）
- **踩坑记录（写进 CodeWiki learnings）**：验证码识别五个独立根因（彩色灰度丢形/片序错/细笔画空网格/平局误杀/漏乘法分支）、CAS SSO 三坑（重定向中断/明文落点断连/探测误判）、批量图片标注不可靠（改结构化拼图 + 客观特征交叉校验）
- **工程修正**：新增 `tauri-app/package.json`（承载 Tauri CLI——CLI 只从 cwd 及子目录发现 src-tauri）；`.gitignore` 补 dist/ 与样本/拼图 PNG
- **未做（后续里程碑）**：M2 门户各页、M2.5 课表页与调课通知引擎、M3 一卡通/电费、M4 WebVPN 路由、M5 通知中心

## 2026-09-17 · 导航改版：悬浮 Dock 标签栏（用户指定，对齐 CampusLogin）

- **模块**：设计文档 §4/§6/§7
- **摘要**：用户指定页面切换采用 Wxxy-CampusLogin 同款悬浮标签栏。分身取证其 `DockNav.tsx` 实现（fixed bottom-5 玻璃 Dock、纯图标+hover tooltip、framer-motion 弹簧胶囊与圆点指示条、gsap 磁吸放大 scale 1.35、zustand activePanel 切换 + AnimatePresence 过渡 + useDeferredValue、内容区 pb-28 预留），设计文档已按本项目语境改写：
  - 顶栏导航废除，改为底部悬浮 Dock（8 项：今日/资讯/待办/日程/应用/钱包/电费/设置）；顶条仅剩 ⌘K/通知/主题/账号
  - **域色创新**：激活胶囊/指示条/图标用该项域色（非统一色），域色编码系统从内容延伸到导航
  - 依赖增量：framer-motion + gsap（磁吸可裁剪）；签名元素由「域色 spine」更新为「域色 Dock」
- **验证**：取证基于 CampusLogin 源码逐文件核查（组件/样式/交互/依赖均文件:行号级证据）

## 2026-09-17 · 课表核心 Rust 移植（campus-schedule crate，M2.5 首个交付）

- **模块**：Rust 协议核心（`crates/campus-schedule/`，纯逻辑无 Tauri 依赖，为安卓 path 依赖预留）
- **摘要**：按用户指定将 shiguangschedule（拾光课程表，Apache-2.0）的算法与数据模型**移植到 Rust**（原计划 TypeScript，用户改定 Rust）：
  - `model.rs`：Course/CourseTableConfig/TimeSlot（serde camelCase 对齐前端 IPC）+ **CourseOverride 调课叠加模型（自建，上游缺口）**+ CourseSource 导入/手动源隔离 + `expand_week_mask` 正方周次位掩码展开
  - `weeks.rs`：周次计算三函数（Kotlin AppSettingsRepository.kt:138-224 → Rust/chrono），支持自定义周起始日
  - `grid.rs`：time_to_grid_scale/grid_scale_to_time/merge_courses（Kotlin WeeklyScheduleViewModel.kt:324-754 → Rust），重叠分簇+贪心分列
  - `timeslots.rs`：默认 13 节作息常量
  - `zhengfang.rs`：正方课表响应解析器（本项目原创），坏数据跳过、课程名稳定配色
- **验证**：cargo test **12/12 通过**——golden 锚定真实教务数据（开学日 2026-09-07 → 2026-09-17=第 2 周与门户一致）、位掩码 4095→1-12 周、分列（单列/两列/链式复用/非本周淡化）、正方样例解析
- **合规（Apache-2.0）**：`crates/campus-schedule/NOTICE.md`（上游逐文件映射+修改说明）、上游 LICENSE 副本（LICENSE-shiguangschedule.txt）、根 `THIRD-PARTY-NOTICES.md`、被移植文件头「Adapted in part from … Modified」标注；不使用上游名号
- **上游参考仓库**：完整克隆至工作区 `../shiguangschedule`（独立仓库，不进本项目 git）
- **待办（M2.5 后续）**：调课通知 L1 规则解析引擎、自动更新 diff、教师课表端点、ICS 导出、Tauri 命令层接线

## 2026-09-17 · 首页 v2 定稿 + 课表功能立项（M2.5，用户拍板）

- **模块**：PLAN.md / 设计文档（§4/§5/§5.1/§11）
- **首页 v2（用户指定精简）**：去掉资讯/快捷应用/待办/会议/信息服务/课表卡片六模块，只留问候+钱包三卡+下一节课横幅+固定快捷动作；各模块独立成页靠 M5 通知触达
- **课表立项 M2.5（推翻原"暂不做"）**，五条需求逐条落计划：
  - 教务自动导入（正方接口 `xskbcx_cxXsKb.html` 2026-09-17 实测通过，含周次位掩码解析）
  - 调课通知自动调整（L1 规则提取 + L2 置信分级自动/待确认，override 模型自建、可按通知 id 撤销）
  - 自动对比更新（快照 diff，匹配键课程名+jxb_id，仅动 import 课程）
  - 导入课程 source 标签隔离（手动课程零触碰）
  - 算法/数据模型移植 shiguangschedule（分身评估：Kotlin/Compose 不可直接复用，移植周次计算/分列算法/weeks 显式列表，Apache-2.0 合规清单）
- **侦察新增**：PLAN.md §3.4 正方教务系统（SSO 入口/课表接口/响应字段）；慧新E校 sessionStorage 缺陷与两源余额偏差入侦察结论
- **风险表新增**：调课通知自然语言解析风险（上线初期默认全人工确认观察期）、教师课表接口待侦察、Apache-2.0 合规
- **验证**：课表接口真实登录态实测（8 门课全字段）；shiguangschedule 评估基于 GitHub 仓库文件树+源码逐文件核查

## 2026-09-17 · 走查遗漏补测（设计文档附录 B，8 项）

- **模块**：设计（`docs/design/frontend-design.md` 附录 B）
- **摘要**：复查发现 8 处未覆盖项并全部补测：资讯正文（新开标签跳官网静态页 → 内嵌阅读优化点坐实）、会议详情页（主持人/参会人员字段）、订阅管理（60 栏目池+拖拽排序）、顶栏全局搜索（检索中心+热搜榜）、应用详情页（确认不存在）、CAS 自助服务三 tab（改密走官方，客户端外链）、慧新E校「我的」页 + 退出流程（单点登出）
- **新痛点**：慧新E校 token 存 sessionStorage 不跨标签页，新开标签必掉登录（客户端后端持 token 规避）；一卡通余额门户 28.01 vs 慧新E校 17.51 两源不同步（客户端以实时源为准）
- **额外情报**：热搜榜暴露未上架的 OA 系统（v1 不覆盖）
- **验证**：全部基于真实登录态浏览器走查

## 2026-09-17 · 应用中心 30 应用逐站 SSO 实测（设计文档附录 A）

- **模块**：设计（`docs/design/frontend-design.md` 附录 A）
- **摘要**：补测应用中心全部 30 个应用（上轮仅测了门户自身页面）：从 `/api/upp/appStore/v2/queryApp` 接口拉取全量清单（含 appLink/isCas 元数据），浏览器逐站访问记录最终落点：
  - **A 类 CAS 直达可用 ~17 个**：教务系统（正方）、创新创业、超星泛雅、whall gemini 表单组（心理预约/请假/贷款/监控/报告厅）、办事大厅、yd.cwxu.edu.cn 表单组（邮箱/报修/漏洞单）、一卡通 SSO 桥、校园一键通、电子资源、万方等
  - **B 类需 WebVPN 会话 7 个**：教学质量保障/财务系统/知网镜像/IEEE/ScienceDirect/SCIE/图书馆空间管理（域名仅经深澜网关可达，M4 打通后自动可用）
  - **C 类异常 4+**：毕业论文系统标记 cas 实则 SSO 断；联创文印/馆藏数字化死链；两个学生表单教师账号 403（权限问题）
  - **关键结论**：官方 `isCas` 字段不可信，客户端需自建 `appAccess.json` 可达性元数据与 A/B/C 打开策略引擎；顺带采集门户 M2 全部数据接口清单（应用/资讯/待办/课表/会议/邮箱卡/消息）
- **验证**：逐站真实访问（ticket 签发链路+落点判定），无凭据操作

## 2026-09-17 · 门户实机走查 + Windows 前端设计文档（M2 前置）

- **模块**：设计（`docs/design/frontend-design.md`）
- **摘要**：真实账号浏览器走查官方门户全部页面（首页/应用中心/待办中心/资讯中心/日程中心/消息中心/个人菜单/上传头像）+ SSO 桥进慧新E校，产出完整设计文档：
  - **痛点清单 10 项分级**（P0：头像上传零辅助/账号管理割裂/重置密码明文进消息流；P1：信息墙无重点/无推送/资讯扫描性差等）
  - **功能重设计映射表**：官方 16 项功能 → 锡院助手设计（含头像上传三步弹层：1:1 裁切 + Canvas 压缩 ≤200KB 默认 jpg）
  - **信息架构**：7 项顶栏导航 + Ctrl+K 命令面板 + 托盘常驻；「今日」页线框
  - **视觉 token 初稿**：锡院紫品牌锚 + 域色编码系统（钱包绿/资讯紫/待办琥珀/日程湖蓝）+「域色 spine」签名元素；Segoe UI + Outfit 数字字体
  - **选型（分身 GitHub 实测）**：Tailwind v4 + shadcn/ui + lucide-react（124k），规避 AntD/Arco/Semi；裁切 react-easy-crop + Canvas 压缩（仅 1 新依赖）；布局对标 Spacedrive/Cap（Tauri+React 同款栈）
- **验证**：走查基于真实登录态（截图+DOM 快照取证）；选型数据来自 GitHub REST API 实测（star/pushed_at，2026-09-17）
- **隐私**：走查截图含个人信息，一律不入库（仅文档文字记录）

## 2026-09-17 · CAS 真实账号端到端登录 + WebVPN 联动登录验证（M1/M4 侦察）

- **模块**：CAS 登录协议（`docs/cas-recon/`）
- **摘要**：在协议逆向基础上完成真实账号端到端验证：
  - **CAS 真实登录成功**：`POST /v1/tickets` 响应顶层即 `{"tgt":"TGT-...","ticket":"ST-..."}`（无 data 包裹、无 Set-Cookie——CASTGC 由前端 JS 写入，客户端可忽略）
  - **门户 SSO 成功**：shiro-cas 验票 302 后种下 `customsid`（Shiro 会话）/`Authorization`（门户 API 令牌）/`rememberMe`；302 目标为 http:// 明文，客户端应替换 https
  - **WebVPN（深澜 Srun）联动登录成功**：CAS `service=https://webvpn.cwxu.edu.cn/login?cas_login=true` → 初始 `wengine_vpn_ticket` → ticket 回跳 → `wengine-vpn-token-login` 一次性 token → WebVPN 会话建立（首页复查不再跳 /login）
  - 深澜代理 URL 活样本已采集（`/https/77726476706e69737468656265737421<加密hex>/...`，两主机样本入库），为 M4 URL 加密逆向铺路
- **交付物**：`cas.js`（CAS 公共库）、`webvpn.js`（WebVPN 探针）、probe.js 增强（--creds 凭据文件读取、SSO 重定向链验证）、REPORT.md 补真实登录与 WebVPN 章节
- **安全**：凭据经文件读取（`--creds`），命令行/日志/响应均不落明文；`账号与密码.txt` 已加入 .gitignore
- **验证**：真实账号两次探针全部通过（门户会话 Cookie + WebVPN 会话 Cookie 判定）
- **PLAN.md**：侦察结论补 ⑥⑦ 两条（真实登录、WebVPN 联动）；M4 深澜素材就绪

## 2026-09-17 · CAS 统一身份认证登录协议逆向与实测（M1 侦察）

- **模块**：CAS 登录协议（`docs/cas-recon/`）
- **摘要**：逆向 CAS 前端 JS（app.2fb1f8a1ec5d2342de95.js），完整还原账密登录协议并实测打通：
  - 流程：`GET /lyuapServer/kaptcha`（算术题验证码，uid+PNG）→ `POST /lyuapServer/v1/tickets`（username / password=RSA密文 / service / id=验证码uid / code=答案，头 `token=RSA("lyasp"+时间戳)`）→ 响应直含 ST/TGT → `service?ticket=ST` 完成门户 SSO
  - 密码加密：textbook RSA 1024 位（e=010001，little-endian 组块 126 字节，无 padding，hex 不补零）；验证码形态定案为算术题（两一位数 +/-/*，可纯本地识别）
  - 实测：假账号+正确验证码 → `NOUSER`（验证码/加密/字段全部通过），错误验证码 → `CODEFALSE`（对照）；全程无 Cookie 依赖
- **交付物**：`docs/cas-recon/REPORT.md`（协议报告）、`probe.js`（端到端探针）、`rsa30.js`（线上 RSA 模块原样提取，Rust 实现对照基准）、`extract-rsa.js`（提取脚本）
- **验证**：node probe.js 实测（HTTP 200 + 响应 JSON 判定）；RSA 对照测试 3/3 一致
- **PLAN.md**：侦察结论 CAS 段重写为已定案；风险表验证码形态结项；M1 勾选侦察项
- **待办**：真实账号跑一次 probe（用户侧执行），随后进入 M1 Rust 协议核心实现
- 备注：CodeWiki 未初始化（尚无源码，M0 脚手架时 `cw init`）
