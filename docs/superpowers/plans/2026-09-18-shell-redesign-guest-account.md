# 计划：外壳视觉重设计 + 游客模式 + 右上角账号系统与头像

> 制定：2026-09-18（会话 sess-c2ed9b35）· 分支 `feat/sess-c2ed9b35-ui-shell-redesign`
> 依据：用户需求（设计丑 / 未登录不得锁死主界面 / 登录收进右上角 / 缺头像）+ `docs/design/frontend-design.md` + 本轮融合门户实机复查（2026-09-18）

## 一、目标与验收

| # | 目标 | 验收证据 |
|---|---|---|
| 1 | **游客可进主界面**：启动不再被登录页拦截，未登录直接进壳与各面板 | `tauri dev` 冷启动落到主界面；断网/无会话同样不阻塞 |
| 2 | **登录收进右上角账号系统**：登录入口、账号信息、切换账号、退出都在顶栏头像下拉里 | 顶栏头像 → 菜单项可完成登录/切换/退出；登录表单以弹层出现（CAPTCHA_MANUAL 手动兜底保留） |
| 3 | **头像落地**：默认头像（首字）、本地上传（裁切压缩）、官方头像同步（getLoginInfo.headPortrait）三态 | 顶栏与账号菜单显示头像；上传后立即刷新；登录后「同步学校头像」拉回真实头像 |
| 4 | **视觉重做**：不再"平白空"——排版分级、表面层次、状态齐备、空态设计化 | 截图对照（游客态 + 登录态各面板）；`npm run build` 与 `cargo test --workspace` 通过 |
| 5 | **门户复查缺口回填**：本轮门户实机取的账号/头像/消息/接口事实写回设计文档 | `docs/design/frontend-design.md` 附录补测表更新 |

**非目标**（不在本次）：门户业务页数据接线（M2）、课表页（M2.5）、通知中心（M5）、react-easy-crop 拖拽缩放裁切器（见 §六 延后项）。

## 二、本轮门户实机复查结论（2026-09-18）

1. **顶栏账号菜单**（`my.cwxu.edu.cn` 右上角）：头像 + 问候语 → 下拉 **我的账号 / 上传头像 / 退出**。官方上传头像弹窗仍为裸文件框 + 三条红字（≤200KB、png/jpg、建议 1:1）+ 删除头像，**无裁切/压缩/预览**（设计文档 P0 痛点 #1 依然成立）。
2. **官方头像可取回**：`GET /api/upp/userControl/getLoginInfo` → `data.headPortrait`（base64 PNG data URL，实测约 38KB）。客户端登录后可直接同步，无需用户重传。
3. **消息铃铛**：未读数 + 全部标记已读 + 四类（系统/办事/资讯/日程）彩色圆标——与我们的域色编码同源，M5 可对齐。
4. **门户无游客浏览**（未登录即跳 CAS；`isGuest` 仅配置位）→「游客进主界面」是客户端独有设计，各面板空态需自定。
5. **门户首页接口全景**（本轮 resource timing 实测，M2 直接复用）：
   - `GET /api/upp/layout/getPageContent?pageId=…` 首页布局
   - `GET /api/uppinfo/infoCenter/querySimpleInfoCenter?pageNum&pageSize&columnIds=…` 资讯（columnIds=9 通知公告；其余栏目 id 已见：f382fddd…／a8bc1e5a…／ea0a5b21…／4f5a7ccb…／5d2c45d2…／d4901da2…）
   - `GET /api/uppflow/affairCenter/queryTabItems?isCount=1` + `GET /api/uppflow/process/querySimpleFlowItems?tabId=todo|done&pageSize&pageNum` 待办三区
   - `GET /api/uppcard/kbsz/queryAWeekSchedule?cardId=…` 课表（首页卡片含「第2周」切换、课程块=课程名,教室,教学班,教师）
   - `GET /api/upp/config/querySemesterInfo` 学期；`GET /api/uppmessage/message/queryBriefMessage`、`/api/uppmessage/polling/getLocalAlert` 消息
   - `GET /api/upp/appStore/queryMyStore?excludeMobile=1` 应用中心；`GET /api/uppcard/serviceTypeShow/selectAppByCardId?cardId=…` 卡片内容；`GET /api/upp/contentDisplay/queryAppointCard/<id>`；`GET /api/upp/extCardAttachment/download/<id>` 附件
   - `GET /api/upp/userControl/getLoginInfo` 登录信息（userId/headPortrait/firLogin/guideUsed/strategy）

## 三、视觉方向（设计技能落地口径）

技能：`hallmark`（redesign 模式，主打）+ `redesign-existing-projects`（审计升级）。**保留**项目已锁定的域色 token 与 Outfit 数字字体（hallmark 预检：palette/font 不推翻），**替换**执行层。

**诊断（现状为什么"丑"）**：单一灰底 + 白色平板卡（border+无投影语言）→ 无层次；h2 20px 一档字体 → 无排版分级；顶栏只有 3 个小图标 → 无产品身份；七个面板是「建设中」文字 → 无空态设计；数字未走 Outfit/表格数字；主题切换是裸日月图标（模板化 AI 指纹）。

**处方**（全部走 token，不许硬编码颜色）：

| 维度 | 规格 |
|---|---|
| 排版 | 新增字号阶：`display` 28/1.15/-0.02em·600、`title` 18/1.3·600、`body` 14、`caption` 12/1.4/+0.01em；数字一律 `tabular-num`（Outfit） |
| 表面 | `--color-bg` 微调为 #f6f5f9；新增 `--color-surface-2`（内层面板）、`--color-line-strong`；卡片圆角 14 / 内层 10 / 控件 8；阴影**带域色染色**（`0 1px 2px rgb(31 27 41/0.04), 0 8px 24px -12px rgb(91 46 144/0.18)`） |
| 氛围 | body 顶部一层极淡域色径向洗（≤5% 透明）+ 可选 2% 噪点覆盖（纯 CSS/SVG data URI，不加资源文件） |
| 顶栏 | 高 56px 常驻：品牌块（28px 圆角方 锡 标 + 「锡院助手」）· 搜索胶囊（icon + 文案 + ⌘K kbd，hover 抬升）· 右侧：铃铛（带未读点，tooltip 标「通知中心 · M5」）→ 账号区（头像 + 名 + chevron） |
| 账号菜单 | 未登录：头像占位 + 「未登录」+ 一句说明 + **登录**主按钮 + 已保存账号快捷登录 + 外观开关 + 设置；已登录：头像 40px + 姓名/学号 + 上传头像 / 同步学校头像 / 切换账号（含移除）/ 外观开关 / 设置 / 退出登录 |
| 头像 | `Avatar` 组件三态优先级：本地 > 官方 > 首字默认（域色渐变底）；尺寸 24/32/40/64；圆形 + 1px 内环 |
| 空态 | `EmptyState` 组件：域色淡底圆形图标 + 标题 + 一句指引 + 动作槽；游客态文案「登录后查看…」+ 登录按钮；已登录未接线模块用「数据接入中」职业化占位（不再裸写"建设中"） |
| 状态 | 交互元素补齐 default/hover/focus-visible/active/disabled/loading；`prefers-reduced-motion` 下弹层只保留透明度过渡；焦点环 ≥3:1 |
| 动效 | 只保留三处：面板弹簧切换（已有）、Dock 磁吸与域色胶囊（已有）、弹层进场（160ms opacity + y:6 + scale .98）；无滚动特效、无弹跳 |

**自评（防模板化）**：不属于 AI 默认三型；域色系统编码语义（钱/事/讯/程）而非装饰；不做「三列等宽卡片」式模板排布（今日页钱包卡走 5/4/3 非对称宽度）；标题一律正向（无斜体），不用 `01 ·` 式编号眉标。

## 四、冻结契约（所有分身必须遵守）

**IPC 命令**（一律 `CommandResult { success, message?, data? }`，camelCase）：
- 新增：`get_avatar` / `set_avatar` / `clear_avatar` / `sync_official_avatar` / `remove_account`
- 变更：`list_accounts` → `{ accounts: [{ username, lastLogin, displayName? }] }`
- 头像数据形状（四个命令统一返回）：`{ imageBase64: string|null, source: "local"|"official"|null }`
- 头像落盘：`%APPDATA%/campushub/profile.json`，`{ localBase64?, officialBase64?, officialFetchedAt? }`；**clamp 单图 ≤512KB base64**，超限 `CommandResult::err`

**前端 store 契约**：
- `stores/uiStore.ts`（持久化键不变 `campushub-ui`）：新增 `loginDialogOpen` / `openLoginDialog` / `closeLoginDialog`、`avatarDialogOpen` / `openAvatarDialog` / `closeAvatarDialog`（均不持久化）；`displayName` 字段**移除**（迁移到 authStore）
- `stores/authStore.ts`（持久化键 `campushub-auth`，仅 partialize 落 `{username, displayName}`）：
  ```ts
  type AuthStatus = "unknown" | "guest" | "authed";
  interface AccountInfo { username: string; lastLogin?: string; displayName?: string }
  interface AuthState {
    status: AuthStatus; username: string | null; displayName: string | null;
    avatarBase64: string | null; avatarSource: "local" | "official" | null;
    accounts: AccountInfo[];
    checkSession(): Promise<void>;
    refreshAccounts(): Promise<void>;
    refreshAvatar(): Promise<void>;
    login(u: string, p: string): Promise<CommandResult<unknown>>;      // 透传，UI 自行处理 CAPTCHA_MANUAL
    loginManual(args): Promise<CommandResult<unknown>>;
    loginSaved(username: string): Promise<CommandResult<unknown>>;
    logout(): Promise<void>;
    uploadAvatar(base64: string): Promise<CommandResult<unknown>>;
    syncOfficialAvatar(): Promise<CommandResult<unknown>>;
    clearAvatar(): Promise<CommandResult<unknown>>;
    removeAccount(username: string): Promise<CommandResult<unknown>>;
  }
  ```
- 面板读登录态只用：`useAuthStore((s) => s.status)` / `displayName` / `avatarBase64`；触发登录只用 `useUiStore((s) => s.openLoginDialog)()`

**组件契约**：`PanelHeader{ title, description?, domain?: "brand"|"info"|"todo"|"sched"|"wallet"|"neutral", actions?: ReactNode }`；`EmptyState{ icon?, domain?, title, hint?, action?, compact? }`；`Avatar{ src?, name?, size?: "sm"|"md"|"lg"|"xl", className? }`；`Surface{ accent?, hover?, className?, children }`（卡片容器，`accent` 为域色 CSS 变量名）

## 五、任务分解（分批派发，单批 ≤2 分身）

### 批次 A（并行，文件不重叠）
**A1 · 设计系统与共享组件**（glm5-3-flash）
- `src/index.css`：token 扩展（字号阶/圆角/染色阴影/`surface-2`/`line-strong`/焦点环/降级动效）+ body 氛围底 + 噪点类；**保留**现有全部颜色 token 名
- `src/components/ui/{button,input,card}.tsx`：按新 token 补状态（hover/active/focus-visible/disabled/loading）
- 新建 `src/components/{PanelHeader,EmptyState,Avatar,Surface}.tsx`（按 §四 契约）
- `src/stores/uiStore.ts`：按 §四 增删字段
- 验收：`npm run build` 通过

**A2 · 后端头像与账号命令**（glm5-3-flash）
- 新建 `src-tauri/src/commands/profile.rs`：`get_avatar` / `set_avatar` / `clear_avatar`（profile.json 读写，**不用 DPAPI**——头像非凭据，明文 base64 落盘即可）+ `sync_official_avatar`（无会话 → err「请先登录」）
- `crates/campus-auth/src/cas.rs`：新增门户 API 取回方法（`portal_login_info()` 或等价的 `portal_get_json(path)` + 解析），从 `getLoginInfo.data.headPortrait` 取头像；带离线单测（fixture JSON，不打网络）
- `src-tauri/src/commands/auth.rs`：`list_accounts` 增 `displayName`；新增 `remove_account`
- `src-tauri/src/lib.rs`：注册 5 条新命令
- 验收：`cargo test --workspace` 通过（新增单测：头像存取轮转、超限拒绝、无会话同步报错）

### 批次 B（并行，文件不重叠，依赖 A 的契约与组件）
**B1 · 壳与账号系统**（glm5-3-flash）
- `src/App.tsx`：**去掉登录门禁**，AppShell 恒渲染；挂载时 `checkSession()`（非阻塞）；`LoginDialog` 挂根节点
- `src/components/AppShell.tsx`：新顶栏（品牌/搜索/铃铛/账号区）；保留面板路由与 AnimatePresence
- 新建 `src/components/AccountMenu.tsx`（§三 菜单规格，含键盘 Esc/点外关闭、aria-haspopup/expanded、外观开关）
- 新建 `src/components/LoginDialog.tsx`：由现 `panels/LoginPanel.tsx` 迁移（四态状态机 + CAPTCHA_MANUAL 手动兜底 + 已保存账号快捷登录 + 焦点入首个输入框 + aria-live 错误）；迁移后**删除** `panels/LoginPanel.tsx`
- 新建 `src/components/AvatarDialog.tsx`：选文件 → Canvas 中心裁方 → 256×256 → JPEG q0.9（含透明通道用 PNG）→ 显示「原 X → Y」→ 保存；错误（非图片/超限）内联提示
- 新建 `src/stores/authStore.ts`（§四 契约）
- 验收：`npm run build` 通过

**B2 · 面板视觉与游客空态**（glm5-3-flash）
- 8 个面板（除 LoginPanel）统一 `PanelHeader` + `Surface`；游客态 `EmptyState`（「登录后查看…」+ 登录按钮）；已登录未接线态「数据接入中 · 门户模块开发中」
- `TodayPanel` 重做：头像 + 问候（authStore.displayName，游客显示「同学」）+ 日期行（`YYYY-MM-DD 周X`，教学周待 M2.5 数据到位再补）+ 钱包三卡（`—` 占位、tabular 数字、域色 spine、hover 抬升）+ 快捷动作（可落地项正常跳转，未落地项 disabled + tooltip「门户接入后开放」，游客点击走 openLoginDialog）+ 游客登录引导条（单行，可关闭）
- `SettingsPanel`：外观（深浅）可用 + 账号区（头像行调 openAvatarDialog、账号列表、登录/退出）+ 关于（版本/数据目录说明）
- 验收：`npm run build` 通过

### 批次 C（主智能体验证）
1. `cd tauri-app/frontend && npm run build`；`cargo test --workspace`
2. `tauri dev` 真机：冷启动 → 主界面（游客态）截图 → 顶栏账号菜单 → 登录弹窗 → 真实账号登录 → 头像「同步学校头像」→ 登录态截图（这一步同时补上 HANDOFF「GUI 真机登录尚未人工确认」的欠账）
3. 截图归档 `docs/verify/`，逐张目视验收（排版/层次/空态/状态）

### 批次 D（主智能体收尾）
设计文档附录更新（本轮门户复查）、`CHANGELOG.md` 一条、`.codewiki/` 增 `decisions/`+`learnings/` 并按需更新模块文章 → `cw index` + `cw meta update` → 明确路径 `git add` 提交 → 本地 `git merge --ff-only` 进 master（本仓库无远端，不用 git-merge-push.sh）

## 六、延后项（明确不做，留痕）

- **react-easy-crop 拖拽/缩放裁切器**：本次头像上传用零依赖「中心裁方 + 缩放」，新依赖与交互升级待用户点名再做（设计文档 §3.1 的完整裁切器仍是目标形态）
- 官方头像**上传回学校**（官方 ≤200KB/1:1 约束已确认）：读方向已做（同步），写方向涉及改动校方资料，需用户明确授权
- 教学周/课表数据、铃铛真实消息列表：等 M2/M2.5/M5

## 七、风险

| 风险 | 处置 |
|---|---|
| 顶栏搜索与铃铛仍是占位 | 显式标注（tooltip/禁用态），不做假交互 |
| 无网络时 `checkSession`/同步头像失败 | 一律降级为游客态与「同步失败」内联提示，不阻塞界面 |
| 头像 base64 体积 | set_avatar 强校验 ≤512KB；前端 256px + q0.9 通常在 30–60KB |
| 显示名来源 | 目前只有 username（门户 `getLoginInfo` 不含姓名）；账号菜单未拿到姓名时显示 username，不编造 |
